//! Per-mount-point project state for live mode.
//!
//! Each mounted project has its own source, build cache, V8 pool,
//! route trie, and file watcher. Projects are fully isolated.

use crate::cache::{BuildCache, CachedBuild};
use crate::source::{LocalSource, SourceProvider};
use crate::watcher::ProjectWatcher;
use anyhow::{Context, Result};
use rex_router::RouteTrie;
use rex_v8::IsolatePool;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use tracing::{debug, info};

/// Wrapper making `IsolatePool` `Sync`.
///
/// `IsolatePool` is `Send` but `!Sync` only because `JoinHandle` is `!Sync`.
/// The pool's public API (`execute`, `reload_all`) is fully thread-safe — all
/// work is dispatched through crossbeam channels. `JoinHandle` is only accessed
/// in `Drop`, which takes `&mut self`.
pub(crate) struct SyncPool(IsolatePool);

// SAFETY: See doc comment above — IsolatePool is thread-safe in practice.
#[allow(unsafe_code)]
unsafe impl Sync for SyncPool {}

impl Deref for SyncPool {
    type Target = IsolatePool;
    fn deref(&self) -> &IsolatePool {
        &self.0
    }
}

/// Configuration for a live project mount.
pub struct LiveProjectConfig {
    /// URL path prefix (e.g., "/dashboard")
    pub prefix: String,
    /// Source directory path
    pub source_path: PathBuf,
    /// Number of V8 worker threads
    pub workers: usize,
}

/// A single mounted project in live mode.
pub struct LiveProject {
    /// URL path prefix (e.g., "/dashboard")
    pub prefix: String,
    /// Source provider
    source: Arc<LocalSource>,
    /// Build cache (stores the most recent successful compilation)
    cache: BuildCache,
    /// V8 isolate pool (lazily initialized on first request)
    pool: RwLock<Option<Arc<SyncPool>>>,
    /// Route trie (updated after each build)
    route_trie: RwLock<Option<RouteTrie>>,
    /// API route trie
    api_route_trie: RwLock<Option<RouteTrie>>,
    /// Number of V8 workers
    workers: usize,
    /// Whether a compilation is in progress (prevents concurrent compiles)
    compiling: AtomicBool,
    /// File watcher for cache invalidation
    _watcher: Option<ProjectWatcher>,
    /// Current manifest JSON (pre-serialized for handler use)
    manifest_json: RwLock<Option<String>>,
}

impl LiveProject {
    /// Create a new live project.
    pub fn new(config: LiveProjectConfig) -> Result<Arc<Self>> {
        let source = Arc::new(
            LocalSource::new(config.source_path.clone()).with_context(|| {
                format!("Failed to open source: {}", config.source_path.display())
            })?,
        );

        let project = Arc::new(Self {
            prefix: config.prefix,
            source: source.clone(),
            cache: BuildCache::new(),
            pool: RwLock::new(None),
            route_trie: RwLock::new(None),
            api_route_trie: RwLock::new(None),
            workers: config.workers,
            compiling: AtomicBool::new(false),
            _watcher: None,
            manifest_json: RwLock::new(None),
        });

        // Start file watcher for cache invalidation
        let watcher = ProjectWatcher::start(source.root().to_path_buf(), Arc::downgrade(&project))?;

        // We just created the Arc so there are no other refs.
        let project = {
            let mut inner = Arc::try_unwrap(project)
                .ok()
                .expect("just created, no other refs");
            inner._watcher = Some(watcher);
            Arc::new(inner)
        };

        Ok(project)
    }

