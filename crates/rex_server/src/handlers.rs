use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Response};
use rex_core::{ProjectConfig, ServerSidePropsContext, ServerSidePropsResult};
use rex_router::RouteTrie;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tracing::{debug, error, info};

use crate::document::{assemble_document, DocumentDescriptor};

/// State that can change during dev-mode rebuilds.
#[derive(Clone)]
pub struct HotState {
    pub route_trie: RouteTrie,
    pub api_route_trie: RouteTrie,
    pub manifest: rex_build::AssetManifest,
    pub build_id: String,
    pub has_custom_404: bool,
    pub has_custom_error: bool,
    pub has_custom_document: bool,
    pub project_config: ProjectConfig,
}

/// Shared application state
pub struct AppState {
    pub isolate_pool: rex_v8::IsolatePool,
    pub is_dev: bool,
    pub hot: RwLock<HotState>,
}

/// Snapshot the hot state (cheap clone, no lock held across await).
fn snapshot(state: &Arc<AppState>) -> HotState {
    state.hot.read().unwrap().clone()
}

/// Generate a full-page error overlay for dev mode.
/// Includes HMR WebSocket connection for auto-reload on fix.
fn dev_error_overlay(title: &str, message: &str, file: Option<&str>) -> String {
    let escaped_message = message
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let file_section = file
        .map(|f| {
            let escaped = f.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
            format!(r#"<div class="file">{escaped}</div>"#)
        })
        .unwrap_or_default();

    format!(
        r#"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1"/>
<title>{title}</title>
<style>
* {{ margin: 0; padding: 0; box-sizing: border-box; }}
body {{
  background: #1a1a2e;
  color: #e0e0e0;
  font-family: 'SF Mono', 'Fira Code', 'JetBrains Mono', Menlo, Consolas, monospace;
  min-height: 100vh;
  display: flex;
  align-items: flex-start;
  justify-content: center;
  padding: 60px 20px;
}}
.container {{
  max-width: 860px;
  width: 100%;
}}
.badge {{
  display: inline-block;
  background: #e63946;
  color: #fff;
  font-size: 11px;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  padding: 4px 10px;
  border-radius: 4px;
  margin-bottom: 16px;
}}
h1 {{
  font-size: 22px;
  font-weight: 600;
  color: #fff;
  margin-bottom: 20px;
  line-height: 1.4;
}}
.file {{
  color: #8892b0;
  font-size: 13px;
  margin-bottom: 16px;
  padding: 8px 12px;
  background: rgba(255,255,255,0.04);
  border-radius: 6px;
  border-left: 3px solid #e63946;
}}
.stack {{
  background: #0d1117;
  border: 1px solid rgba(255,255,255,0.08);
  border-radius: 8px;
  padding: 20px;
  overflow-x: auto;
  font-size: 13px;
  line-height: 1.7;
  white-space: pre-wrap;
  word-wrap: break-word;
  color: #f0c674;
}}
.hint {{
  margin-top: 24px;
  font-size: 12px;
  color: #555;
}}
.dot {{
  display: inline-block;
  width: 8px;
  height: 8px;
  border-radius: 50%;
  margin-right: 8px;
  background: #e63946;
  animation: pulse 2s infinite;
}}
.dot.connected {{ background: #2ecc71; animation: none; }}
@keyframes pulse {{ 0%,100% {{ opacity: 1; }} 50% {{ opacity: 0.3; }} }}
</style>
</head>
<body>
<div class="container">
  <div class="badge">{title}</div>
  {file_section}
  <div class="stack">{escaped_message}</div>
  <div class="hint"><span class="dot" id="dot"></span><span id="status">Waiting for changes...</span></div>
</div>
<script>
(function() {{
  var proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
  var ws = new WebSocket(proto + '//' + location.host + '/_rex/hmr');
  var dot = document.getElementById('dot');
  var status = document.getElementById('status');
  ws.onopen = function() {{
    dot.className = 'dot connected';
    status.textContent = 'Connected — save a file to reload';
  }};
  ws.onmessage = function(e) {{
    try {{ var m = JSON.parse(e.data); }} catch(x) {{ return; }}
    if (m.type === 'update' || m.type === 'full-reload') location.reload();
  }};
  ws.onclose = function() {{
    dot.className = 'dot';
    status.textContent = 'Disconnected — retrying...';
    setTimeout(function() {{ location.reload(); }}, 2000);
  }};
}})();
</script>
</body>
</html>"#
    )
}

/// Render a custom error page (404 or _error) via SSR, returning the full HTML document.
async fn render_error_page(
    state: &Arc<AppState>,
    hot: &HotState,
    page_key: &str,
    status: StatusCode,
    props: &str,
) -> Response {
    let key = page_key.to_string();
    let props_clone = props.to_string();
    let ssr_result = state
        .isolate_pool
        .execute(move |iso| iso.render_page(&key, &props_clone))
        .await;

    let render = match ssr_result {
        Ok(Ok(r)) => r,
        _ => return (status, Html(format!("{} Error", status.as_u16()))).into_response(),
    };

    // Render _document if present
    let doc_desc = get_document_descriptor(state, hot).await;

    let manifest_json = serde_json::to_string(&serde_json::json!({
        "build_id": hot.build_id,
        "pages": hot.manifest.pages,
    })).unwrap();

    let document = assemble_document(
        &render.body,
        &render.head,
        props,
        &hot.manifest.vendor_scripts,
        &[],
        &hot.manifest.global_css,
        hot.manifest.app_script.as_deref(),
        &hot.build_id,
        state.is_dev,
        doc_desc.as_ref(),
        Some(&manifest_json),
    );

    (status, Html(document)).into_response()
}

/// Get document descriptor from _document rendering, if present.
async fn get_document_descriptor(state: &Arc<AppState>, hot: &HotState) -> Option<DocumentDescriptor> {
    if !hot.has_custom_document {
        return None;
    }
    let result = state
        .isolate_pool
        .execute(move |iso| iso.render_document())
        .await;
    match result {
        Ok(Ok(Some(json))) => {
            serde_json::from_str(&json).ok()
        }
        _ => None,
    }
}

/// API response from V8 handler execution
#[derive(serde::Deserialize)]
struct ApiResponse {
    #[serde(rename = "statusCode")]
    status_code: u16,
    #[serde(default)]
    headers: HashMap<String, String>,
    #[serde(default)]
    body: String,
}

/// API route handler - handles all HTTP methods for /api/* routes
pub async fn api_handler(
    State(state): State<Arc<AppState>>,
    method: axum::http::Method,
    uri: Uri,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let path = uri.path();
    info!(path, method = %method, "Handling API request");

    let hot = snapshot(&state);
    let route_match = match hot.api_route_trie.match_path(path) {
        Some(m) => m,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    let route_key = route_match.route.module_name();

    // Parse query string
    let query: HashMap<String, String> = uri
        .query()
        .map(|q| {
            url::form_urlencoded::parse(q.as_bytes())
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        })
        .unwrap_or_default();

    // Parse body based on content-type
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let body_value = if content_type.starts_with("application/json") {
        serde_json::from_slice::<serde_json::Value>(&body).unwrap_or(serde_json::Value::Null)
    } else if !body.is_empty() {
        serde_json::Value::String(String::from_utf8_lossy(&body).into_owned())
    } else {
        serde_json::Value::Null
    };

    // Build request JSON for V8
    let req_data = serde_json::json!({
        "method": method.as_str(),
        "url": path,
        "headers": headers.iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect::<HashMap<String, String>>(),
        "query": query,
        "body": body_value,
        "cookies": {},
    });
    let req_json = serde_json::to_string(&req_data).unwrap();

    // Execute in V8
    let result = state
        .isolate_pool
        .execute(move |iso| iso.call_api_handler(&route_key, &req_json))
        .await;

    match result {
        Ok(Ok(json)) => {
            let api_res: ApiResponse = match serde_json::from_str(&json) {
                Ok(r) => r,
                Err(e) => {
                    error!("Failed to parse API response: {e}");
                    return (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response();
                }
            };

            let status = StatusCode::from_u16(api_res.status_code).unwrap_or(StatusCode::OK);
            let mut builder = Response::builder().status(status);
            for (k, v) in &api_res.headers {
                builder = builder.header(k.as_str(), v.as_str());
            }
            builder.body(Body::from(api_res.body)).unwrap()
        }
        Ok(Err(e)) => {
            error!("API handler V8 error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, format!("API error: {e}")).into_response()
        }
        Err(e) => {
            error!("API handler pool error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Check redirect rules and return early if matched.
fn check_redirects(path: &str, config: &ProjectConfig) -> Option<Response> {
    for rule in &config.redirects {
        if let Some(params) = ProjectConfig::match_pattern(&rule.source, path) {
            let dest = ProjectConfig::apply_params(&rule.destination, &params);
            let status = if rule.permanent {
                StatusCode::PERMANENT_REDIRECT // 308
            } else {
                StatusCode::from_u16(rule.status_code)
                    .unwrap_or(StatusCode::TEMPORARY_REDIRECT)
            };
            debug!(from = path, to = %dest, status = %status.as_u16(), "Redirect rule matched");
            return Some(
                Response::builder()
                    .status(status)
                    .header("location", &dest)
                    .body(Body::empty())
                    .unwrap(),
            );
        }
    }
    None
}

/// Check rewrite rules and return the rewritten path if matched.
fn check_rewrites(path: &str, config: &ProjectConfig) -> Option<String> {
    for rule in &config.rewrites {
        if let Some(params) = ProjectConfig::match_pattern(&rule.source, path) {
            let dest = ProjectConfig::apply_params(&rule.destination, &params);
            debug!(from = path, to = %dest, "Rewrite rule matched");
            return Some(dest);
        }
    }
    None
}

/// Collect custom headers that match the given path.
fn collect_headers(path: &str, config: &ProjectConfig) -> Vec<(String, String)> {
    let mut result = Vec::new();
    for rule in &config.headers {
        if ProjectConfig::match_pattern(&rule.source, path).is_some() {
            for entry in &rule.headers {
                result.push((entry.key.clone(), entry.value.clone()));
            }
        }
    }
    result
}

/// Main page handler - catches all routes and performs SSR
pub async fn page_handler(
    State(state): State<Arc<AppState>>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let path = uri.path();
    info!(path, "Handling page request");

    let hot = snapshot(&state);

    // Check redirect rules first
    if let Some(redirect_response) = check_redirects(path, &hot.project_config) {
        return redirect_response;
    }

    // Check rewrite rules (transparently serve a different path)
    let effective_path;
    let path = if let Some(rewritten) = check_rewrites(path, &hot.project_config) {
        effective_path = rewritten;
        &effective_path
    } else {
        path
    };

    // Collect custom headers to add to the response
    let custom_headers = collect_headers(path, &hot.project_config);

    let mut response = page_handler_inner(&state, &hot, path, &uri, &headers).await;

    // Apply custom headers
    for (key, value) in custom_headers {
        if let (Ok(name), Ok(val)) = (
            axum::http::header::HeaderName::from_bytes(key.as_bytes()),
            axum::http::header::HeaderValue::from_str(&value),
        ) {
            response.headers_mut().insert(name, val);
        }
    }

    response
}

async fn page_handler_inner(
    state: &Arc<AppState>,
    hot: &HotState,
    path: &str,
    uri: &Uri,
    headers: &HeaderMap,
) -> Response {

    // Try to match the route
    let route_match = match hot.route_trie.match_path(path) {
        Some(m) => m,
        None => {
            debug!(path, "No route matched");
            if hot.has_custom_404 {
                return render_error_page(&state, &hot, "404", StatusCode::NOT_FOUND, "{}").await;
            }
            return (StatusCode::NOT_FOUND, Html("404 - Page Not Found".to_string())).into_response();
        }
    };

    let route_key = route_match.route.module_name();
    let params = route_match.params.clone();

    // Parse query string
    let query: HashMap<String, String> = uri
        .query()
        .map(|q| {
            url::form_urlencoded::parse(q.as_bytes())
                .map(|(k, v): (std::borrow::Cow<str>, std::borrow::Cow<str>)| {
                    (k.to_string(), v.to_string())
                })
                .collect()
        })
        .unwrap_or_default();

    // Extract headers
    let header_map: HashMap<String, String> = headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();

    // Build GSSP context
    let context = ServerSidePropsContext {
        params,
        query,
        resolved_url: path.to_string(),
        headers: header_map,
        cookies: HashMap::new(),
    };
    let context_json = serde_json::to_string(&context).unwrap();

    // Detect data strategy and execute the appropriate data function
    let route_key_for_detect = route_key.clone();
    let strategy = state
        .isolate_pool
        .execute(move |iso| iso.detect_data_strategy(&route_key_for_detect))
        .await
        .ok()
        .and_then(|r| r.ok())
        .unwrap_or_else(|| "getServerSideProps".to_string());

    let route_key_clone = route_key.clone();
    let gssp_result = match strategy.as_str() {
        "getStaticProps" => {
            // In dev mode, re-execute GSP on each request (like Next.js)
            let ctx_json = serde_json::json!({ "params": context.params }).to_string();
            state
                .isolate_pool
                .execute(move |iso| iso.get_static_props(&route_key_clone, &ctx_json))
                .await
        }
        _ => {
            // getServerSideProps or none
            state
                .isolate_pool
                .execute(move |iso| iso.get_server_side_props(&route_key_clone, &context_json))
                .await
        }
    };

    let props_json = match gssp_result {
        Ok(Ok(json)) => json,
        Ok(Err(e)) => {
            error!("GSSP error: {e}");
            if state.is_dev {
                return Html(dev_error_overlay("Server Props Error", &e.to_string(), None)).into_response();
            } else if hot.has_custom_error {
                let err_props = serde_json::json!({ "statusCode": 500 }).to_string();
                return render_error_page(&state, &hot, "_error", StatusCode::INTERNAL_SERVER_ERROR, &err_props).await;
            }
            return (StatusCode::INTERNAL_SERVER_ERROR, Html(format!("Server error: {e}"))).into_response();
        }
        Err(e) => {
            error!("Isolate pool error: {e}");
            if state.is_dev {
                return Html(dev_error_overlay("Runtime Error", &e.to_string(), None)).into_response();
            } else if hot.has_custom_error {
                let err_props = serde_json::json!({ "statusCode": 500 }).to_string();
                return render_error_page(&state, &hot, "_error", StatusCode::INTERNAL_SERVER_ERROR, &err_props).await;
            }
            return (StatusCode::INTERNAL_SERVER_ERROR, Html("Internal server error".to_string())).into_response();
        }
    };

    // Parse GSSP result to check for redirect/notFound
    match serde_json::from_str::<ServerSidePropsResult>(&props_json) {
        Ok(ServerSidePropsResult::Redirect { redirect }) => {
            let status = if redirect.permanent { 301 } else { redirect.status_code };
            debug!(destination = %redirect.destination, status, "Redirecting");
            return Response::builder()
                .status(status)
                .header("Location", &redirect.destination)
                .body(Body::empty())
                .unwrap();
        }
        Ok(ServerSidePropsResult::NotFound { not_found: true }) => {
            if hot.has_custom_404 {
                return render_error_page(&state, &hot, "404", StatusCode::NOT_FOUND, "{}").await;
            }
            return (StatusCode::NOT_FOUND, Html("404 - Not Found".to_string())).into_response();
        }
        _ => {}
    }

    // Extract just the props value for rendering
    let render_props = match serde_json::from_str::<serde_json::Value>(&props_json) {
        Ok(val) => {
            if let Some(props) = val.get("props") {
                serde_json::to_string(props).unwrap()
            } else {
                "{}".to_string()
            }
        }
        Err(_) => "{}".to_string(),
    };

    // Render the page
    let route_key_clone = route_key.clone();
    let render_props_clone = render_props.clone();
    let ssr_result = state
        .isolate_pool
        .execute(move |iso| iso.render_page(&route_key_clone, &render_props_clone))
        .await;

    let render = match ssr_result {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            error!("SSR render error: {e}");
            if state.is_dev {
                return Html(dev_error_overlay("SSR Error", &e.to_string(), None)).into_response();
            } else if hot.has_custom_error {
                let err_props = serde_json::json!({ "statusCode": 500 }).to_string();
                return render_error_page(&state, &hot, "_error", StatusCode::INTERNAL_SERVER_ERROR, &err_props).await;
            } else {
                return (StatusCode::INTERNAL_SERVER_ERROR, Html("Internal server error".to_string())).into_response();
            }
        }
        Err(e) => {
            error!("Isolate pool error: {e}");
            if state.is_dev {
                return Html(dev_error_overlay("Runtime Error", &e.to_string(), None)).into_response();
            } else if hot.has_custom_error {
                let err_props = serde_json::json!({ "statusCode": 500 }).to_string();
                return render_error_page(&state, &hot, "_error", StatusCode::INTERNAL_SERVER_ERROR, &err_props).await;
            }
            return (StatusCode::INTERNAL_SERVER_ERROR, Html("Internal server error".to_string())).into_response();
        }
    };

    // Look up client assets for this route
    let page_assets = hot.manifest.pages.get(&route_match.route.pattern);
    let client_scripts: Vec<String> = page_assets
        .map(|assets| vec![assets.js.clone()])
        .unwrap_or_default();

    // Collect CSS: global (from _app) + per-page
    let mut css_files = hot.manifest.global_css.clone();
    if let Some(assets) = page_assets {
        css_files.extend(assets.css.iter().cloned());
    }

    // Render _document if present
    let doc_desc = get_document_descriptor(&state, &hot).await;

    let manifest_json = serde_json::to_string(&serde_json::json!({
        "build_id": hot.build_id,
        "pages": hot.manifest.pages,
    })).unwrap();

    let document = assemble_document(
        &render.body,
        &render.head,
        &render_props,
        &hot.manifest.vendor_scripts,
        &client_scripts,
        &css_files,
        hot.manifest.app_script.as_deref(),
        &hot.build_id,
        state.is_dev,
        doc_desc.as_ref(),
        Some(&manifest_json),
    );

    Html(document).into_response()
}

/// Data endpoint: GET /_rex/data/{buildId}/{path}.json
/// Returns GSSP result as JSON for client-side navigation
pub async fn data_handler(
    State(state): State<Arc<AppState>>,
    Path((build_id, page_path)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let hot = snapshot(&state);

    // Build ID mismatch = stale client
    if build_id != hot.build_id {
        return StatusCode::NOT_FOUND.into_response();
    }

    let path = format!("/{}", page_path.trim_end_matches(".json"));

    let route_match = match hot.route_trie.match_path(&path) {
        Some(m) => m,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    let route_key = route_match.route.module_name();
    let params = route_match.params.clone();

    // Detect data strategy
    let route_key_detect = route_key.clone();
    let strategy = state
        .isolate_pool
        .execute(move |iso| iso.detect_data_strategy(&route_key_detect))
        .await
        .ok()
        .and_then(|r| r.ok())
        .unwrap_or_else(|| "getServerSideProps".to_string());

    let result = match strategy.as_str() {
        "getStaticProps" => {
            let ctx_json = serde_json::json!({ "params": params }).to_string();
            state
                .isolate_pool
                .execute(move |iso| iso.get_static_props(&route_key, &ctx_json))
                .await
        }
        _ => {
            let context = ServerSidePropsContext {
                params,
                query: HashMap::new(),
                resolved_url: path,
                headers: headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                    .collect(),
                cookies: HashMap::new(),
            };
            let context_json = serde_json::to_string(&context).unwrap();
            state
                .isolate_pool
                .execute(move |iso| iso.get_server_side_props(&route_key, &context_json))
                .await
        }
    };

    match result {
        Ok(Ok(json)) => Response::builder()
            .header("Content-Type", "application/json")
            .body(Body::from(json))
            .unwrap(),
        Ok(Err(e)) => {
            error!("Data endpoint GSSP error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
        Err(e) => {
            error!("Data endpoint pool error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use axum::routing::get;
    use axum::Router;
    use http_body_util::BodyExt;
    use rex_core::{DynamicSegment, PageType, Route};
    use std::path::PathBuf;
    use tower::ServiceExt;

    /// Same minimal React stub as rex_v8 tests.
    const MOCK_REACT_RUNTIME: &str = r#"
        globalThis.__React = {
            createElement: function(type, props) {
                var children = Array.prototype.slice.call(arguments, 2);
                return { type: type, props: props || {}, children: children };
            }
        };
        var React = globalThis.__React;

        function renderElement(el) {
            if (el === null || el === undefined) return '';
            if (typeof el === 'string') return el;
            if (typeof el === 'number') return String(el);
            if (Array.isArray(el)) return el.map(renderElement).join('');
            if (typeof el.type === 'function') {
                var merged = Object.assign({}, el.props);
                if (el.children.length > 0) merged.children = el.children.length === 1 ? el.children[0] : el.children;
                return renderElement(el.type(merged));
            }
            if (typeof el.type === 'string') {
                var attrs = '';
                var p = el.props || {};
                for (var k in p) {
                    if (k === 'children') continue;
                    if (p.hasOwnProperty(k)) attrs += ' ' + k + '="' + p[k] + '"';
                }
                var inner = '';
                if (p.children) inner += renderElement(p.children);
                inner += el.children.map(renderElement).join('');
                if (!inner) return '<' + el.type + attrs + '/>';
                return '<' + el.type + attrs + '>' + inner + '</' + el.type + '>';
            }
            return '';
        }

        globalThis.__ReactDOMServer = {
            renderToString: function(el) { return renderElement(el); }
        };
    "#;

    fn make_server_bundle(pages: &[(&str, &str, Option<&str>)]) -> String {
        let mut bundle = String::new();
        bundle.push_str("'use strict';\n");
        bundle.push_str("globalThis.__rex_pages = globalThis.__rex_pages || {};\n\n");

        for (key, component, gssp) in pages {
            bundle.push_str(&format!(
                "globalThis.__rex_pages['{}'] = (function() {{\n  var exports = {{}};\n",
                key
            ));
            bundle.push_str(&format!("  exports.default = {};\n", component));
            if let Some(gssp_code) = gssp {
                bundle.push_str(&format!(
                    "  exports.getServerSideProps = {};\n",
                    gssp_code
                ));
            }
            bundle.push_str("  return exports;\n})();\n\n");
        }

        bundle.push_str(
            r#"
globalThis.__rex_head_elements = [];
globalThis.__rex_head_component = function Head(props) {
    if (props.children) {
        var children = Array.isArray(props.children) ? props.children : [props.children];
        for (var i = 0; i < children.length; i++) {
            if (children[i]) globalThis.__rex_head_elements.push(children[i]);
        }
    }
    return null;
};

globalThis.__rex_render_page = function(routeKey, propsJson) {
    var React = globalThis.__React;
    var ReactDOMServer = globalThis.__ReactDOMServer;
    if (!React || !ReactDOMServer) throw new Error('React not loaded');
    var page = globalThis.__rex_pages[routeKey];
    if (!page) throw new Error('Page not found: ' + routeKey);
    var Component = page.default;
    if (!Component) throw new Error('No default export: ' + routeKey);
    var props = JSON.parse(propsJson);

    globalThis.__rex_head_elements = [];
    var bodyHtml = ReactDOMServer.renderToString(React.createElement(Component, props));
    var headHtml = '';
    for (var i = 0; i < globalThis.__rex_head_elements.length; i++) {
        headHtml += ReactDOMServer.renderToString(globalThis.__rex_head_elements[i]);
    }
    return JSON.stringify({ body: bodyHtml, head: headHtml });
};

globalThis.__rex_gssp_resolved = null;
globalThis.__rex_gssp_rejected = null;

globalThis.__rex_get_server_side_props = function(routeKey, contextJson) {
    var page = globalThis.__rex_pages[routeKey];
    if (!page || !page.getServerSideProps) return JSON.stringify({ props: {} });
    var context = JSON.parse(contextJson);
    var result = page.getServerSideProps(context);
    if (result && typeof result.then === 'function') {
        globalThis.__rex_gssp_resolved = null;
        globalThis.__rex_gssp_rejected = null;
        result.then(
            function(v) { globalThis.__rex_gssp_resolved = v; },
            function(e) { globalThis.__rex_gssp_rejected = e; }
        );
        return '__REX_ASYNC__';
    }
    return JSON.stringify(result);
};

globalThis.__rex_resolve_gssp = function() {
    if (globalThis.__rex_gssp_rejected) throw globalThis.__rex_gssp_rejected;
    if (globalThis.__rex_gssp_resolved !== null) return JSON.stringify(globalThis.__rex_gssp_resolved);
    throw new Error('GSSP promise did not resolve');
};

globalThis.__rex_detect_data_strategy = function(routeKey) {
    var page = globalThis.__rex_pages[routeKey];
    if (!page) return 'none';
    if (page.getStaticProps && page.getServerSideProps) {
        throw new Error('Page exports both getStaticProps and getServerSideProps.');
    }
    if (page.getStaticProps) return 'getStaticProps';
    if (page.getServerSideProps) return 'getServerSideProps';
    return 'none';
};

globalThis.__rex_gsp_resolved = null;
globalThis.__rex_gsp_rejected = null;

globalThis.__rex_get_static_props = function(routeKey, contextJson) {
    var page = globalThis.__rex_pages[routeKey];
    if (!page || !page.getStaticProps) return JSON.stringify({ props: {} });
    var context = JSON.parse(contextJson);
    var result = page.getStaticProps(context);
    if (result && typeof result.then === 'function') {
        globalThis.__rex_gsp_resolved = null;
        globalThis.__rex_gsp_rejected = null;
        result.then(
            function(v) { globalThis.__rex_gsp_resolved = v; },
            function(e) { globalThis.__rex_gsp_rejected = e; }
        );
        return '__REX_GSP_ASYNC__';
    }
    return JSON.stringify(result);
};

globalThis.__rex_resolve_gsp = function() {
    if (globalThis.__rex_gsp_rejected) throw globalThis.__rex_gsp_rejected;
    if (globalThis.__rex_gsp_resolved !== null) return JSON.stringify(globalThis.__rex_gsp_resolved);
    throw new Error('GSP promise did not resolve');
};
"#,
        );
        bundle
    }

    fn make_route(pattern: &str, file_path: &str, segments: Vec<DynamicSegment>) -> Route {
        let specificity = if segments.is_empty() { 100 } else { 50 };
        Route {
            pattern: pattern.to_string(),
            file_path: PathBuf::from(file_path),
            abs_path: PathBuf::from(format!("/fake/pages/{file_path}")),
            dynamic_segments: segments,
            page_type: PageType::Regular,
            specificity,
        }
    }

    /// Build a test app with router, isolate pool, and routes wired up.
    fn build_test_app(
        routes: Vec<Route>,
        pages: &[(&str, &str, Option<&str>)],
    ) -> Router {
        rex_v8::init_v8();
        let bundle = format!("{}\n{}", MOCK_REACT_RUNTIME, make_server_bundle(pages));
        let pool = rex_v8::IsolatePool::new(
            1,
            Arc::new(bundle),
        )
        .expect("failed to create pool");

        let trie = RouteTrie::from_routes(&routes);
        let manifest = rex_build::AssetManifest::new("test-build-id".to_string());

        let state = Arc::new(AppState {
            isolate_pool: pool,
            is_dev: false,
            hot: RwLock::new(HotState {
                route_trie: trie,
                api_route_trie: RouteTrie::from_routes(&[]),
                manifest,
                build_id: "test-build-id".to_string(),
                has_custom_404: false,
                has_custom_error: false,
                has_custom_document: false,
                project_config: rex_core::ProjectConfig::default(),
            }),
        });

        Router::new()
            .route(
                "/_rex/data/{build_id}/{*path}",
                get(data_handler),
            )
            .fallback(page_handler)
            .with_state(state)
    }

    async fn body_string(body: Body) -> String {
        let bytes = body.collect().await.unwrap().to_bytes();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn test_page_returns_html_with_ssr() {
        let app = build_test_app(
            vec![make_route("/", "index.tsx", vec![])],
            &[(
                "index",
                "function Index() { return React.createElement('h1', null, 'Home'); }",
                None,
            )],
        );

        let resp = app
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let html = body_string(resp.into_body()).await;
        assert!(html.contains("<h1>Home</h1>"), "missing SSR content: {html}");
        assert!(html.contains("<!DOCTYPE html>"), "missing doctype: {html}");
        assert!(html.contains("__REX_DATA__"), "missing data script: {html}");
    }

    #[tokio::test]
    async fn test_page_404_no_route() {
        let app = build_test_app(
            vec![make_route("/", "index.tsx", vec![])],
            &[(
                "index",
                "function Index() { return React.createElement('h1', null, 'Home'); }",
                None,
            )],
        );

        let resp = app
            .oneshot(Request::get("/nonexistent").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_page_with_gssp_props() {
        let app = build_test_app(
            vec![make_route("/", "index.tsx", vec![])],
            &[(
                "index",
                "function Index(props) { return React.createElement('p', null, props.msg); }",
                Some("function(ctx) { return { props: { msg: 'hello from gssp' } }; }"),
            )],
        );

        let resp = app
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let html = body_string(resp.into_body()).await;
        assert!(
            html.contains("<p>hello from gssp</p>"),
            "GSSP props not rendered: {html}"
        );
    }

    #[tokio::test]
    async fn test_page_gssp_redirect() {
        let app = build_test_app(
            vec![make_route("/old", "old.tsx", vec![])],
            &[(
                "old",
                "function Old() { return React.createElement('div'); }",
                Some("function(ctx) { return { redirect: { destination: '/new', statusCode: 307 } }; }"),
            )],
        );

        let resp = app
            .oneshot(Request::get("/old").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(resp.headers().get("location").unwrap(), "/new");
    }

    #[tokio::test]
    async fn test_page_gssp_not_found() {
        let app = build_test_app(
            vec![make_route("/hidden", "hidden.tsx", vec![])],
            &[(
                "hidden",
                "function Hidden() { return React.createElement('div'); }",
                Some("function(ctx) { return { notFound: true }; }"),
            )],
        );

        let resp = app
            .oneshot(Request::get("/hidden").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_dynamic_route_params() {
        let app = build_test_app(
            vec![make_route(
                "/blog/:slug",
                "blog/[slug].tsx",
                vec![DynamicSegment::Single("slug".into())],
            )],
            &[(
                "blog/[slug]",
                "function Post(props) { return React.createElement('h1', null, props.slug); }",
                Some("function(ctx) { return { props: { slug: ctx.params.slug } }; }"),
            )],
        );

        let resp = app
            .oneshot(Request::get("/blog/my-post").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let html = body_string(resp.into_body()).await;
        assert!(
            html.contains("<h1>my-post</h1>"),
            "dynamic param not passed: {html}"
        );
    }

    #[tokio::test]
    async fn test_data_handler_returns_json() {
        let app = build_test_app(
            vec![make_route("/about", "about.tsx", vec![])],
            &[(
                "about",
                "function About() { return React.createElement('div'); }",
                Some("function(ctx) { return { props: { title: 'data test' } }; }"),
            )],
        );

        let resp = app
            .oneshot(
                Request::get("/_rex/data/test-build-id/about.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/json"
        );
        let json = body_string(resp.into_body()).await;
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["props"]["title"], "data test");
    }

    #[tokio::test]
    async fn test_data_handler_stale_build_id() {
        let app = build_test_app(
            vec![make_route("/", "index.tsx", vec![])],
            &[(
                "index",
                "function Index() { return React.createElement('div'); }",
                None,
            )],
        );

        let resp = app
            .oneshot(
                Request::get("/_rex/data/wrong-build-id/index.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_data_handler_no_route() {
        let app = build_test_app(
            vec![make_route("/", "index.tsx", vec![])],
            &[(
                "index",
                "function Index() { return React.createElement('div'); }",
                None,
            )],
        );

        let resp = app
            .oneshot(
                Request::get("/_rex/data/test-build-id/nonexistent.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    fn build_test_app_with_config(
        routes: Vec<Route>,
        pages: &[(&str, &str, Option<&str>)],
        project_config: rex_core::ProjectConfig,
    ) -> Router {
        rex_v8::init_v8();
        let bundle = format!("{}\n{}", MOCK_REACT_RUNTIME, make_server_bundle(pages));
        let pool = rex_v8::IsolatePool::new(1, Arc::new(bundle)).expect("failed to create pool");

        let trie = RouteTrie::from_routes(&routes);
        let manifest = rex_build::AssetManifest::new("test-build-id".to_string());

        let state = Arc::new(AppState {
            isolate_pool: pool,
            is_dev: false,
            hot: RwLock::new(HotState {
                route_trie: trie,
                api_route_trie: RouteTrie::from_routes(&[]),
                manifest,
                build_id: "test-build-id".to_string(),
                has_custom_404: false,
                has_custom_error: false,
                has_custom_document: false,
                project_config,
            }),
        });

        Router::new()
            .route("/_rex/data/{build_id}/{*path}", get(data_handler))
            .fallback(page_handler)
            .with_state(state)
    }

    #[tokio::test]
    async fn test_config_redirect() {
        let config = rex_core::ProjectConfig {
            redirects: vec![rex_core::RedirectRule {
                source: "/old-page".to_string(),
                destination: "/new-page".to_string(),
                status_code: 307,
                permanent: false,
            }],
            ..Default::default()
        };

        let app = build_test_app_with_config(
            vec![make_route("/new-page", "new.tsx", vec![])],
            &[(
                "new",
                "function New() { return React.createElement('div', null, 'New'); }",
                None,
            )],
            config,
        );

        let resp = app
            .oneshot(Request::get("/old-page").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(resp.headers().get("location").unwrap(), "/new-page");
    }

    #[tokio::test]
    async fn test_config_redirect_permanent() {
        let config = rex_core::ProjectConfig {
            redirects: vec![rex_core::RedirectRule {
                source: "/legacy".to_string(),
                destination: "/modern".to_string(),
                status_code: 308,
                permanent: true,
            }],
            ..Default::default()
        };

        let app = build_test_app_with_config(
            vec![make_route("/", "index.tsx", vec![])],
            &[(
                "index",
                "function Index() { return React.createElement('div'); }",
                None,
            )],
            config,
        );

        let resp = app
            .oneshot(Request::get("/legacy").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::PERMANENT_REDIRECT);
        assert_eq!(resp.headers().get("location").unwrap(), "/modern");
    }

    #[tokio::test]
    async fn test_config_redirect_with_params() {
        let config = rex_core::ProjectConfig {
            redirects: vec![rex_core::RedirectRule {
                source: "/blog/:slug".to_string(),
                destination: "/posts/:slug".to_string(),
                status_code: 307,
                permanent: false,
            }],
            ..Default::default()
        };

        let app = build_test_app_with_config(
            vec![],
            &[],
            config,
        );

        let resp = app
            .oneshot(Request::get("/blog/hello").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(resp.headers().get("location").unwrap(), "/posts/hello");
    }

    #[tokio::test]
    async fn test_config_rewrite() {
        let config = rex_core::ProjectConfig {
            rewrites: vec![rex_core::RewriteRule {
                source: "/docs".to_string(),
                destination: "/".to_string(),
            }],
            ..Default::default()
        };

        let app = build_test_app_with_config(
            vec![make_route("/", "index.tsx", vec![])],
            &[(
                "index",
                "function Index() { return React.createElement('h1', null, 'Home'); }",
                None,
            )],
            config,
        );

        // /docs should be rewritten to / and serve the index page
        let resp = app
            .oneshot(Request::get("/docs").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let html = body_string(resp.into_body()).await;
        assert!(html.contains("Home"), "rewrite should serve index page: {html}");
    }

    #[tokio::test]
    async fn test_config_custom_headers() {
        let config = rex_core::ProjectConfig {
            headers: vec![rex_core::HeaderRule {
                source: "/".to_string(),
                headers: vec![
                    rex_core::HeaderEntry {
                        key: "X-Custom".to_string(),
                        value: "hello".to_string(),
                    },
                    rex_core::HeaderEntry {
                        key: "X-Frame-Options".to_string(),
                        value: "DENY".to_string(),
                    },
                ],
            }],
            ..Default::default()
        };

        let app = build_test_app_with_config(
            vec![make_route("/", "index.tsx", vec![])],
            &[(
                "index",
                "function Index() { return React.createElement('div', null, 'Hi'); }",
                None,
            )],
            config,
        );

        let resp = app
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get("x-custom").unwrap(), "hello");
        assert_eq!(resp.headers().get("x-frame-options").unwrap(), "DENY");
    }
}
