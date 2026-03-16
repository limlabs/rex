use anyhow::Result;
use crossbeam_channel::{bounded, Sender};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::thread::{self, JoinHandle};
use tracing::{debug, error, warn};

type WorkItem = Box<dyn FnOnce(&mut crate::SsrIsolate) + Send + 'static>;

/// Shared state for lazy V8 reload: a new bundle is staged here and each
/// isolate thread picks it up before its next work item.
struct PendingReload {
    generation: AtomicU64,
    bundle: RwLock<Arc<String>>,
}

/// A pool of V8 isolates, each pinned to its own OS thread.
/// Work is dispatched via channels and results returned via oneshot.
///
/// On drop, senders are closed first (signaling threads to exit),
/// then threads are joined to ensure clean shutdown.
pub struct IsolatePool {
    senders: Vec<Option<Sender<WorkItem>>>,
    threads: Vec<Option<JoinHandle<()>>>,
    next: AtomicUsize,
    size: usize,
    pending: Arc<PendingReload>,
}

impl IsolatePool {
    /// Create a new pool with `size` isolates.
    /// Each isolate is initialized with the self-contained server bundle JS.
    /// If `project_root` is provided, fs polyfill callbacks are sandboxed to it.
    pub fn new(
        size: usize,
        server_bundle_js: Arc<String>,
        project_root: Option<Arc<String>>,
    ) -> Result<Self> {
        let mut senders = Vec::with_capacity(size);
        let mut threads = Vec::with_capacity(size);

        let pending = Arc::new(PendingReload {
            generation: AtomicU64::new(0),
            bundle: RwLock::new(server_bundle_js.clone()),
        });

        for i in 0..size {
            let (tx, rx) = bounded::<WorkItem>(64);
            let bundle_js = server_bundle_js.clone();
            let root = project_root.clone();
            let pending = Arc::clone(&pending);

            let handle = thread::Builder::new()
                .name(format!("rex-v8-isolate-{i}"))
                .stack_size(16 * 1024 * 1024) // 16 MB — needed for deeply nested React trees (e.g. large MDX pages)
                .spawn(move || {
                    // Initialize V8 on this thread (safe to call multiple times)
                    crate::init_v8();

                    let mut isolate = match crate::SsrIsolate::new(
                        &bundle_js,
                        root.as_deref().map(|s| s.as_str()),
                    ) {
                        Ok(iso) => iso,
                        Err(e) => {
                            error!("Failed to create V8 isolate {i}: {e:#}");
                            return;
                        }
                    };

                    debug!("V8 isolate {i} ready");

                    let mut local_generation = 0u64;

                    while let Ok(work) = rx.recv() {
                        // Lazy reload: pick up pending bundle before processing work
                        let current_gen = pending.generation.load(Ordering::Acquire);
                        if current_gen > local_generation {
                            let new_bundle = pending
                                .bundle
                                .read()
                                .expect("PendingReload lock poisoned")
                                .clone();
                            if let Err(e) = isolate.reload(&new_bundle) {
                                error!("V8 isolate {i} lazy reload failed: {e:#}");
                                // Bump generation to avoid retry loop — isolate auto-restores last_bundle
                            }
                            local_generation = current_gen;
                        }

                        work(&mut isolate);
                    }

                    debug!("V8 isolate {i} shutting down");
                })?;

            senders.push(Some(tx));
            threads.push(Some(handle));
        }

        debug!(count = size, "V8 isolate pool created");

        Ok(Self {
            senders,
            threads,
            next: AtomicUsize::new(0),
            size,
            pending,
        })
    }

