//! Build cache for live mode.
//!
//! Caches compilation results keyed by project. Uses timestamp-based
//! invalidation by default (zero config) — checks source file mtimes
//! on every request to detect changes.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

/// Cached build result for a project.
#[derive(Debug, Clone)]
pub struct CachedBuild {
    /// The compiled server bundle JS
    pub server_bundle_js: Arc<String>,
    /// Build ID (used for client asset URLs)
    pub build_id: String,
    /// The asset manifest from the build
    pub manifest: rex_core::AssetManifest,
    /// The scan result that produced this build
    pub scan: rex_router::ScanResult,
    /// Timestamp when the most recently modified source file was last changed
    pub source_mtime: SystemTime,
    /// Monotonic build counter
    pub build_number: u64,
}

/// Build cache for a single project.
///
/// Stores the most recent successful build. Invalidation is done by
/// comparing source file mtimes against the cached build's timestamp.
pub struct BuildCache {
    current: std::sync::RwLock<Option<Arc<CachedBuild>>>,
    build_counter: AtomicU64,
}

impl Default for BuildCache {
    fn default() -> Self {
        Self::new()
    }
}

impl BuildCache {
    pub fn new() -> Self {
        Self {
            current: std::sync::RwLock::new(None),
            build_counter: AtomicU64::new(0),
        }
    }

    /// Get the current cached build, if any.
    pub fn get(&self) -> Option<Arc<CachedBuild>> {
        self.current
            .read()
            .expect("BuildCache lock poisoned")
            .clone()
    }

    /// Store a new build result.
    pub fn set(&self, build: CachedBuild) {
        let mut guard = self.current.write().expect("BuildCache lock poisoned");
        *guard = Some(Arc::new(build));
    }

    /// Invalidate the cache (forces rebuild on next request).
    pub fn invalidate(&self) {
        let mut guard = self.current.write().expect("BuildCache lock poisoned");
        *guard = None;
    }

    /// Get the next build number (monotonically increasing).
    pub fn next_build_number(&self) -> u64 {
        self.build_counter.fetch_add(1, Ordering::Relaxed)
    }
}
