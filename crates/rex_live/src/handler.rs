//! Axum handler for live mode requests.
//!
//! Routes requests to the correct project based on URL prefix,
//! ensures the project is compiled, then delegates to the standard
//! rex_server page handler infrastructure.

use crate::server::LiveServer;
use axum::body::Body;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, error, info};

use rex_core::{DataStrategy, ServerSidePropsContext};
use rex_server::document::{assemble_document, DocumentParams};

/// Main live mode request handler.
///
/// 1. Match request path to a project by longest prefix
/// 2. Ensure project is compiled (lazy build on first request)
/// 3. Route match within the project
/// 4. SSR render via V8
/// 5. Return HTML response
pub async fn live_handler(
    State(server): State<Arc<LiveServer>>,
    request: axum::extract::Request,
) -> Response {
    live_handler_impl(server, request).await
}

#[allow(clippy::too_many_lines)]
async fn live_handler_impl(server: Arc<LiveServer>, request: axum::extract::Request) -> Response {
    let uri = request.uri().clone();
    let headers = request.headers().clone();
    let path = uri.path();

    // Find the project for this request
    let (project, remaining_path) = match server.match_project(path) {
        Some(result) => result,
        None => {
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"error":"no project mounted at this path"}"#))
                .expect("response build");
        }
    };

    info!(
        prefix = %project.prefix,
        path = remaining_path,
        "Live request"
    );

    // Ensure the project is built
    let build = match project.ensure_built().await {
        Ok(build) => build,
        Err(e) => {
            error!(prefix = %project.prefix, "Compilation failed: {e:#}");
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("content-type", "text/html; charset=utf-8")
                .body(Body::from(format!(
                    "<html><body><h1>Compilation Error</h1><pre style=\"padding:20px;color:#e63946;font-family:monospace\">{}</pre></body></html>",
                    html_escape(&format!("{e:#}"))
                )))
                .expect("response build");
        }
    };

    // Route matching
    let route_trie = match project.route_trie() {
        Some(trie) => trie,
        None => {
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from("Route trie not initialized"))
                .expect("response build");
        }
    };

    let route_match = match route_trie.match_path(remaining_path) {
        Some(m) => m,
        None => {
            debug!(path = remaining_path, "No route matched in project");
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header("content-type", "text/html; charset=utf-8")
                .body(Body::from("404 - Page Not Found"))
                .expect("response build");
        }
    };

    let route_key = route_match.route.module_name();
    let params = route_match.params.clone();

    // Get the pool
    let pool = match project.pool() {
        Some(pool) => pool,
        None => {
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from("V8 pool not initialized"))
                .expect("response build");
        }
    };

    // Determine data strategy
    let strategy = build
        .manifest
        .pages
        .get(&route_match.route.pattern)
        .map(|p| &p.data_strategy)
        .cloned()
        .unwrap_or_default();

    // Fetch data props
    let route_key_clone = route_key.clone();
    let gssp_result = match strategy {
        DataStrategy::None => Ok(Ok(r#"{"props":{}}"#.to_string())),
        DataStrategy::GetStaticProps => {
            let ctx_json = serde_json::json!({ "params": params }).to_string();
            pool.execute(move |iso| iso.get_static_props(&route_key_clone, &ctx_json))
                .await
        }
        DataStrategy::GetServerSideProps => {
            let query: HashMap<String, String> = uri
                .query()
                .map(|q| {
                    q.split('&')
                        .filter_map(|pair| {
                            let mut parts = pair.splitn(2, '=');
                            Some((
                                parts.next()?.to_string(),
                                parts.next().unwrap_or("").to_string(),
                            ))
                        })
                        .collect()
                })
                .unwrap_or_default();

            let header_map: HashMap<String, String> = headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                .collect();

            let context = ServerSidePropsContext {
                params,
                query,
                resolved_url: remaining_path.to_string(),
                headers: header_map,
                cookies: HashMap::new(),
            };
            let context_json = serde_json::to_string(&context).expect("JSON serialization");

            pool.execute(move |iso| iso.get_server_side_props(&route_key_clone, &context_json))
                .await
        }
    };

    let props_json = match gssp_result {
        Ok(Ok(json)) => json,
        Ok(Err(e)) => {
            error!("GSSP error: {e}");
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("content-type", "text/html; charset=utf-8")
                .body(Body::from(format!(
                    "<html><body><h1>Server Error</h1><pre>{}</pre></body></html>",
                    html_escape(&e.to_string())
                )))
                .expect("response build");
        }
        Err(e) => {
            error!("V8 pool error: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Parse props, check for redirect/notFound
    let parsed = serde_json::from_str::<serde_json::Value>(&props_json)
        .unwrap_or_else(|_| serde_json::json!({"props": {}}));

    if let Some(redirect) = parsed.get("redirect") {
        let destination = redirect
            .get("destination")
            .and_then(|d| d.as_str())
            .unwrap_or("/");
        let permanent = redirect
            .get("permanent")
            .and_then(|p| p.as_bool())
            .unwrap_or(false);
        let status = if permanent { 301 } else { 307 };
        return Response::builder()
            .status(status)
            .header("Location", destination)
            .body(Body::empty())
            .expect("response build");
    }

    if parsed
        .get("notFound")
        .and_then(|n| n.as_bool())
        .unwrap_or(false)
    {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("404 - Not Found"))
            .expect("response build");
    }

    let render_props = match parsed.get("props") {
        Some(props) => serde_json::to_string(props).expect("JSON serialization"),
        None => "{}".to_string(),
    };

    // Look up client assets
    let page_assets = build.manifest.pages.get(&route_match.route.pattern);
    let client_scripts: Vec<String> = page_assets
        .map(|assets| vec![assets.js.clone()])
        .unwrap_or_default();

    let mut css_files = build.manifest.global_css.clone();
    if let Some(assets) = page_assets {
        css_files.extend(assets.css.iter().cloned());
    }

    let manifest_json = project.manifest_json().unwrap_or_default();

    // SSR render
    let route_key_clone = route_key.clone();
    let render_props_clone = render_props.clone();
    let ssr_result = pool
        .execute(move |iso| iso.render_page(&route_key_clone, &render_props_clone))
        .await;

    let (body_html, head_html) = match ssr_result {
        Ok(Ok(r)) => (r.body, r.head),
        Ok(Err(e)) => {
            error!("SSR render error: {e}");
            let msg = html_escape(&e.to_string());
            (
                format!("<pre style=\"padding:20px;color:#e63946;font-family:monospace\">SSR Error: {msg}</pre>"),
                String::new(),
            )
        }
        Err(e) => {
            error!("V8 pool error: {e}");
            ("<h1>Internal Server Error</h1>".to_string(), String::new())
        }
    };

    // Assemble the HTML document
    // Use the prefix-aware static path for client assets
    let _prefix = if project.prefix == "/" {
        ""
    } else {
        &project.prefix
    };

    let html = assemble_document(&DocumentParams {
        ssr_html: &body_html,
        head_html: &head_html,
        props_json: &render_props,
        client_scripts: &client_scripts,
        css_files: &css_files,
        css_contents: &build.manifest.css_contents,
        app_script: build.manifest.app_script.as_deref(),
        is_dev: false,
        doc_descriptor: None,
        manifest_json: Some(&manifest_json),
        font_preloads: &build.manifest.font_preloads,
        import_map_json: None,
    });

    Response::builder()
        .header("content-type", "text/html; charset=utf-8")
        .body(Body::from(html))
        .expect("response build")
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_escape_ampersand() {
        assert_eq!(html_escape("a & b"), "a &amp; b");
    }

    #[test]
    fn html_escape_tags() {
        assert_eq!(html_escape("<script>"), "&lt;script&gt;");
    }

    #[test]
    fn html_escape_quotes() {
        assert_eq!(html_escape(r#"he said "hi""#), "he said &quot;hi&quot;");
    }

    #[test]
    fn html_escape_combined() {
        assert_eq!(
            html_escape(r#"<a href="x&y">"#),
            "&lt;a href=&quot;x&amp;y&quot;&gt;"
        );
    }

    #[test]
    fn html_escape_passthrough() {
        assert_eq!(html_escape("plain text"), "plain text");
    }
}
