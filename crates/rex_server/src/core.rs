use rex_core::{MiddlewareResult, ProjectConfig};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::debug;

use crate::state::{AppState, HotState};

// Handler implementations live in core_handlers.rs
pub use crate::core_handlers::{handle_api, handle_data, handle_image, handle_page};

/// Framework-agnostic HTTP request.
#[derive(Debug, Clone)]
pub struct RexRequest {
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

/// Framework-agnostic HTTP response.
#[derive(Debug, Clone)]
pub struct RexResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: RexBody,
}

/// Response body variants.
#[derive(Debug, Clone)]
pub enum RexBody {
    Full(Vec<u8>),
    Empty,
}

impl RexResponse {
    pub fn html(status: u16, html: String) -> Self {
        Self {
            status,
            headers: vec![(
                "content-type".to_string(),
                "text/html; charset=utf-8".to_string(),
            )],
            body: RexBody::Full(html.into_bytes()),
        }
    }

    pub fn json(status: u16, json: String) -> Self {
        Self {
            status,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: RexBody::Full(json.into_bytes()),
        }
    }

    pub fn redirect(status: u16, location: &str) -> Self {
        Self {
            status,
            headers: vec![("location".to_string(), location.to_string())],
            body: RexBody::Empty,
        }
    }

    pub fn text(status: u16, text: String) -> Self {
        Self {
            status,
            headers: vec![("content-type".to_string(), "text/plain".to_string())],
            body: RexBody::Full(text.into_bytes()),
        }
    }

    pub fn bytes(status: u16, content_type: &str, data: Vec<u8>) -> Self {
        Self {
            status,
            headers: vec![("content-type".to_string(), content_type.to_string())],
            body: RexBody::Full(data),
        }
    }

    pub fn not_found() -> Self {
        Self::text(404, "404 - Page Not Found".to_string())
    }

    pub fn internal_error() -> Self {
        Self::text(500, "Internal Server Error".to_string())
    }
}

impl RexRequest {
    /// Parse query string into key-value pairs.
    pub fn query_params(&self) -> HashMap<String, String> {
        self.query
            .as_deref()
            .map(|q| {
                url::form_urlencoded::parse(q.as_bytes())
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Result of matching a route, with the pattern and params.
#[derive(Debug, Clone)]
pub struct RouteMatchResult {
    pub pattern: String,
    pub module_name: String,
    pub params: HashMap<String, String>,
}

/// Match a request path against the page route trie.
pub fn match_route(hot: &HotState, path: &str) -> Option<RouteMatchResult> {
    hot.route_trie.match_path(path).map(|m| RouteMatchResult {
        pattern: m.route.pattern.clone(),
        module_name: m.route.module_name(),
        params: m.params,
    })
}

/// Check redirect rules and return redirect response if matched.
pub fn check_redirects(path: &str, config: &ProjectConfig) -> Option<RexResponse> {
    for rule in &config.redirects {
        if let Some(params) = ProjectConfig::match_pattern(&rule.source, path) {
            let dest = ProjectConfig::apply_params(&rule.destination, &params);
            let status = if rule.permanent {
                308
            } else {
                rule.status_code
            };
            debug!(from = path, to = %dest, status, "Redirect rule matched");
            return Some(RexResponse::redirect(status, &dest));
        }
    }
    None
}

/// Check rewrite rules and return the rewritten path if matched.
pub fn check_rewrites(path: &str, config: &ProjectConfig) -> Option<String> {
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
pub fn collect_custom_headers(path: &str, config: &ProjectConfig) -> Vec<(String, String)> {
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
pub fn should_run_middleware(path: &str, hot: &HotState) -> bool {
    if !hot.has_middleware {
        return false;
    }
    match &hot.middleware_matchers {
        None => false,
        Some(matchers) => {
            if matchers.is_empty() {
                return true;
            }
            matchers
                .iter()
                .any(|pattern| ProjectConfig::match_pattern(pattern, path).is_some())
        }
    }
}

/// Execute middleware in V8 and return the result.
pub async fn execute_middleware(
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

/// Convert a `RexBody` to a `String` (lossy UTF-8).
pub fn body_to_string(body: &RexBody) -> String {
    match body {
        RexBody::Full(bytes) => String::from_utf8_lossy(bytes).to_string(),
        RexBody::Empty => String::new(),
    }
}

/// Top-level request dispatcher — routes to page/api/data/image based on path.
///
/// Does NOT handle static file serving (/_rex/static/*) — that must be handled
/// by the caller (Axum's ServeDir, or NAPI reading from disk).
pub async fn handle_request(
    state: &Arc<AppState>,
    hot: &Arc<HotState>,
    req: &RexRequest,
) -> RexResponse {
    let path = &req.path;

    // /_rex/router.js
    if path == "/_rex/router.js" {
        return RexResponse {
            status: 200,
            headers: vec![(
                "content-type".to_string(),
                "application/javascript".to_string(),
            )],
            body: RexBody::Full(
                include_str!(concat!(env!("OUT_DIR"), "/router.js"))
                    .as_bytes()
                    .to_vec(),
            ),
        };
    }

    // /_rex/data/{buildId}/{path}.json
    if let Some(rest) = path.strip_prefix("/_rex/data/") {
        if let Some(slash_pos) = rest.find('/') {
            let build_id = &rest[..slash_pos];
            let page_path = &rest[slash_pos + 1..];
            return handle_data(state, hot, build_id, page_path, req).await;
        }
    }

    // /_rex/image
    if path == "/_rex/image" {
        return handle_image(state, req).await;
    }

    // /mcp — handled by Axum router directly, but for NAPI/core dispatch:
    if path == "/mcp" {
        return RexResponse {
            status: 405,
            headers: vec![("content-type".to_string(), "text/plain".to_string())],
            body: RexBody::Full(b"MCP endpoint requires POST via Axum router".to_vec()),
        };
    }

    // /api/*
    if path.starts_with("/api/") || path == "/api" {
        return handle_api(state, hot, req).await;
    }

    // Page SSR (everything else)
    handle_page(state, hot, req).await
}
