//! Dev middleware for serving OXC-transformed source files and pre-bundled deps.
//!
//! Provides two endpoints for unbundled dev mode:
//! - `/_rex/dev/{*path}` — reads a source file, OXC-transforms it, and serves as JS
//! - `/_rex/deps/{*path}` — serves pre-bundled dependency ESM files (React, etc.)

use axum::extract::Path;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use rex_build::transform::TransformCache;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::debug;

/// Shared state for dev middleware handlers.
///
/// Cloned into each route handler closure. The inner data is behind `Arc`s
/// so clones are cheap.
#[derive(Clone)]
pub struct DevMiddleware {
    transform_cache: Arc<TransformCache>,
    client_deps: Arc<HashMap<String, String>>,
    project_root: PathBuf,
    /// Route pattern → relative file path, for generating dev-entry hydration modules.
    page_entries: Arc<HashMap<String, String>>,
}

impl DevMiddleware {
    pub fn new(
        transform_cache: Arc<TransformCache>,
        client_deps: HashMap<String, String>,
        project_root: PathBuf,
        page_entries: HashMap<String, String>,
    ) -> Self {
        Self {
            transform_cache,
            client_deps: Arc::new(client_deps),
            project_root,
            page_entries: Arc::new(page_entries),
        }
    }

    /// Build an axum Router with dev middleware routes.
    ///
    /// Generic over `S` so the returned router merges cleanly with any state type.
    /// Handlers capture their state via closures, following the HMR route pattern.
    pub fn into_router<S>(self) -> Router<S>
    where
        S: Clone + Send + Sync + 'static,
    {
        let dev_handler = self.clone();
        let deps_handler = self.clone();
        let entry_handler = self;

        Router::new()
            .route(
                "/_rex/dev/{*path}",
                get(move |Path(path): Path<String>| {
                    let handler = dev_handler.clone();
                    async move { handler.serve_source(&path).await }
                }),
            )
            .route(
                "/_rex/deps/{*path}",
                get(move |Path(name): Path<String>| {
                    let handler = deps_handler.clone();
                    async move { handler.serve_dep(&name) }
                }),
            )
            .route(
                "/_rex/dev-entry/{*path}",
                get(move |Path(path): Path<String>| {
                    let handler = entry_handler.clone();
                    async move { handler.serve_dev_entry(&path) }
                }),
            )
    }

    /// Serve an OXC-transformed source file as JavaScript.
    ///
    /// Maps URL path to filesystem path (relative to project root), reads the
    /// file, transforms via the shared `TransformCache`, and returns JS.
    async fn serve_source(&self, url_path: &str) -> Response {
        let file_path = self.project_root.join(url_path);

        let source = match tokio::fs::read_to_string(&file_path).await {
            Ok(s) => s,
            Err(_) => {
                return (StatusCode::NOT_FOUND, format!("Not found: {url_path}")).into_response();
            }
        };

        match self.transform_cache.transform(&file_path, &source) {
            Ok(js) => {
                debug!(path = url_path, bytes = js.len(), "Served dev source");
                (
                    StatusCode::OK,
                    [
                        (
                            header::CONTENT_TYPE,
                            "application/javascript; charset=utf-8",
                        ),
                        (header::CACHE_CONTROL, "no-cache, no-store, must-revalidate"),
                    ],
                    js,
                )
                    .into_response()
            }
            Err(e) => {
                let msg = format!(
                    "// Transform error: {e:#}\nconsole.error({:?});",
                    e.to_string()
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    [(
                        header::CONTENT_TYPE,
                        "application/javascript; charset=utf-8",
                    )],
                    msg,
                )
                    .into_response()
            }
        }
    }

    /// Serve a pre-bundled dependency ESM file.
    ///
    /// These are immutable (content-addressed by rolldown), so they get a long
    /// `Cache-Control` header.
    fn serve_dep(&self, name: &str) -> Response {
        match self.client_deps.get(name) {
            Some(js) => (
                StatusCode::OK,
                [
                    (
                        header::CONTENT_TYPE,
                        "application/javascript; charset=utf-8",
                    ),
                    (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
                ],
                js.clone(),
            )
                .into_response(),
            None => (StatusCode::NOT_FOUND, format!("Dep not found: {name}")).into_response(),
        }
    }

    /// Serve a generated hydration entry module for a page.
    ///
    /// The entry module imports the page component from `/_rex/dev/`, registers it
    /// on `window.__REX_PAGES`, and hydrates the React tree.
    fn serve_dev_entry(&self, url_path: &str) -> Response {
        // Find the route pattern that matches this file path
        let route_pattern = self
            .page_entries
            .iter()
            .find(|(_, rel_path)| rel_path.as_str() == url_path)
            .map(|(pattern, _)| pattern.as_str());

        let Some(pattern) = route_pattern else {
            return (
                StatusCode::NOT_FOUND,
                format!("No page entry for: {url_path}"),
            )
                .into_response();
        };

        let js = generate_dev_entry(url_path, pattern);
        (
            StatusCode::OK,
            [
                (
                    header::CONTENT_TYPE,
                    "application/javascript; charset=utf-8",
                ),
                (header::CACHE_CONTROL, "no-cache, no-store, must-revalidate"),
            ],
            js,
        )
            .into_response()
    }
}

/// Generate a hydration entry module for dev mode.
///
/// Mirrors the virtual entry in `client_bundle.rs` but imports from `/_rex/dev/` URLs
/// instead of bundled paths, and uses the browser import map for bare specifiers.
fn generate_dev_entry(rel_path: &str, route_pattern: &str) -> String {
    format!(
        r#"import {{ createElement }} from 'react';
import {{ hydrateRoot }} from 'react-dom/client';
import Page from '/_rex/dev/{rel_path}';

window.__REX_PAGES = window.__REX_PAGES || {{}};
window.__REX_PAGES['{route_pattern}'] = {{ default: Page }};

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
