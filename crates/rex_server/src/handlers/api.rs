use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rex_core::MiddlewareAction;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{error, info};

use super::{execute_middleware, should_run_middleware, AppState};
use crate::state::snapshot;

/// API response from V8 handler execution
#[derive(serde::Deserialize)]
pub(super) struct ApiResponse {
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
