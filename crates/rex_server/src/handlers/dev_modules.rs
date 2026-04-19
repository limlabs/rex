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

    let classes = extract_css_classes(&source);

    let stem = css_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("module")
        .replace(".module", "");

    let js = generate_css_proxy_js(&classes, &stem);

    (
        StatusCode::OK,
        [("content-type", "application/javascript; charset=utf-8")],
        js,
    )
        .into_response()
}

/// Handler for `/_rex/entry/{*pattern}` — serves virtual page entry modules.
///
/// Generates an ESM module that imports the page component from `/_rex/src/`
/// and sets up hydration (same logic as rolldown virtual entries, but importing
/// from dev-mode URLs instead of filesystem paths).
pub async fn entry_handler(
    State(state): State<Arc<AppState>>,
    Path(pattern): Path<String>,
) -> impl IntoResponse {
    if !state.is_dev {
        return (StatusCode::NOT_FOUND, "Not available in production").into_response();
    }

    let hot = crate::state::snapshot(&state);

    // Special case: _app entry
    if pattern == "_app" {
        let app_path = match hot.route_paths.get("/_app") {
            Some(p) => p,
            None => {
                return (StatusCode::NOT_FOUND, "No _app found").into_response();
            }
        };
        let rel_path = app_path
            .strip_prefix(&state.project_root)
            .unwrap_or(app_path)
            .to_string_lossy()
            .replace('\\', "/");
        let js = generate_app_entry_js(&rel_path);
        return (
            StatusCode::OK,
            [
                ("content-type", "application/javascript; charset=utf-8"),
                ("cache-control", "no-store"),
            ],
            js,
        )
            .into_response();
    }

    // Normalise: the pattern comes from the URL path after /_rex/entry/
    // e.g. "/"  or  "/about"  or  "/blog/:slug"
    let route_pattern = if pattern.starts_with('/') {
        pattern.clone()
    } else {
        format!("/{pattern}")
    };

    // Look up the source file path from the route_paths map
    let abs_path = match hot.route_paths.get(&route_pattern) {
        Some(p) => p.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                format!("No route for pattern: {route_pattern}"),
            )
                .into_response();
        }
    };

    let rel_path = abs_path
        .strip_prefix(&state.project_root)
        .unwrap_or(&abs_path)
        .to_string_lossy()
        .replace('\\', "/");

    let src_url = format!("/_rex/src/{rel_path}");
    let js = generate_page_entry_js(&route_pattern, &src_url);

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

/// Generate the page entry JS module for a given route.
pub(crate) fn generate_page_entry_js(route_pattern: &str, src_url: &str) -> String {
    format!(
        r#"import {{ createElement }} from 'react';
import {{ hydrateRoot }} from 'react-dom/client';
import Page from '{src_url}';

window.__REX_PAGES = window.__REX_PAGES || {{}};
window.__REX_PAGES['{route_pattern}'] = {{ default: Page }};

// Expose render function for client-side navigation (used by router.js)
if (!window.__REX_RENDER__) {{
  window.__REX_RENDER__ = function(Component, props) {{
    var element;
    if (window.__REX_APP__) {{
      element = createElement(window.__REX_APP__, {{ Component: Component, pageProps: props }});
    }} else {{
      element = createElement(Component, props);
    }}
    if (window.__REX_ROOT__) {{
      window.__REX_ROOT__.render(element);
    }}
  }};
}}

if (!window.__REX_NAVIGATING__) {{
  var dataEl = document.getElementById('__REX_DATA__');
  var pageProps = dataEl ? JSON.parse(dataEl.textContent) : {{}};
  var container = document.getElementById('__rex');
  if (container) {{
    var element;
    if (window.__REX_APP__) {{
      element = createElement(window.__REX_APP__, {{ Component: Page, pageProps: pageProps }});
    }} else {{
      element = createElement(Page, pageProps);
    }}
    window.__REX_ROOT__ = hydrateRoot(container, element);
  }}
}}
"#
    )
}

/// Generate the _app entry JS module.
pub(crate) fn generate_app_entry_js(rel_path: &str) -> String {
    format!("import App from '/_rex/src/{rel_path}';\nwindow.__REX_APP__ = App;\n")
}

/// Extract CSS class names from a CSS source string.
pub(crate) fn extract_css_classes(source: &str) -> Vec<String> {
    let mut classes = Vec::new();
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
    classes
}

/// Generate a CSS module JS proxy from class names and a stem.
pub(crate) fn generate_css_proxy_js(classes: &[String], stem: &str) -> String {
    let mut js = String::from("export default {\n");
    for class in classes {
        let camel = to_camel_case(class);
        js.push_str(&format!("  \"{camel}\": \"{stem}_{class}\",\n"));
    }
    js.push_str("};\n");
    js
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    #[test]
    fn test_to_camel_case() {
        assert_eq!(to_camel_case("foo-bar"), "fooBar");
        assert_eq!(to_camel_case("already"), "already");
        assert_eq!(to_camel_case("a-b-c"), "aBC");
    }

    #[test]
    fn test_browser_transform_cache_basic() {
        let cache = BrowserTransformCache::new();
        let now = SystemTime::now();
        let path = "/project/src/index.tsx";

        // Initially empty
        assert!(cache.get(path, now).is_none());

        // Insert and retrieve
        cache.insert(path.to_string(), now, "const x = 1;".to_string());
        let hit = cache.get(path, now);
        assert_eq!(hit.unwrap(), "const x = 1;");

        // Different mtime → cache miss
        let later = now + Duration::from_secs(1);
        assert!(
            cache.get(path, later).is_none(),
            "Changed mtime should miss the cache"
        );

        // Invalidate explicitly
        cache.insert(path.to_string(), later, "const x = 2;".to_string());
        assert!(cache.get(path, later).is_some());
        cache.invalidate(path);
        assert!(
            cache.get(path, later).is_none(),
            "Invalidated entry should not be returned"
        );
    }

    #[test]
    fn test_generate_page_entry_js() {
        let js = generate_page_entry_js("/about", "/_rex/src/pages/about.tsx");
        assert!(js.contains("hydrateRoot"));
        assert!(js.contains("__REX_PAGES"));
        assert!(js.contains("__REX_RENDER__"));
        assert!(js.contains("/_rex/src/pages/about.tsx"));
        assert!(js.contains("'/about'"));
    }

    #[test]
    fn test_generate_app_entry_js() {
        let js = generate_app_entry_js("pages/_app.tsx");
        assert!(js.contains("/_rex/src/pages/_app.tsx"));
        assert!(js.contains("__REX_APP__"));
    }

    #[test]
    fn test_extract_css_classes() {
        let css = ".foo { color: red; }\n.bar-baz { margin: 0; }\n.a_b { padding: 1px; }\np { font-size: 14px; }";
        let classes = extract_css_classes(css);
        assert_eq!(classes, vec!["foo", "bar-baz", "a_b"]);
    }

    #[test]
    fn test_extract_css_classes_empty() {
        let css = "body { margin: 0; }";
        let classes = extract_css_classes(css);
        assert!(classes.is_empty());
    }

    #[test]
    fn test_generate_css_proxy_js() {
        let classes = vec!["foo".to_string(), "bar-baz".to_string()];
        let js = generate_css_proxy_js(&classes, "styles");
        assert!(js.contains("\"foo\": \"styles_foo\""));
        assert!(js.contains("\"barBaz\": \"styles_bar-baz\""));
        assert!(js.starts_with("export default {"));
    }
}
