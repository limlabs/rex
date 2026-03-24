//! Dev-only handler for serving OXC-transformed source files to the browser.
//!
//! Serves individual ESM modules via `/_rex/src/{path}` with:
//! - OXC TS/TSX → JS transformation
//! - Import rewriting: relative paths → `/_rex/src/...` URLs
//! - DCE: strips `getServerSideProps` / `getStaticProps`
//! - LRU cache keyed by (path, mtime) to avoid re-transforming on every request

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use super::AppState;

/// In-memory LRU transform cache.
/// Keyed by canonical file path, invalidated by mtime change.
pub struct BrowserTransformCache {
    entries: Mutex<HashMap<String, CacheEntry>>,
}

struct CacheEntry {
    mtime: SystemTime,
    js: String,
}

impl Default for BrowserTransformCache {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserTransformCache {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Invalidate a specific path (called on HMR file change).
    pub fn invalidate(&self, path: &str) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.remove(path);
        }
    }

    fn get(&self, path: &str, mtime: SystemTime) -> Option<String> {
        let entries = self.entries.lock().ok()?;
        let entry = entries.get(path)?;
        if entry.mtime == mtime {
            Some(entry.js.clone())
        } else {
            None
        }
    }

    fn insert(&self, path: String, mtime: SystemTime, js: String) {
        if let Ok(mut entries) = self.entries.lock() {
            // Simple size cap — evict everything if cache gets too large
            if entries.len() > 1000 {
                entries.clear();
            }
            entries.insert(path, CacheEntry { mtime, js });
        }
    }
}

/// Handler for `/_rex/src/{*path}` — serves OXC-transformed source files.
pub async fn src_handler(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
) -> impl IntoResponse {
    // Only available in dev mode
    if !state.is_dev {
        return (StatusCode::NOT_FOUND, "Not available in production").into_response();
    }

    // Resolve the path — it's relative to project root
    let file_path = state.project_root.join(&path);

    // Security: reject path traversal
    let canonical = match file_path.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (StatusCode::NOT_FOUND, format!("File not found: {path}")).into_response()
        }
    };
    let canonical_root = state
        .project_root
        .canonicalize()
        .unwrap_or_else(|_| state.project_root.clone());
    if !canonical.starts_with(&canonical_root) {
        return (StatusCode::FORBIDDEN, "Path traversal not allowed").into_response();
    }

    // Handle special file types
    let ext = canonical.extension().and_then(|e| e.to_str()).unwrap_or("");

    match ext {
        // CSS imports → empty module (CSS loaded via <style> tags)
        "css" if !path.contains(".module.") => {
            return (
                StatusCode::OK,
                [("content-type", "application/javascript; charset=utf-8")],
                "export default {};",
            )
                .into_response();
        }
        // CSS modules → return JS proxy (class name map)
        "css" if path.contains(".module.") => {
            return serve_css_module_proxy(&canonical).into_response();
        }
        // Image/asset imports → export URL
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "ico" | "svg" | "avif" => {
            let asset_url = format!(
                "/_rex/static/{}",
                canonical.file_name().unwrap_or_default().to_string_lossy()
            );
            return (
                StatusCode::OK,
                [("content-type", "application/javascript; charset=utf-8")],
                format!("export default \"{asset_url}\";"),
            )
                .into_response();
        }
        _ => {}
    }

    // Check transform cache
    let mtime = std::fs::metadata(&canonical)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);

    let cache = state
        .browser_transform_cache
        .get_or_init(BrowserTransformCache::new);
    let cache_key = canonical.to_string_lossy().to_string();

    if let Some(cached) = cache.get(&cache_key, mtime) {
        return (
            StatusCode::OK,
            [
                ("content-type", "application/javascript; charset=utf-8"),
                ("cache-control", "no-store"),
            ],
            cached,
        )
            .into_response();
    }

    // Read and transform the source file
    let source = match std::fs::read_to_string(&canonical) {
        Ok(s) => s,
        Err(e) => {
            return (StatusCode::NOT_FOUND, format!("Cannot read {path}: {e}")).into_response()
        }
    };

    // Build dep specifiers set for import rewriting
    let known_deps = rex_build::esm_transform::dep_specifiers(false);

    match rex_build::esm_transform::transform_for_browser(
        &source,
        &canonical,
        &canonical_root,
        &known_deps,
    ) {
        Ok(js) => {
            cache.insert(cache_key, mtime, js.clone());
            (
                StatusCode::OK,
                [
                    ("content-type", "application/javascript; charset=utf-8"),
                    ("cache-control", "no-store"),
                ],
                js,
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Transform error for {path}: {e}"),
        )
            .into_response(),
    }
}

/// Generate a JS proxy module for a CSS module file.
/// Returns `export default { className: "scoped_className", ... }`.
fn serve_css_module_proxy(css_path: &std::path::Path) -> impl IntoResponse {
    let source = match std::fs::read_to_string(css_path) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::NOT_FOUND,
                format!("Cannot read CSS module: {e}"),
            )
                .into_response()
        }
    };

    // Simple CSS module class extraction — find .className patterns
    let mut classes: Vec<String> = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix('.') {
            if let Some(name) = rest.split([' ', '{', ',', ':', '>']).next() {
                let name = name.trim();
                if !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
                {
                    classes.push(name.to_string());
                }
            }
        }
    }

    let stem = css_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("module")
        .replace(".module", "");

    // Generate a simple proxy — in the full implementation, this would use
    // the same scoping logic as css_modules.rs
    let mut js = String::from("export default {\n");
    for class in &classes {
        let camel = to_camel_case(class);
        js.push_str(&format!("  \"{camel}\": \"{stem}_{class}\",\n"));
    }
    js.push_str("};\n");

    (
        StatusCode::OK,
        [("content-type", "application/javascript; charset=utf-8")],
        js,
    )
        .into_response()
}

/// Convert kebab-case to camelCase for CSS module class names.
fn to_camel_case(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = false;
    for ch in s.chars() {
        if ch == '-' {
            capitalize_next = true;
        } else if capitalize_next {
            result.extend(ch.to_uppercase());
            capitalize_next = false;
        } else {
            result.push(ch);
        }
    }
    result
}
