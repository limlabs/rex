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
    MiddlewareModified,
    McpModified,
    /// A source file outside pages/app dirs was modified (e.g. components/, lib/)
    SourceModified,
}

#[derive(Debug, Clone)]
pub struct FileEvent {
    pub kind: FileEventKind,
    pub path: PathBuf,
}

/// Start watching the project root for file changes. Returns a receiver for file events.
///
/// Watches recursively but skips `node_modules/`, `.rex/`, `.git/`, and `target/`.
/// - `.tsx/.ts/.jsx/.js/.mdx` files under `pages_dir` or `app_dir` → PageModified / PageRemoved
/// - `.tsx/.ts/.jsx/.js/.mdx` files elsewhere in project → SourceModified
/// - `.css` files anywhere → CssModified
pub fn start_watcher(
    project_root: &Path,
    pages_dir: &Path,
    app_dir: &Path,
) -> Result<mpsc::Receiver<FileEvent>> {
    let (tx, rx) = mpsc::channel();
    let project_root_owned = project_root.to_path_buf();
    let pages_dir_owned = pages_dir.to_path_buf();
    let app_dir_owned = app_dir.to_path_buf();
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
                                if is_middleware_file(&path, &project_root_owned) {
                                    if path.exists() {
                                        FileEventKind::MiddlewareModified
                                    } else {
                                        FileEventKind::PageRemoved
                                    }
                                } else if is_mcp_file(&path, &project_root_owned) {
                                    if path.exists() {
                                        FileEventKind::McpModified
                                    } else {
                                        FileEventKind::PageRemoved
                                    }
                                } else if is_css_file(&path) {
                                    if path.exists() {
                                        FileEventKind::CssModified
                                    } else {
                                        continue;
                                    }
                                } else if is_page_file(&path)
                                    && (path.starts_with(&pages_dir_owned)
                                        || path.starts_with(&app_dir_owned))
                                {
                                    if path.exists() {
                                        FileEventKind::PageModified
                                    } else {
                                        FileEventKind::PageRemoved
                                    }
                                } else if is_page_file(&path) && path.exists() {
                                    // Source file outside pages/app (e.g. components/, lib/)
                                    FileEventKind::SourceModified
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
        Some("tsx" | "ts" | "jsx" | "js" | "mdx")
    )
}

/// Check for executable script extensions only (no .mdx).
/// Middleware and MCP tools must be JS/TS modules, not content files.
fn is_script_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("tsx" | "ts" | "jsx" | "js")
    )
}

fn is_middleware_file(path: &Path, project_root: &Path) -> bool {
    if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
        if name == "middleware" && is_script_file(path) {
            // Must be at project root (not nested in pages/ or subdirs)
            if let Some(parent) = path.parent() {
                return parent == project_root;
            }
        }
    }
    false
}

fn is_mcp_file(path: &Path, project_root: &Path) -> bool {
    if is_script_file(path) {
        if let Some(parent) = path.parent() {
            return parent == project_root.join("mcp");
        }
    }
    false
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
