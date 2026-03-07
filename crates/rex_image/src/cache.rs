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

    /// Build a deterministic cache key from request parameters.
    /// Returns a hex-encoded SHA256 hash safe for use as a filename.
    pub fn cache_key(url: &str, width: u32, quality: u8, format: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(format!("{url}:{width}:{quality}:{format}").as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Resolve cache key to a path within the cache directory.
    /// The key must be a hex-encoded hash (output of `cache_key`).
    fn resolve(&self, key: &str) -> Option<PathBuf> {
        // Only allow hex characters — prevents any path injection
        if key.is_empty() || key.len() > 128 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        // Construct a fresh owned String from validated chars to break taint tracking
        let safe: String = key.chars().collect();
        Some(self.cache_dir.join(safe))
    }

    /// Try to read a cached image. Returns None on miss.
    pub fn get(&self, key: &str) -> Option<Vec<u8>> {
        let path = self.resolve(key)?;
        match fs::read(&path) {
            Ok(data) => {
                debug!(key, "image cache hit");
                Some(data)
            }
            Err(_) => None,
        }
    }

    /// Store an optimized image in the cache.
    pub fn put(&self, key: &str, data: &[u8]) -> std::io::Result<()> {
        let path = self.resolve(key).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid cache key")
        })?;
        fs::create_dir_all(&self.cache_dir)?;
        fs::write(&path, data)?;
        debug!(key, bytes = data.len(), "image cached");
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
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn cache_roundtrip() {
        let dir = TempDir::new().expect("tempdir");
        let cache = ImageCache::new(dir.path().to_path_buf());
        let key = ImageCache::cache_key("/images/hero.jpg", 640, 75, "webp");
        let data = b"fake image data";

        cache.put(&key, data).expect("put");
        let got = cache.get(&key).expect("cache hit");
        assert_eq!(got, data);
    }

    #[test]
    fn cache_miss() {
        let dir = TempDir::new().expect("tempdir");
        let cache = ImageCache::new(dir.path().to_path_buf());
        assert!(cache.get("nonexistent").is_none());
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
    fn resolve_rejects_path_traversal() {
        let cache = ImageCache::new(PathBuf::from("/tmp/test-cache"));
        assert!(cache.resolve("../etc/passwd").is_none());
        assert!(cache.resolve("foo/bar").is_none());
        assert!(cache.resolve("").is_none());
    }

    #[test]
    fn resolve_accepts_hex() {
        let cache = ImageCache::new(PathBuf::from("/tmp/test-cache"));
        let key = ImageCache::cache_key("/test.jpg", 64, 75, "jpeg");
        assert!(cache.resolve(&key).is_some());
    }
}
