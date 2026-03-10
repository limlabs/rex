//! Source provider abstraction for live mode.
//!
//! MVP supports local filesystem only. S3 provider will be added later.

use anyhow::Result;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Metadata about a source file, used for cache invalidation.
#[derive(Debug, Clone)]
pub struct FileMeta {
    pub modified: SystemTime,
    pub size: u64,
}

/// A source of project files (local filesystem, S3, etc.)
pub trait SourceProvider: Send + Sync {
    /// Get the root path for this source (used as rolldown cwd)
    fn root(&self) -> &Path;

    /// Check if a directory exists in the source
    fn dir_exists(&self, path: &str) -> bool;

    /// Get metadata for a file (mtime, size). Returns None if file doesn't exist.
    fn file_meta(&self, path: &str) -> Result<Option<FileMeta>>;
}

/// Local filesystem source provider.
pub struct LocalSource {
    root: PathBuf,
}

impl LocalSource {
    pub fn new(root: PathBuf) -> Result<Self> {
        let root = root.canonicalize()?;
        Ok(Self { root })
    }
}

impl SourceProvider for LocalSource {
    fn root(&self) -> &Path {
        &self.root
    }

    fn dir_exists(&self, path: &str) -> bool {
        self.root.join(path).is_dir()
    }

    fn file_meta(&self, path: &str) -> Result<Option<FileMeta>> {
        let full_path = self.root.join(path);
        match std::fs::metadata(&full_path) {
            Ok(meta) => Ok(Some(FileMeta {
                modified: meta.modified()?,
                size: meta.len(),
            })),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}
