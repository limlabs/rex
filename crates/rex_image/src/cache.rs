use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use tracing::debug;

pub struct ImageCache {
    cache_dir: PathBuf,
}

impl ImageCache {
    pub fn new(cache_dir: PathBuf) -> Self {
        Self { cache_dir }
    }

    /// Compute a cache key from request parameters.
    /// Returns a hex-encoded SHA256 hash (64 chars, `[0-9a-f]` only).
    fn cache_key(url: &str, width: u32, quality: u8, format: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(format!("{url}:{width}:{quality}:{format}").as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Validate the cache key and build the path under `cache_dir`.
    /// Rejects any key containing non-hex characters to prevent path traversal.
    fn validated_cache_path(&self, key: &str) -> Option<PathBuf> {
        if key.is_empty() || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        Some(self.cache_dir.join(key))
    }

    /// Try to read a cached image. Returns None on miss.
    pub fn get(&self, url: &str, width: u32, quality: u8, format: &str) -> Option<Vec<u8>> {
        let key = Self::cache_key(url, width, quality, format);
        let path = self.validated_cache_path(&key)?;
        match fs::read(&path) {
            Ok(data) => {
                debug!(%url, width, quality, format, "image cache hit");
                Some(data)
            }
            Err(_) => None,
        }
    }

    /// Store an optimized image in the cache.
    pub fn put(
        &self,
        url: &str,
        width: u32,
        quality: u8,
        format: &str,
        data: &[u8],
    ) -> std::io::Result<()> {
        let key = Self::cache_key(url, width, quality, format);
        let path = self.validated_cache_path(&key).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid cache key")
        })?;
        fs::create_dir_all(&self.cache_dir)?;
        fs::write(&path, data)?;
        debug!(%url, width, quality, format, bytes = data.len(), "image cached");
        Ok(())
    }

    /// Remove all cached images.
    pub fn clear(&self) -> std::io::Result<()> {
        if self.cache_dir.exists() {
            fs::remove_dir_all(&self.cache_dir)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn cache_roundtrip() {
        let dir = TempDir::new().expect("tempdir");
        let cache = ImageCache::new(dir.path().to_path_buf());
        let data = b"fake image data";

        cache
            .put("/images/hero.jpg", 640, 75, "webp", data)
            .expect("put");
        let got = cache
            .get("/images/hero.jpg", 640, 75, "webp")
            .expect("cache hit");
        assert_eq!(got, data);
    }

    #[test]
    fn cache_miss() {
        let dir = TempDir::new().expect("tempdir");
        let cache = ImageCache::new(dir.path().to_path_buf());
        assert!(cache.get("/nonexistent.jpg", 64, 75, "jpeg").is_none());
    }

    #[test]
    fn cache_key_determinism() {
        let k1 = ImageCache::cache_key("/images/hero.jpg", 640, 75, "webp");
        let k2 = ImageCache::cache_key("/images/hero.jpg", 640, 75, "webp");
        let k3 = ImageCache::cache_key("/images/hero.jpg", 320, 75, "webp");
        assert_eq!(k1, k2);
        assert_ne!(k1, k3);
    }

    #[test]
    fn cache_key_is_hex_only() {
        let key = ImageCache::cache_key("/../../../etc/passwd", 64, 75, "jpeg");
        // SHA256 hex output: only hex chars, no path separators
        assert!(key.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(key.len(), 64); // SHA256 = 32 bytes = 64 hex chars
    }

    #[test]
    fn validated_cache_path_rejects_non_hex() {
        let cache = ImageCache::new(PathBuf::from("/tmp/test-cache"));
        assert!(cache.validated_cache_path("").is_none());
        assert!(cache.validated_cache_path("../etc/passwd").is_none());
        assert!(cache.validated_cache_path("foo/bar").is_none());
        assert!(cache.validated_cache_path("abcdef0123456789").is_some());
    }
}
