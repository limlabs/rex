use anyhow::Result;
use crossbeam_channel::{bounded, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use tracing::{debug, error, warn};

type WorkItem = Box<dyn FnOnce(&mut crate::SsrIsolate) + Send + 'static>;

/// A pool of V8 isolates, each pinned to its own OS thread.
/// Work is dispatched via channels and results returned via oneshot.
///
/// On drop, senders are closed first (signaling threads to exit),
/// then threads are joined to ensure clean shutdown.
pub struct IsolatePool {
    senders: Vec<Option<Sender<WorkItem>>>,
    threads: Vec<Option<JoinHandle<()>>>,
    next: std::sync::atomic::AtomicUsize,
    size: usize,
}

impl IsolatePool {
    /// Create a new pool with `size` isolates.
    /// Each isolate is initialized with the self-contained server bundle JS.
    pub fn new(size: usize, server_bundle_js: Arc<String>) -> Result<Self> {
        let mut senders = Vec::with_capacity(size);
        let mut threads = Vec::with_capacity(size);

        for i in 0..size {
            let (tx, rx) = bounded::<WorkItem>(64);
            let bundle_js = server_bundle_js.clone();

            let handle = thread::Builder::new()
                .name(format!("rex-v8-isolate-{i}"))
                .spawn(move || {
                    // Initialize V8 on this thread (safe to call multiple times)
                    crate::init_v8();

                    let mut isolate = match crate::SsrIsolate::new(&bundle_js) {
                        Ok(iso) => iso,
                        Err(e) => {
                            error!("Failed to create V8 isolate {i}: {e:#}");
                            return;
                        }
                    };

                    debug!("V8 isolate {i} ready");

                    while let Ok(work) = rx.recv() {
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
            next: std::sync::atomic::AtomicUsize::new(0),
            size,
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
        let idx = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % self.size;

        self.senders[idx]
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("V8 isolate pool is shut down"))?
            .send(work)
            .map_err(|_| anyhow::anyhow!("V8 isolate thread has shut down"))?;

        rx.await
            .map_err(|_| anyhow::anyhow!("V8 isolate dropped the response"))
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
