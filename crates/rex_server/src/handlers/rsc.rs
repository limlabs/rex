use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::error;

use super::{AppState, HotState};
use crate::document::{assemble_rsc_body_tail, assemble_rsc_head_shell};
use crate::state::snapshot;

/// RSC flight data endpoint: GET /_rex/rsc/{buildId}/{path}
/// Returns flight data as text/x-component for client-side RSC navigation.
pub async fn rsc_handler(
    State(state): State<Arc<AppState>>,
    Path((build_id, page_path)): Path<(String, String)>,
    uri: Uri,
) -> Response {
    rsc_flight_response(state, &build_id, &page_path, uri).await
}

/// Handler for root RSC flight requests: GET /_rex/rsc/{buildId}
/// Axum's `{*path}` catch-all doesn't match when the path segment is empty,
/// so we need a separate route for the root page.
pub async fn rsc_handler_root(
    State(state): State<Arc<AppState>>,
    Path(build_id): Path<String>,
    uri: Uri,
) -> Response {
    rsc_flight_response(state, &build_id, "", uri).await
}

async fn rsc_flight_response(
    state: Arc<AppState>,
    build_id: &str,
    page_path: &str,
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

    let path = if page_path.is_empty() {
        "/".to_string()
    } else {
        format!("/{page_path}")
    };
    let route_match = match app_route_trie.match_path(&path) {
        Some(m) => m,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    // Automatic static optimization: serve pre-rendered flight data
    if let Some(cached) = hot.prerendered_app.get(&route_match.route.pattern) {
        return Response::builder()
            .header("Content-Type", "text/x-component")
            .header("Cache-Control", "public, max-age=31536000, immutable")
            .header("x-rex-render-mode", "static")
            .body(Body::from(cached.flight.clone()))
            .expect("response build");
    }

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

/// Render an app/ route using RSC with streaming (head shell + body tail).
pub(super) async fn render_app_route(
    state: &Arc<AppState>,
    hot: &Arc<HotState>,
    route_match: &rex_core::RouteMatch,
    _path: &str,
    uri: &Uri,
    req_headers: Option<&axum::http::HeaderMap>,
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
    let css_files = hot.manifest.global_css.clone();
    let css_contents = hot.manifest.css_contents.clone();

    // Serialize request headers/cookies for V8 context (enables next/headers)
    let headers_json = req_headers
        .map(|hm| {
            let map: HashMap<String, String> = hm
                .iter()
                .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
                .collect();
            serde_json::to_string(&map).unwrap_or_else(|_| "{}".to_string())
        })
        .unwrap_or_else(|| "{}".to_string());
    let cookies_json = req_headers
        .and_then(|hm| hm.get("cookie"))
        .and_then(|v| v.to_str().ok())
        .map(|cookie_str| {
            let map: HashMap<String, String> = cookie_str
                .split(';')
                .filter_map(|pair| {
                    let mut parts = pair.splitn(2, '=');
                    let k = parts.next()?.trim().to_string();
                    let v = parts.next().unwrap_or("").trim().to_string();
                    Some((k, v))
                })
                .collect();
            serde_json::to_string(&map).unwrap_or_else(|_| "{}".to_string())
        })
        .unwrap_or_else(|| "{}".to_string());

    // Render RSC first to detect notFound/redirect before streaming headers
    let route_key_clone = route_key.clone();
    let props_clone = props_json.clone();
    let rsc_result = state
        .isolate_pool
        .execute(move |iso| {
            let _ = iso.set_request_context(&headers_json, &cookies_json);
            let result = iso.render_rsc_to_html(&route_key_clone, &props_clone);
            let _ = iso.clear_request_context();
            result
        })
        .await;

    // Handle notFound() → 404 response
    // Handle redirect() → 30x response
    if let Ok(Err(e)) = &rsc_result {
        let msg = e.to_string();
        if msg.contains("__REX_NOT_FOUND__") {
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header("content-type", "text/html; charset=utf-8")
                .body(Body::from(
                    "<html><body><h1>404 - Not Found</h1></body></html>",
                ))
                .expect("response build");
        }
        if let Some(rest) = msg.strip_prefix("__REX_REDIRECT__:") {
            // Format: "status:url"
            if let Some((status_str, url)) = rest.split_once(':') {
                let status = status_str.parse::<u16>().unwrap_or(307);
                return Response::builder()
                    .status(status)
                    .header("location", url)
                    .body(Body::empty())
                    .expect("response build");
            }
        }
    }

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

    // Build head shell + body tail
    let shell = assemble_rsc_head_shell(
        &client_chunks,
        &client_manifest_json,
        &css_files,
        &css_contents,
    );

    let tail = assemble_rsc_body_tail(
        &body_html,
        &head_html,
        &flight_data,
        &client_chunks,
        &client_manifest_json,
        is_dev,
        Some(&manifest_json),
    );

    Response::builder()
        .header("content-type", "text/html; charset=utf-8")
        .body(Body::from(format!("{shell}{tail}")))
        .expect("response build")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::rsc_handler;
    use crate::handlers::test_support::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use axum::Router;
    use rex_core::{DynamicSegment, PageType, Route};
    use tower::ServiceExt;

    fn make_app_route(pattern: &str, file_path: &str, segments: Vec<DynamicSegment>) -> Route {
        let specificity = if segments.is_empty() { 100 } else { 50 };
        Route {
            pattern: pattern.to_string(),
            file_path: file_path.into(),
            abs_path: format!("/fake/app/{file_path}").into(),
            dynamic_segments: segments,
            page_type: PageType::Regular,
            specificity,
        }
    }

    #[tokio::test]
    async fn test_rsc_handler_stale_build_id() {
        let app = TestAppBuilder::new()
            .app_routes(vec![make_app_route("/", "page.tsx", vec![])])
            .extra_bundle(TEST_RSC_FLIGHT_RUNTIME)
            .custom_router(|state| {
                Router::new()
                    .route("/_rex/rsc/{build_id}/{*path}", get(rsc_handler))
                    .with_state(state)
            })
            .build();

        let resp = app
            .oneshot(
                Request::get("/_rex/rsc/wrong-build-id/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_rsc_handler_no_app_trie_404() {
        // No app_routes → app_route_trie is None → 404
        let app = TestAppBuilder::new()
            .extra_bundle(TEST_RSC_FLIGHT_RUNTIME)
            .custom_router(|state| {
                Router::new()
                    .route("/_rex/rsc/{build_id}/{*path}", get(rsc_handler))
                    .with_state(state)
            })
            .build();

        let resp = app
            .oneshot(
                Request::get("/_rex/rsc/test-build-id/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_rsc_handler_no_route_match_404() {
        let app = TestAppBuilder::new()
            .app_routes(vec![make_app_route(
                "/dashboard",
                "dashboard/page.tsx",
                vec![],
            )])
            .extra_bundle(TEST_RSC_FLIGHT_RUNTIME)
            .custom_router(|state| {
                Router::new()
                    .route("/_rex/rsc/{build_id}/{*path}", get(rsc_handler))
                    .with_state(state)
            })
            .build();

        let resp = app
            .oneshot(
                Request::get("/_rex/rsc/test-build-id/nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_rsc_handler_returns_flight_data() {
        let app = TestAppBuilder::new()
            .app_routes(vec![make_app_route(
                "/dashboard",
                "dashboard/page.tsx",
                vec![],
            )])
            .extra_bundle(TEST_RSC_FLIGHT_RUNTIME)
            .custom_router(|state| {
                Router::new()
                    .route("/_rex/rsc/{build_id}/{*path}", get(rsc_handler))
                    .with_state(state)
            })
            .build();

        let resp = app
            .oneshot(
                Request::get("/_rex/rsc/test-build-id/dashboard")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "text/x-component"
        );
        let body = body_string(resp.into_body()).await;
        let val: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(val["route"], "/dashboard");
    }
}
