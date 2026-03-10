//! File watcher for live mode cache invalidation.
//!
//! Watches project source directories and invalidates the build cache
//! when source files change.

use crate::project::LiveProject;
use anyhow::Result;
use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
use std::path::PathBuf;
use std::sync::Weak;
use std::time::Duration;
use tracing::{debug, info};

/// Watches a project directory and invalidates its build cache on changes.
pub struct ProjectWatcher {
    _debouncer: notify_debouncer_mini::Debouncer<notify::RecommendedWatcher>,
}

impl ProjectWatcher {
    /// Start watching a project directory.
    pub fn start(root: PathBuf, project: Weak<LiveProject>) -> Result<Self> {
        let mut debouncer = new_debouncer(
            Duration::from_millis(200),
            move |events: std::result::Result<
                Vec<notify_debouncer_mini::DebouncedEvent>,
                notify::Error,
            >| {
                let Ok(events) = events else { return };

                // Check if any events are for source files
                let has_source_change = events.iter().any(|e| {
                    if e.kind != DebouncedEventKind::Any {
                        return false;
                    }
                    let ext = e.path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    matches!(ext, "tsx" | "ts" | "jsx" | "js" | "css" | "mdx")
                });

                if has_source_change {
                    if let Some(project) = project.upgrade() {
                        debug!("Source file changed, invalidating build cache");
                        project.invalidate();
                    }
                }
            },
        )?;

        // Watch the pages/ and app/ directories
        for dir_name in &["pages", "src/pages", "app", "src/app"] {
            let dir = root.join(dir_name);
            if dir.is_dir() {
                debouncer
                    .watcher()
                    .watch(&dir, notify::RecursiveMode::Recursive)?;
                debug!("Watching {}", dir.display());
            }
        }

        info!("File watcher started for {}", root.display());

        Ok(Self {
            _debouncer: debouncer,
        })
    }
}
