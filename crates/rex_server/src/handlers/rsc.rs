use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::error;

use super::AppState;
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