    /// Get or compile the project, returning the cached build.
    pub async fn ensure_built(&self) -> Result<Arc<CachedBuild>> {
        // Fast path: check cache
        if let Some(cached) = self.cache.get() {
            // Timestamp-based check: are any source files newer?
            let latest = crate::compiler::latest_source_mtime_pub(self.source.root())?;
            if latest <= cached.source_mtime {
                return Ok(cached);
            }
            info!(prefix = %self.prefix, "Source files changed, recompiling");
        }

        // Prevent concurrent compilations
        if self
            .compiling
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            // Another task is compiling — wait for it by polling the cache
            debug!(prefix = %self.prefix, "Waiting for in-progress compilation");
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                if let Some(cached) = self.cache.get() {
                    return Ok(cached);
                }
                if !self.compiling.load(Ordering::SeqCst) {
                    // Compilation finished but cache is empty — it failed
                    anyhow::bail!("Compilation failed for {}", self.prefix);
                }
            }
        }

        let result =
            crate::compiler::compile_project(self.source.as_ref(), &self.cache, self.workers).await;

        self.compiling.store(false, Ordering::SeqCst);

        let cached = result?;

        // Update V8 pool and route tries
        self.update_from_build(&cached).await?;

        Ok(cached)
    }

    /// Update the V8 pool and route tries from a new build.
    async fn update_from_build(&self, build: &CachedBuild) -> Result<()> {
        let bundle_js = build.server_bundle_js.clone();
        let project_root = Arc::new(self.source.root().to_string_lossy().to_string());

        // Initialize or reload V8 pool
        {
            let existing_pool = self.pool.read().expect("pool lock poisoned").clone();

            if let Some(pool) = existing_pool {
                // Reload existing isolates (guard already dropped)
                pool.reload_all(bundle_js.clone()).await?;
                debug!(prefix = %self.prefix, "V8 isolates reloaded");
            } else {
                // Create new pool
                rex_v8::init_v8();
                let pool = IsolatePool::new(self.workers, bundle_js, Some(project_root))?;
                let mut pool_guard = self.pool.write().expect("pool lock poisoned");
                *pool_guard = Some(Arc::new(SyncPool(pool)));
                debug!(prefix = %self.prefix, workers = self.workers, "V8 isolate pool created");
            }
        }

        // Update route tries
        {
            let route_trie = RouteTrie::from_routes(&build.scan.routes);
            let api_route_trie = RouteTrie::from_routes(&build.scan.api_routes);
            let mut rt = self.route_trie.write().expect("route_trie lock poisoned");
            *rt = Some(route_trie);
            let mut at = self
                .api_route_trie
                .write()
                .expect("api_route_trie lock poisoned");
            *at = Some(api_route_trie);
        }

        // Update manifest JSON
        {
            let json = rex_server::state::HotState::compute_manifest_json(
                &build.build_id,
                &build.manifest,
            );
            let mut mj = self
                .manifest_json
                .write()
                .expect("manifest_json lock poisoned");
            *mj = Some(json);
        }

        Ok(())
    }

    /// Get the V8 isolate pool. Returns None if not yet initialized.
    pub(crate) fn pool(&self) -> Option<Arc<SyncPool>> {
        self.pool.read().expect("pool lock poisoned").clone()
    }

    /// Get the route trie. Returns None if not yet built.
    pub fn route_trie(&self) -> Option<RouteTrie> {
        self.route_trie
            .read()
            .expect("route_trie lock poisoned")
            .clone()
    }

    /// Get the API route trie. Returns None if not yet built.
    pub fn api_route_trie(&self) -> Option<RouteTrie> {
        self.api_route_trie
            .read()
            .expect("api_route_trie lock poisoned")
            .clone()
    }

    /// Get the pre-serialized manifest JSON.
    pub fn manifest_json(&self) -> Option<String> {
        self.manifest_json
            .read()
            .expect("manifest_json lock poisoned")
            .clone()
    }

    /// Invalidate the build cache (called by file watcher on source changes).
    pub fn invalidate(&self) {
        self.cache.invalidate();
        info!(prefix = %self.prefix, "Build cache invalidated");
    }

    /// Get the source root path.
    pub fn source_root(&self) -> &std::path::Path {
        self.source.root()
    }

    /// Get the static output directory (.rex/build/client).
    pub fn static_dir(&self) -> PathBuf {
        self.source.root().join(".rex/build/client")
    }
}
