use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use rex_core::{DataStrategy, Fallback, ServerSidePropsContext};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::error;

use super::AppState;
use crate::state::snapshot;

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

    // Serve pre-rendered data for static path pages
    if let Some(page) = hot
        .prerendered
        .get(&path)
        .or_else(|| hot.prerendered.get(&route_match.route.pattern))
    {
        return Response::builder()
            .header("Content-Type", "application/json")
            .body(Body::from(format!(r#"{{"props":{}}}"#, page.props_json)))
            .expect("response build");
    }

    // If this is a getStaticPaths page with fallback: false, return 404
    if let Some(page_assets) = hot.manifest.pages.get(&route_match.route.pattern) {
        if page_assets.has_static_paths && page_assets.fallback == Fallback::False {
            return StatusCode::NOT_FOUND.into_response();
        }
        // fallback: "blocking" — continue to SSR below
    }

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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use crate::handlers::test_support::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use rex_core::DynamicSegment;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_data_handler_returns_json() {
        let app = TestAppBuilder::new()
            .routes(
                vec![make_route("/about", "about.tsx", vec![])],
                vec![(
                    "about",
                    "function About() { return React.createElement('div'); }",
                    Some("function(ctx) { return { props: { title: 'data test' } }; }"),
                )],
            )
            .build();

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
        let app = TestAppBuilder::new()
            .routes(
                vec![make_route("/", "index.tsx", vec![])],
                vec![(
                    "index",
                    "function Index() { return React.createElement('div'); }",
                    None,
                )],
            )
            .build();

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
        let app = TestAppBuilder::new()
            .routes(
                vec![make_route("/", "index.tsx", vec![])],
                vec![(
                    "index",
                    "function Index() { return React.createElement('div'); }",
                    None,
                )],
            )
            .build();

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

    #[tokio::test]
    async fn test_data_handler_no_gssp_returns_empty_props() {
        let app = TestAppBuilder::new()
            .routes(
                vec![make_route("/static-page", "static.tsx", vec![])],
                vec![(
                    "static",
                    "function Static() { return React.createElement('div', null, 'Static'); }",
                    None,
                )],
            )
            .build();

        let resp = app
            .oneshot(
                Request::get("/_rex/data/test-build-id/static-page.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_string(resp.into_body()).await;
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["props"], serde_json::json!({}));
    }

    #[tokio::test]
    async fn test_data_handler_dynamic_route() {
        let app = TestAppBuilder::new()
            .routes(
                vec![make_route(
                    "/posts/:id",
                    "posts/[id].tsx",
                    vec![DynamicSegment::Single("id".into())],
                )],
                vec![(
                    "posts/[id]",
                    "function Post(props) { return React.createElement('div'); }",
                    Some("function(ctx) { return { props: { id: ctx.params.id } }; }"),
                )],
            )
            .build();

        let resp = app
            .oneshot(
                Request::get("/_rex/data/test-build-id/posts/42.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_string(resp.into_body()).await;
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["props"]["id"], "42");
    }
}
