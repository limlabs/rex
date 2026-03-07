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

    /// Build a deterministic cache path from request parameters.
    /// The filename is a hex-encoded SHA256 hash — no user input reaches the path.
    fn cache_path(&self, url: &str, width: u32, quality: u8, format: &str) -> PathBuf {
        let mut hasher = Sha256::new();
        hasher.update(format!("{url}:{width}:{quality}:{format}").as_bytes());
        let hex_hash = hex::encode(hasher.finalize());
        self.cache_dir.join(hex_hash)
    }

    /// Try to read a cached image. Returns None on miss.
    pub fn get(&self, url: &str, width: u32, quality: u8, format: &str) -> Option<Vec<u8>> {
        let path = self.cache_path(url, width, quality, format);
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
        let path = self.cache_path(url, width, quality, format);
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
        let cache = ImageCache::new(PathBuf::from("/tmp/test-cache"));
        let p1 = cache.cache_path("/images/hero.jpg", 640, 75, "webp");
        let p2 = cache.cache_path("/images/hero.jpg", 640, 75, "webp");
        let p3 = cache.cache_path("/images/hero.jpg", 320, 75, "webp");
        assert_eq!(p1, p2);
        assert_ne!(p1, p3);
    }

    #[test]
    fn cache_path_is_hex_only() {
        let cache = ImageCache::new(PathBuf::from("/tmp/test-cache"));
        let path = cache.cache_path("/../../../etc/passwd", 64, 75, "jpeg");
        let filename = path.file_name().unwrap().to_str().unwrap();
        // SHA256 hex output: only hex chars, no path separators
        assert!(filename.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(filename.len(), 64); // SHA256 = 32 bytes = 64 hex chars
    }
}
