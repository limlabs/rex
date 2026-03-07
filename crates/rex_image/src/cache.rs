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
    pub fn cache_key(url: &str, width: u32, quality: u8, format: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(format!("{url}:{width}:{quality}:{format}").as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Validate that a cache key is a safe filename (hex chars only).
    fn validate_key(key: &str) -> bool {
        !key.is_empty() && key.len() <= 128 && key.bytes().all(|b| b.is_ascii_hexdigit())
    }

    /// Try to read a cached image. Returns None on miss.
    pub fn get(&self, key: &str) -> Option<Vec<u8>> {
        if !Self::validate_key(key) {
            return None;
        }
        let path = self.cache_dir.join(key);
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
        if !Self::validate_key(key) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid cache key",
            ));
        }
        fs::create_dir_all(&self.cache_dir)?;
        let path = self.cache_dir.join(key);
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
}
