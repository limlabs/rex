use anyhow::Result;
use notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;
use tracing::debug;

#[derive(Debug, Clone)]
pub enum FileEventKind {
    PageModified,
    PageRemoved,
}

#[derive(Debug, Clone)]
pub struct FileEvent {
    pub kind: FileEventKind,
    pub path: PathBuf,
}

/// Start watching the pages directory. Returns a receiver for file events.
pub fn start_watcher(pages_dir: &Path) -> Result<mpsc::Receiver<FileEvent>> {
    let (tx, rx) = mpsc::channel();
    let pages_dir_owned = pages_dir.to_path_buf();

    let mut debouncer = new_debouncer(
        Duration::from_millis(200),
        move |result: Result<Vec<notify_debouncer_mini::DebouncedEvent>, notify::Error>| {
            match result {
                Ok(events) => {
                    for event in events {
                        let path = event.path;

                        // Only care about page files
                        if !is_page_file(&path) {
                            continue;
                        }

                        let kind = match event.kind {
                            DebouncedEventKind::Any => {
                                if path.exists() {
                                    // Could be new or modified
                                    FileEventKind::PageModified
                                } else {
                                    FileEventKind::PageRemoved
                                }
                            }
                            _ => continue,
                        };

                        debug!(path = %path.display(), ?kind, "File event");
                        let _ = tx.send(FileEvent { kind, path });
                    }
                }
                Err(e) => {
                    tracing::error!("Watcher error: {e}");
                }
            }
        },
    )?;

    debouncer
        .watcher()
        .watch(&pages_dir_owned, RecursiveMode::Recursive)?;

    debug!(dir = %pages_dir.display(), "File watcher started");

    // Keep the debouncer alive by leaking it (it runs in its own thread)
    std::mem::forget(debouncer);

    Ok(rx)
}

fn is_page_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("tsx" | "ts" | "jsx" | "js")
    )
}
