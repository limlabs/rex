use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Response};
use futures::stream::{self, StreamExt};
use rex_core::{
    DataStrategy, MiddlewareAction, MiddlewareResult, ProjectConfig, ServerSidePropsContext,
};
use rex_router::RouteTrie;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tracing::{debug, error, info};

use crate::document::{
    assemble_body_tail, assemble_document, assemble_head_shell, assemble_rsc_body_tail,
    assemble_rsc_head_shell, DocumentDescriptor, DocumentParams,
};

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
    /// Pre-serialized manifest JSON (build_id + pages), computed once on construction.
    pub manifest_json: String,
    /// Cached document descriptor from _document rendering. None if no custom _document.
    pub document_descriptor: Option<DocumentDescriptor>,
    /// Whether middleware.ts exists in the project root.
    pub has_middleware: bool,
    /// Middleware matcher patterns (None = no middleware, Some(empty) = run on all).
    pub middleware_matchers: Option<Vec<String>>,
    /// App route trie for app/ router (RSC). None if no app/ directory.
    pub app_route_trie: Option<RouteTrie>,
    /// Whether mcp/ directory has tool files.
    pub has_mcp_tools: bool,
}

impl HotState {
    /// Compute the manifest_json field from current state.
    pub fn compute_manifest_json(build_id: &str, manifest: &rex_build::AssetManifest) -> String {
        let mut json = serde_json::json!({
            "build_id": build_id,
            "pages": manifest.pages,
        });
        if !manifest.app_routes.is_empty() {
            json["app_routes"] = serde_json::to_value(&manifest.app_routes).unwrap_or_default();
        }
        serde_json::to_string(&json).expect("JSON serialization")
    }
}

/// Shared application state
pub struct AppState {
    pub isolate_pool: rex_v8::IsolatePool,
    pub is_dev: bool,
    pub project_root: PathBuf,
    pub image_cache: rex_image::ImageCache,
    pub hot: RwLock<Arc<HotState>>,
}

/// Snapshot the hot state (O(1) Arc clone, no lock held across await).
pub fn snapshot(state: &Arc<AppState>) -> Arc<HotState> {
    Arc::clone(&state.hot.read().expect("HotState lock poisoned"))
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
            let escaped = f
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
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
    hot: &Arc<HotState>,
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

    let document = assemble_document(&DocumentParams {
        ssr_html: &render.body,
        head_html: &render.head,
        props_json: props,
        client_scripts: &[],
        css_files: &hot.manifest.global_css,
        css_contents: &hot.manifest.css_contents,
        app_script: hot.manifest.app_script.as_deref(),
        is_dev: state.is_dev,
        doc_descriptor: hot.document_descriptor.as_ref(),
        manifest_json: Some(&hot.manifest_json),
    });

    (status, Html(document)).into_response()
}