    /// Execute a closure on a V8 isolate and return the result asynchronously.
    pub async fn execute<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&mut crate::SsrIsolate) -> R + Send + 'static,
        R: Send + 'static,
    {
        let (tx, rx) = tokio::sync::oneshot::channel();

        let work: WorkItem = Box::new(move |isolate| {
            let result = f(isolate);
            let _ = tx.send(result);
        });

        // Round-robin dispatch
        let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.size;

        self.senders[idx]
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("V8 isolate pool is shut down"))?
            .send(work)
            .map_err(|_| anyhow::anyhow!("V8 isolate thread has shut down"))?;

        rx.await
            .map_err(|_| anyhow::anyhow!("V8 isolate dropped the response"))
    }

    /// Stage a new bundle for lazy reload. Each isolate thread will pick it up
    /// before processing its next work item — no synchronous reload wait.
    pub fn mark_stale(&self, new_bundle: Arc<String>) {
        *self
            .pending
            .bundle
            .write()
            .expect("PendingReload lock poisoned") = new_bundle;
        self.pending.generation.fetch_add(1, Ordering::Release);
        debug!("V8 isolates marked stale (lazy reload pending)");
    }

    /// Reload all isolates with a new server bundle
    pub async fn reload_all(&self, new_bundle: Arc<String>) -> Result<()> {
        let mut handles = Vec::new();

        for i in 0..self.size {
            let bundle = new_bundle.clone();
            let (tx, rx) = tokio::sync::oneshot::channel();

            let work: WorkItem = Box::new(move |isolate| {
                let result = isolate.reload(&bundle);
                let _ = tx.send(result);
            });

            self.senders[i]
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("V8 isolate pool is shut down"))?
                .send(work)
                .map_err(|_| anyhow::anyhow!("V8 isolate thread has shut down"))?;

            handles.push(rx);
        }

        for handle in handles {
            handle.await??;
        }

        debug!("All V8 isolates reloaded");
        Ok(())
    }

    /// Load RSC bundles into all isolates in the pool.
    pub async fn load_rsc_bundles_all(
        &self,
        flight_bundle: Arc<String>,
        ssr_bundle: Arc<String>,
    ) -> Result<()> {
        let mut handles = Vec::new();

        for i in 0..self.size {
            let flight = flight_bundle.clone();
            let ssr = ssr_bundle.clone();
            let (tx, rx) = tokio::sync::oneshot::channel();

            let work: WorkItem = Box::new(move |isolate| {
                let result = isolate.load_rsc_bundles(&flight, &ssr);
                let _ = tx.send(result);
            });

            self.senders[i]
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("V8 isolate pool is shut down"))?
                .send(work)
                .map_err(|_| anyhow::anyhow!("V8 isolate thread has shut down"))?;

            handles.push(rx);
        }

        for handle in handles {
            handle.await??;
        }

        debug!("RSC bundles loaded into all V8 isolates");
        Ok(())
    }

    /// Load ESM modules into all isolates in the pool.
    pub async fn load_esm_modules_all(
        &self,
        polyfills_js: std::sync::Arc<String>,
        dep_modules: std::sync::Arc<Vec<crate::ssr_isolate_esm::EsmSourceModule>>,
        source_modules: std::sync::Arc<Vec<crate::ssr_isolate_esm::EsmSourceModule>>,
        entry_specifier: std::sync::Arc<String>,
        entry_source: std::sync::Arc<String>,
    ) -> Result<()> {
        let mut handles = Vec::new();

        for i in 0..self.size {
            let polyfills = polyfills_js.clone();
            let deps = dep_modules.clone();
            let sources = source_modules.clone();
            let spec = entry_specifier.clone();
            let src = entry_source.clone();
            let (tx, rx) = tokio::sync::oneshot::channel();

            let work: WorkItem = Box::new(move |isolate| {
                let result = isolate.load_esm_modules(&polyfills, &deps, &sources, &spec, &src);
                let _ = tx.send(result);
            });

            self.senders[i]
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("V8 isolate pool is shut down"))?
                .send(work)
                .map_err(|_| anyhow::anyhow!("V8 isolate thread has shut down"))?;

            handles.push(rx);
        }

        for handle in handles {
            handle.await??;
        }

        debug!("ESM modules loaded into all V8 isolates");
        Ok(())
    }

    /// Invalidate ESM modules in all isolates (for HMR).
    pub async fn invalidate_esm_module_all(
        &self,
        dep_modules: std::sync::Arc<Vec<crate::ssr_isolate_esm::EsmSourceModule>>,
        source_modules: std::sync::Arc<Vec<crate::ssr_isolate_esm::EsmSourceModule>>,
        entry_specifier: std::sync::Arc<String>,
        entry_source: std::sync::Arc<String>,
    ) -> Result<()> {
        let mut handles = Vec::new();

        for i in 0..self.size {
            let deps = dep_modules.clone();
            let sources = source_modules.clone();
            let spec = entry_specifier.clone();
            let src = entry_source.clone();
            let (tx, rx) = tokio::sync::oneshot::channel();

            let work: WorkItem = Box::new(move |isolate| {
                let result = isolate.invalidate_esm_module(&deps, &sources, &spec, &src);
                let _ = tx.send(result);
            });

            self.senders[i]
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("V8 isolate pool is shut down"))?
                .send(work)
                .map_err(|_| anyhow::anyhow!("V8 isolate thread has shut down"))?;

            handles.push(rx);
        }

        for handle in handles {
            handle.await??;
        }

        debug!("ESM module invalidated in all V8 isolates");
        Ok(())
    }
}

impl Drop for IsolatePool {
    fn drop(&mut self) {
        // Drop all senders first — this closes the channels and causes
        // worker threads to exit their recv() loops.
        for sender in &mut self.senders {
            sender.take();
        }

        // Join all threads to wait for in-flight work to complete.
        for (i, handle) in self.threads.iter_mut().enumerate() {
            if let Some(h) = handle.take() {
                if let Err(e) = h.join() {
                    warn!("V8 isolate thread {i} panicked: {e:?}");
                }
            }
        }

        debug!("V8 isolate pool shut down");
    }
}
