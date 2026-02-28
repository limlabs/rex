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
    CssModified,
}

#[derive(Debug, Clone)]
pub struct FileEvent {
    pub kind: FileEventKind,
    pub path: PathBuf,
}

/// Start watching the project root for file changes. Returns a receiver for file events.
///
/// Watches recursively but skips `node_modules/`, `.rex/`, `.git/`, and `target/`.
/// - `.tsx/.ts/.jsx/.js` files under `pages_dir` → PageModified / PageRemoved
/// - `.css` files anywhere → CssModified
pub fn start_watcher(project_root: &Path, pages_dir: &Path) -> Result<mpsc::Receiver<FileEvent>> {
    let (tx, rx) = mpsc::channel();
    let project_root_owned = project_root.to_path_buf();
    let pages_dir_owned = pages_dir.to_path_buf();
    let watch_dir = project_root_owned.clone();

    let mut debouncer = new_debouncer(
        Duration::from_millis(50),
        move |result: Result<Vec<notify_debouncer_mini::DebouncedEvent>, notify::Error>| {
            match result {
                Ok(events) => {
                    for event in events {
                        let path = event.path;

                        // Skip ignored directories
                        if should_skip(&path, &project_root_owned) {
                            continue;
                        }

                        let kind = match event.kind {
                            DebouncedEventKind::Any => {
                                if is_css_file(&path) {
                                    if path.exists() {
                                        FileEventKind::CssModified
                                    } else {
                                        continue;
                                    }
                                } else if is_page_file(&path) && path.starts_with(&pages_dir_owned)
                                {
                                    if path.exists() {
                                        FileEventKind::PageModified
                                    } else {
                                        FileEventKind::PageRemoved
                                    }
                                } else {
                                    continue;
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
        .watch(&watch_dir, RecursiveMode::Recursive)?;

    debug!(dir = %project_root.display(), "File watcher started");

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

fn is_css_file(path: &Path) -> bool {
    matches!(path.extension().and_then(|e| e.to_str()), Some("css"))
}

fn should_skip(path: &Path, project_root: &Path) -> bool {
    let rel = path.strip_prefix(project_root).unwrap_or(path);
    for component in rel.components() {
        let s = component.as_os_str().to_string_lossy();
        if s == "node_modules" || s == ".rex" || s == ".git" || s == "target" {
            return true;
        }
    }
    false
}