/// Compute document descriptor from V8 _document rendering.
/// Used to populate `HotState.document_descriptor` at build time and on rebuilds.
pub async fn compute_document_descriptor(pool: &rex_v8::IsolatePool) -> Option<DocumentDescriptor> {
    let result = pool.execute(move |iso| iso.render_document()).await;
    match result {
        Ok(Ok(Some(json))) => serde_json::from_str(&json).ok(),
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

    // Run middleware before route matching
    if should_run_middleware(path, &hot) {
        let header_map: HashMap<String, String> = headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        match execute_middleware(&state, path, method.as_str(), &header_map).await {
            Ok(Some(mw)) => match mw.action {
                MiddlewareAction::Redirect => {
                    let url = mw.url.as_deref().unwrap_or("/");
                    let status =
                        StatusCode::from_u16(mw.status).unwrap_or(StatusCode::TEMPORARY_REDIRECT);
                    return Response::builder()
                        .status(status)
                        .header("location", url)
                        .body(Body::empty())
                        .expect("response build");
                }
                MiddlewareAction::Rewrite | MiddlewareAction::Next => {
                    // For API routes, rewrite/next continue normally
                }
            },
            Ok(None) => {}
            Err(e) => {
                error!("Middleware error: {e}");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Middleware error: {e}"),
                )
                    .into_response();
            }
        }
    }

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
    let req_json = serde_json::to_string(&req_data).expect("JSON serialization");

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
            builder
                .body(Body::from(api_res.body))
                .expect("response build")
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
                StatusCode::from_u16(rule.status_code).unwrap_or(StatusCode::TEMPORARY_REDIRECT)
            };
            debug!(from = path, to = %dest, status = %status.as_u16(), "Redirect rule matched");
            return Some(
                Response::builder()
                    .status(status)
                    .header("location", &dest)
                    .body(Body::empty())
                    .expect("response build"),
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

/// Check if middleware should run for the given path.
fn should_run_middleware(path: &str, hot: &HotState) -> bool {
    if !hot.has_middleware {
        return false;
    }
    match &hot.middleware_matchers {
        None => false,
        Some(matchers) => {
            if matchers.is_empty() {
                // No matcher = run on all paths
                return true;
            }
            matchers
                .iter()
                .any(|pattern| ProjectConfig::match_pattern(pattern, path).is_some())
        }
    }
}

/// Execute middleware in V8 and return the result.
async fn execute_middleware(
    state: &Arc<AppState>,
    path: &str,
    method: &str,
    headers: &HashMap<String, String>,
) -> Result<Option<MiddlewareResult>, String> {
    let req_data = serde_json::json!({
        "method": method,
        "url": path,
        "pathname": path,
        "headers": headers,
        "cookies": {},
    });
    let req_json = serde_json::to_string(&req_data).expect("JSON serialization");

    let result = state
        .isolate_pool
        .execute(move |iso| iso.run_middleware(&req_json))
        .await;

    match result {
        Ok(Ok(Some(json))) => match serde_json::from_str::<MiddlewareResult>(&json) {
            Ok(mw) => Ok(Some(mw)),
            Err(e) => Err(format!("Failed to parse middleware result: {e}")),
        },
        Ok(Ok(None)) => Ok(None),
        Ok(Err(e)) => Err(format!("Middleware V8 error: {e}")),
        Err(e) => Err(format!("Middleware pool error: {e}")),
    }
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

    // Run middleware before anything else
    let mut mw_response_headers: Vec<(String, String)> = Vec::new();
    if should_run_middleware(path, &hot) {
        let header_map: HashMap<String, String> = headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        match execute_middleware(&state, path, "GET", &header_map).await {
            Ok(Some(mw)) => match mw.action {
                MiddlewareAction::Redirect => {
                    let url = mw.url.as_deref().unwrap_or("/");
                    let status =
                        StatusCode::from_u16(mw.status).unwrap_or(StatusCode::TEMPORARY_REDIRECT);
                    return Response::builder()
                        .status(status)
                        .header("location", url)
                        .body(Body::empty())
                        .expect("response build");
                }
                MiddlewareAction::Rewrite => {
                    if let Some(url) = &mw.url {
                        // Parse the URL to get just the path
                        let rewrite_path = if let Ok(parsed) = url::Url::parse(url) {
                            parsed.path().to_string()
                        } else {
                            url.clone()
                        };
                        mw_response_headers = mw.response_headers.into_iter().collect();
                        let custom_headers = collect_headers(&rewrite_path, &hot.project_config);
                        let mut response =
                            page_handler_inner(&state, &hot, &rewrite_path, &uri, &headers).await;
                        for (key, value) in custom_headers.into_iter().chain(mw_response_headers) {
                            if let (Ok(name), Ok(val)) = (
                                axum::http::header::HeaderName::from_bytes(key.as_bytes()),
                                axum::http::header::HeaderValue::from_str(&value),
                            ) {
                                response.headers_mut().insert(name, val);
                            }
                        }
                        return response;
                    }
                }
                MiddlewareAction::Next => {
                    mw_response_headers = mw.response_headers.into_iter().collect();
                }
            },
            Ok(None) => {}
            Err(e) => {
                error!("Middleware error: {e}");
                if state.is_dev {
                    return Html(dev_error_overlay("Middleware Error", &e, None)).into_response();
                }
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        }
    }

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

    // Apply custom headers + middleware response headers
    for (key, value) in custom_headers.into_iter().chain(mw_response_headers) {
        if let (Ok(name), Ok(val)) = (
            axum::http::header::HeaderName::from_bytes(key.as_bytes()),
            axum::http::header::HeaderValue::from_str(&value),
        ) {
            response.headers_mut().insert(name, val);
        }
    }

    response
}

/// Render an app/ route using RSC with streaming (head shell + body tail).
async fn render_app_route(
    state: &Arc<AppState>,
    hot: &Arc<HotState>,
    route_match: &rex_core::RouteMatch,
    _path: &str,
    uri: &Uri,
) -> Response {
    let route_key = route_match.route.pattern.clone();
    let params = route_match.params.clone();
    let search_params: HashMap<String, String> = uri
        .query()
        .map(|q| {
            url::form_urlencoded::parse(q.as_bytes())
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let props_json =
        serde_json::json!({ "params": params, "searchParams": search_params }).to_string();

    // Look up client chunks for this app route
    let app_assets = hot.manifest.app_routes.get(&route_key);
    let client_chunks: Vec<String> = app_assets
        .map(|a| a.client_chunks.clone())
        .unwrap_or_default();

    // Serialize client reference manifest
    let client_manifest_json = hot
        .manifest
        .client_reference_manifest
        .as_ref()
        .and_then(|m| serde_json::to_string(m).ok())
        .unwrap_or_else(|| "{}".to_string());

    let is_dev = state.is_dev;
    let manifest_json = hot.manifest_json.clone();

    // Flush head shell immediately so browser starts fetching resources
    let shell = assemble_rsc_head_shell(&client_chunks, &client_manifest_json);

    let shell_chunk = stream::once(async { Ok::<_, std::convert::Infallible>(shell) });

    let state_clone = state.clone();
    let route_key_clone = route_key.clone();
    let props_clone = props_json.clone();
    let client_chunks_clone = client_chunks.clone();
    let client_manifest_json_clone = client_manifest_json.clone();

    let tail_chunk = stream::once(async move {
        let rsc_result = state_clone
            .isolate_pool
            .execute(move |iso| iso.render_rsc_to_html(&route_key_clone, &props_clone))
            .await;

        let (body_html, head_html, flight_data) = match rsc_result {
            Ok(Ok(r)) => (r.body, r.head, r.flight),
            Ok(Err(e)) => {
                error!("RSC render error: {e}");
                let msg = e.to_string().replace('<', "&lt;").replace('>', "&gt;");
                if is_dev {
                    (
                        format!("<pre style=\"padding:20px;color:#e63946;font-family:monospace\">RSC Error: {msg}</pre>"),
                        String::new(),
                        String::new(),
                    )
                } else {
                    (
                        "<h1>Internal Server Error</h1>".to_string(),
                        String::new(),
                        String::new(),
                    )
                }
            }
            Err(e) => {
                error!("RSC pool error: {e}");
                (
                    "<h1>Internal Server Error</h1>".to_string(),
                    String::new(),
                    String::new(),
                )
            }
        };

        let tail = assemble_rsc_body_tail(
            &body_html,
            &head_html,
            &flight_data,
            &client_chunks_clone,
            &client_manifest_json_clone,
            is_dev,
            Some(&manifest_json),
        );

        Ok::<_, std::convert::Infallible>(tail)
    });

    let body = Body::from_stream(shell_chunk.chain(tail_chunk));

    Response::builder()
        .header("content-type", "text/html; charset=utf-8")
        .body(body)
        .expect("response build")
}

async fn page_handler_inner(
    state: &Arc<AppState>,
    hot: &Arc<HotState>,
    path: &str,
    uri: &Uri,
    headers: &HeaderMap,
) -> Response {
    // Check app/ routes first (RSC)
    if let Some(ref app_trie) = hot.app_route_trie {
        if let Some(app_match) = app_trie.match_path(path) {
            return render_app_route(state, hot, &app_match, path, uri).await;
        }
    }

    // Try to match the route
    let route_match = match hot.route_trie.match_path(path) {
        Some(m) => m,
        None => {
            debug!(path, "No route matched");
            if hot.has_custom_404 {
                return render_error_page(state, hot, "404", StatusCode::NOT_FOUND, "{}").await;
            }
            return (
                StatusCode::NOT_FOUND,
                Html("404 - Page Not Found".to_string()),
            )
                .into_response();
        }
    };

    let route_key = route_match.route.module_name();
    let params = route_match.params.clone();

    // Look up data strategy from build manifest (detected at build time)
    let strategy = hot
        .manifest
        .pages
        .get(&route_match.route.pattern)
        .map(|p| &p.data_strategy)
        .cloned()
        .unwrap_or_default();

    // Fetch data props based on strategy
    let route_key_clone = route_key.clone();
    let gssp_result = match strategy {
        DataStrategy::None => {
            // No data function — skip V8 entirely
            Ok(Ok(r#"{"props":{}}"#.to_string()))
        }
        DataStrategy::GetStaticProps => {
            let ctx_json = serde_json::json!({ "params": params }).to_string();
            state
                .isolate_pool
                .execute(move |iso| iso.get_static_props(&route_key_clone, &ctx_json))
                .await
        }
        DataStrategy::GetServerSideProps => {
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

            let header_map: HashMap<String, String> = headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                .collect();

            let context = ServerSidePropsContext {
                params,
                query,
                resolved_url: path.to_string(),
                headers: header_map,
                cookies: HashMap::new(),
            };
            let context_json = serde_json::to_string(&context).expect("JSON serialization");

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
                return Html(dev_error_overlay(
                    "Server Props Error",
                    &e.to_string(),
                    None,
                ))
                .into_response();
            } else if hot.has_custom_error {
                let err_props = serde_json::json!({ "statusCode": 500 }).to_string();
                return render_error_page(
                    state,
                    hot,
                    "_error",
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &err_props,
                )
                .await;
            }
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html(format!("Server error: {e}")),
            )
                .into_response();
        }
        Err(e) => {
            error!("Isolate pool error: {e}");
            if state.is_dev {
                return Html(dev_error_overlay("Runtime Error", &e.to_string(), None))
                    .into_response();
            } else if hot.has_custom_error {
                let err_props = serde_json::json!({ "statusCode": 500 }).to_string();
                return render_error_page(
                    state,
                    hot,
                    "_error",
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &err_props,
                )
                .await;
            }
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html("Internal server error".to_string()),
            )
                .into_response();
        }
    };

    // Single-parse: parse once into Value, check redirect/notFound, extract props
    let parsed = match serde_json::from_str::<serde_json::Value>(&props_json) {
        Ok(val) => val,
        Err(_) => serde_json::json!({"props": {}}),
    };

    // Check for redirect
    if let Some(redirect) = parsed.get("redirect") {
        let destination = redirect
            .get("destination")
            .and_then(|d| d.as_str())
            .unwrap_or("/");
        let permanent = redirect
            .get("permanent")
            .and_then(|p| p.as_bool())
            .unwrap_or(false);
        let status_code = redirect
            .get("statusCode")
            .and_then(|s| s.as_u64())
            .unwrap_or(307) as u16;
        let status = if permanent { 301 } else { status_code };
        debug!(destination, status, "Redirecting");
        return Response::builder()
            .status(status)
            .header("Location", destination)
            .body(Body::empty())
            .expect("response build");
    }

    // Check for notFound
    if parsed
        .get("notFound")
        .and_then(|n| n.as_bool())
        .unwrap_or(false)
    {
        if hot.has_custom_404 {
            return render_error_page(state, hot, "404", StatusCode::NOT_FOUND, "{}").await;
        }
        return (StatusCode::NOT_FOUND, Html("404 - Not Found".to_string())).into_response();
    }

    // Extract props for rendering (already parsed, no re-parse needed)
    let render_props = match parsed.get("props") {
        Some(props) => serde_json::to_string(props).expect("JSON serialization"),
        None => "{}".to_string(),
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

    let manifest_json = hot.manifest_json.clone();
    let shared_chunks = hot.manifest.shared_chunks.clone();
    let app_script = hot.manifest.app_script.clone();
    let is_dev = state.is_dev;

    // Build the HTML head shell: doctype, <head> with CSS + JS modulepreload hints, opening <body>.
    // This is flushed to the browser immediately so it can start fetching resources
    // while V8 renders the page body.
    let shell = assemble_head_shell(
        &css_files,
        &hot.manifest.css_contents,
        &shared_chunks,
        app_script.as_deref(),
        &client_scripts,
        hot.document_descriptor.as_ref(),
    );

    // Stream response in two chunks: shell (immediate) → render + tail (after V8)
    let state_clone = state.clone();
    let route_key_clone = route_key.clone();
    let render_props_clone = render_props.clone();

    let shell_chunk = stream::once(async { Ok::<_, std::convert::Infallible>(shell) });

    let tail_chunk = stream::once(async move {
        let ssr_result = state_clone
            .isolate_pool
            .execute(move |iso| iso.render_page(&route_key_clone, &render_props_clone))
            .await;

        let (body_html, head_html) = match ssr_result {
            Ok(Ok(r)) => (r.body, r.head),
            Ok(Err(e)) => {
                error!("SSR render error: {e}");
                let msg = e.to_string().replace('<', "&lt;").replace('>', "&gt;");
                if is_dev {
                    (format!("<pre style=\"padding:20px;color:#e63946;font-family:monospace\">SSR Error: {msg}</pre>"), String::new())
                } else {
                    ("<h1>Internal Server Error</h1>".to_string(), String::new())
                }
            }
            Err(e) => {
                error!("Isolate pool error: {e}");
                ("<h1>Internal Server Error</h1>".to_string(), String::new())
            }
        };

        let tail = assemble_body_tail(
            &body_html,
            &head_html,
            &render_props,
            &client_scripts,
            app_script.as_deref(),
            is_dev,
            Some(&manifest_json),
        );

        Ok::<_, std::convert::Infallible>(tail)
    });

    let body = Body::from_stream(shell_chunk.chain(tail_chunk));

    Response::builder()
        .header("content-type", "text/html; charset=utf-8")
        .body(body)
        .expect("response build")
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

    // Look up data strategy from build manifest (detected at build time)
    let strategy = hot
        .manifest
        .pages
        .get(&route_match.route.pattern)
        .map(|p| &p.data_strategy)
        .cloned()
        .unwrap_or_default();

    let result = match strategy {
        DataStrategy::None => Ok(Ok(r#"{"props":{}}"#.to_string())),
        DataStrategy::GetStaticProps => {
            let ctx_json = serde_json::json!({ "params": params }).to_string();
            state
                .isolate_pool
                .execute(move |iso| iso.get_static_props(&route_key, &ctx_json))
                .await
        }
        DataStrategy::GetServerSideProps => {
            let header_map: HashMap<String, String> = headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                .collect();
            let context = ServerSidePropsContext {
                params,
                query: HashMap::new(),
                resolved_url: path,
                headers: header_map,
                cookies: HashMap::new(),
            };
            let context_json = serde_json::to_string(&context).expect("JSON serialization");
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
            .expect("response build"),
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

/// RSC flight data endpoint: GET /_rex/rsc/{buildId}/{path}
/// Returns flight data as text/x-component for client-side RSC navigation.
pub async fn rsc_handler(
    State(state): State<Arc<AppState>>,
    Path((build_id, page_path)): Path<(String, String)>,
    uri: Uri,
) -> Response {
    let hot = snapshot(&state);

    // Build ID mismatch = stale client
    if build_id != hot.build_id {
        return StatusCode::NOT_FOUND.into_response();
    }

    let app_route_trie = match &hot.app_route_trie {
        Some(trie) => trie,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    let path = format!("/{page_path}");
    let route_match = match app_route_trie.match_path(&path) {
        Some(m) => m,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    let route_key = &route_match.route.pattern;
    let params = route_match.params.clone();

    // Pass both route params and query string to the RSC render
    let search_params: HashMap<String, String> = uri
        .query()
        .map(|q| {
            url::form_urlencoded::parse(q.as_bytes())
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let props_json =
        serde_json::json!({ "params": params, "searchParams": search_params }).to_string();
    let route_key_owned = route_key.to_string();

    let result = state
        .isolate_pool
        .execute(move |iso| iso.render_rsc_flight(&route_key_owned, &props_json))
        .await;

    match result {
        Ok(Ok(flight_data)) => Response::builder()
            .header("Content-Type", "text/x-component")
            .header("Cache-Control", "no-cache")
            .body(Body::from(flight_data))
            .expect("response build"),
        Ok(Err(e)) => {
            error!("RSC flight render error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
        Err(e) => {
            error!("RSC pool error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Server action handler: POST /_rex/action/{build_id}/{action_id}
///
/// Dispatches a server function call from the client. The request body
/// is a JSON array of arguments. Returns `{ result: ... }` or `{ error: ... }`.
pub async fn server_action_handler(
    State(state): State<Arc<AppState>>,
    Path((build_id, action_id)): Path<(String, String)>,
    body: axum::body::Bytes,
) -> Response {
    let hot = snapshot(&state);

    // Build ID mismatch = stale client
    if build_id != hot.build_id {
        return StatusCode::NOT_FOUND.into_response();
    }

    let args_json = match std::str::from_utf8(&body) {
        Ok(s) => s.to_string(),
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "Invalid UTF-8 body").into_response();
        }
    };

    let action_id_owned = action_id.clone();

    let result = state
        .isolate_pool
        .execute(move |iso| iso.call_server_action(&action_id_owned, &args_json))
        .await;

    match result {
        Ok(Ok(json_result)) => Response::builder()
            .header("Content-Type", "application/json")
            .body(Body::from(json_result))
            .expect("response build"),
        Ok(Err(e)) => {
            error!("Server action error: {e}");
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "error": e.to_string() }).to_string(),
                ))
                .expect("response build")
        }
        Err(e) => {
            error!("Server action pool error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Query parameters for the image optimization endpoint.
#[derive(serde::Deserialize)]
pub struct ImageQuery {
    pub url: String,
    pub w: u32,
    #[serde(default = "default_quality")]
    pub q: u8,
    pub f: Option<String>,
}

fn default_quality() -> u8 {
    75
}

/// Image optimization endpoint: GET /_rex/image?url=/images/hero.jpg&w=640&q=75&f=webp
pub async fn image_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ImageQuery>,
) -> Response {
    // Validate width
    if query.w < 16 || query.w > 4096 {
        return (StatusCode::BAD_REQUEST, "width must be 16–4096").into_response();
    }

    // Determine output format: explicit `f=` param takes priority,
    // otherwise preserve PNG for .png sources (keeps transparency),
    // and use JPEG for everything else.
    let explicit_format = match &query.f {
        Some(f) => match f.as_str() {
            "webp" => Some(rex_image::OutputFormat::WebP),
            "jpeg" | "jpg" => Some(rex_image::OutputFormat::Jpeg),
            "png" => Some(rex_image::OutputFormat::Png),
            _ => return (StatusCode::BAD_REQUEST, "unsupported format").into_response(),
        },
        None => None,
    };
    let format = explicit_format.unwrap_or_else(|| {
        if query.url.ends_with(".png") {
            rex_image::OutputFormat::Png
        } else {
            let accept = headers
                .get("accept")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            rex_image::negotiate_format(accept)
        }
    });

    // Check cache
    let cache_key =
        rex_image::ImageCache::cache_key(&query.url, query.w, query.q, format.extension());

    if let Some(data) = state.image_cache.get(&cache_key) {
        return Response::builder()
            .header("Content-Type", format.content_type())
            .header("Cache-Control", "public, max-age=31536000, immutable")
            .body(Body::from(data))
            .expect("response build");
    }

    // Resolve source file from public/ directory (only local files)
    let url_path = query.url.trim_start_matches('/');
    let file_path = state.project_root.join("public").join(url_path);

    // Prevent path traversal
    let canonical = match file_path.canonicalize() {
        Ok(p) => p,
        Err(_) => return (StatusCode::NOT_FOUND, "image not found").into_response(),
    };
    let public_dir = state.project_root.join("public");
    if let Ok(public_canonical) = public_dir.canonicalize() {
        if !canonical.starts_with(&public_canonical) {
            return (StatusCode::BAD_REQUEST, "invalid path").into_response();
        }
    }

    let src_bytes = match std::fs::read(&canonical) {
        Ok(data) => data,
        Err(_) => return (StatusCode::NOT_FOUND, "image not found").into_response(),
    };

    let params = rex_image::OptimizeParams {
        width: query.w,
        quality: query.q,
        format,
    };

    match rex_image::optimize(&src_bytes, &params) {
        Ok(optimized) => {
            // Cache the result (ignore cache write errors)
            if let Err(e) = state.image_cache.put(&cache_key, &optimized) {
                debug!("image cache write failed: {e}");
            }

            Response::builder()
                .header("Content-Type", format.content_type())
                .header("Cache-Control", "public, max-age=31536000, immutable")
                .body(Body::from(optimized))
                .expect("response build")
        }
        Err(e) => {
            error!("image optimization failed: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
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
                bundle.push_str(&format!("  exports.getServerSideProps = {};\n", gssp_code));
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
    fn build_test_app(routes: Vec<Route>, pages: &[(&str, &str, Option<&str>)]) -> Router {
        rex_v8::init_v8();
        let bundle = format!("{}\n{}", MOCK_REACT_RUNTIME, make_server_bundle(pages));
        let pool =
            rex_v8::IsolatePool::new(1, Arc::new(bundle), None).expect("failed to create pool");

        let trie = RouteTrie::from_routes(&routes);
        let mut manifest = rex_build::AssetManifest::new("test-build-id".to_string());

        // Register pages in manifest with correct data strategy
        for (route, (_, _, gssp)) in routes.iter().zip(pages.iter()) {
            let strategy = if gssp.is_some() {
                DataStrategy::GetServerSideProps
            } else {
                DataStrategy::None
            };
            manifest.add_page(&route.pattern, "test.js", strategy);
        }

        let build_id = "test-build-id".to_string();
        let manifest_json = HotState::compute_manifest_json(&build_id, &manifest);

        let state = Arc::new(AppState {
            isolate_pool: pool,
            is_dev: false,
            project_root: PathBuf::from("/tmp/rex-test"),
            image_cache: rex_image::ImageCache::new(PathBuf::from("/tmp/rex-test-cache")),
            hot: RwLock::new(Arc::new(HotState {
                route_trie: trie,
                api_route_trie: RouteTrie::from_routes(&[]),
                manifest,
                build_id,
                has_custom_404: false,
                has_custom_error: false,
                has_custom_document: false,
                project_config: rex_core::ProjectConfig::default(),
                manifest_json,
                document_descriptor: None,
                has_middleware: false,
                middleware_matchers: None,
                app_route_trie: None,
                has_mcp_tools: false,
            })),
        });

        Router::new()
            .route("/_rex/data/{build_id}/{*path}", get(data_handler))
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
        assert!(
            html.contains("<h1>Home</h1>"),
            "missing SSR content: {html}"
        );
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
        let pool =
            rex_v8::IsolatePool::new(1, Arc::new(bundle), None).expect("failed to create pool");

        let trie = RouteTrie::from_routes(&routes);
        let mut manifest = rex_build::AssetManifest::new("test-build-id".to_string());

        // Register pages in manifest with correct data strategy
        for (route, (_, _, gssp)) in routes.iter().zip(pages.iter()) {
            let strategy = if gssp.is_some() {
                DataStrategy::GetServerSideProps
            } else {
                DataStrategy::None
            };
            manifest.add_page(&route.pattern, "test.js", strategy);
        }

        let build_id = "test-build-id".to_string();
        let manifest_json = HotState::compute_manifest_json(&build_id, &manifest);

        let state = Arc::new(AppState {
            isolate_pool: pool,
            is_dev: false,
            project_root: PathBuf::from("/tmp/rex-test"),
            image_cache: rex_image::ImageCache::new(PathBuf::from("/tmp/rex-test-cache")),
            hot: RwLock::new(Arc::new(HotState {
                route_trie: trie,
                api_route_trie: RouteTrie::from_routes(&[]),
                manifest,
                build_id,
                has_custom_404: false,
                has_custom_error: false,
                has_custom_document: false,
                project_config,
                manifest_json,
                document_descriptor: None,
                has_middleware: false,
                middleware_matchers: None,
                app_route_trie: None,
                has_mcp_tools: false,
            })),
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

        let app = build_test_app_with_config(vec![], &[], config);

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
        assert!(
            html.contains("Home"),
            "rewrite should serve index page: {html}"
        );
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

    /// Build a test app with middleware injected into V8.
    fn build_test_app_with_middleware(
        routes: Vec<Route>,
        pages: &[(&str, &str, Option<&str>)],
        middleware_js: &str,
        matchers: Vec<String>,
    ) -> Router {
        rex_v8::init_v8();
        let mut bundle = format!("{}\n{}", MOCK_REACT_RUNTIME, make_server_bundle(pages));
        bundle.push_str(middleware_js);
        let pool =
            rex_v8::IsolatePool::new(1, Arc::new(bundle), None).expect("failed to create pool");

        let trie = RouteTrie::from_routes(&routes);
        let mut manifest = rex_build::AssetManifest::new("test-build-id".to_string());

        for (route, (_, _, gssp)) in routes.iter().zip(pages.iter()) {
            let strategy = if gssp.is_some() {
                DataStrategy::GetServerSideProps
            } else {
                DataStrategy::None
            };
            manifest.add_page(&route.pattern, "test.js", strategy);
        }

        let build_id = "test-build-id".to_string();
        let manifest_json = HotState::compute_manifest_json(&build_id, &manifest);

        let state = Arc::new(AppState {
            isolate_pool: pool,
            is_dev: false,
            project_root: PathBuf::from("/tmp/rex-test"),
            image_cache: rex_image::ImageCache::new(PathBuf::from("/tmp/rex-test-cache")),
            hot: RwLock::new(Arc::new(HotState {
                route_trie: trie,
                api_route_trie: RouteTrie::from_routes(&[]),
                manifest,
                build_id,
                has_custom_404: false,
                has_custom_error: false,
                has_custom_document: false,
                project_config: rex_core::ProjectConfig::default(),
                manifest_json,
                document_descriptor: None,
                has_middleware: true,
                middleware_matchers: Some(matchers),
                app_route_trie: None,
                has_mcp_tools: false,
            })),
        });

        Router::new()
            .route("/_rex/data/{build_id}/{*path}", get(data_handler))
            .fallback(page_handler)
            .with_state(state)
    }

    /// Minimal middleware runtime for tests (mirrors MIDDLEWARE_RUNTIME from bundler).
    const TEST_MIDDLEWARE_REDIRECT: &str = r#"
        globalThis.__rex_run_middleware = function(reqJson) {
            var req = JSON.parse(reqJson);
            if (req.pathname === '/protected') {
                return JSON.stringify({
                    action: 'redirect',
                    url: '/login',
                    status: 302,
                    request_headers: {},
                    response_headers: {}
                });
            }
            return JSON.stringify({
                action: 'next',
                url: null,
                status: 307,
                request_headers: {},
                response_headers: {}
            });
        };
    "#;

    #[tokio::test]
    async fn test_middleware_redirect() {
        let app = build_test_app_with_middleware(
            vec![
                make_route("/", "index.tsx", vec![]),
                make_route("/login", "login.tsx", vec![]),
                make_route("/protected", "protected.tsx", vec![]),
            ],
            &[
                (
                    "index",
                    "function Index() { return React.createElement('div', null, 'Home'); }",
                    None,
                ),
                (
                    "login",
                    "function Login() { return React.createElement('div', null, 'Login'); }",
                    None,
                ),
                (
                    "protected",
                    "function Protected() { return React.createElement('div', null, 'Secret'); }",
                    None,
                ),
            ],
            TEST_MIDDLEWARE_REDIRECT,
            vec!["/protected".to_string()],
        );

        // /protected should redirect to /login
        let resp = app
            .clone()
            .oneshot(Request::get("/protected").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FOUND); // 302
        assert_eq!(resp.headers().get("location").unwrap(), "/login");

        // / should pass through (not matched by middleware matchers)
        let resp = app
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_middleware_next_passthrough() {
        let app = build_test_app_with_middleware(
            vec![make_route("/", "index.tsx", vec![])],
            &[(
                "index",
                "function Index() { return React.createElement('div', null, 'Home'); }",
                None,
            )],
            TEST_MIDDLEWARE_REDIRECT,
            vec!["/".to_string()],
        );

        // / matches middleware matchers but middleware returns next for non-/protected
        let resp = app
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let html = String::from_utf8_lossy(&body);
        assert!(html.contains("Home"), "should render the page: {html}");
    }
}
