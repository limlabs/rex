use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use futures::stream::{self, StreamExt};
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

/// Render an app/ route using RSC with streaming (head shell + body tail).
pub(super) async fn render_app_route(
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
