#![allow(clippy::unwrap_used)]

use super::*;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::any;
use axum::Router;
use rex_core::{DynamicSegment, PageType, Route};
use test_support::*;
use tower::ServiceExt;

// ── API handler tests ───────────────────────────────────────────────

#[tokio::test]
async fn test_api_handler_get_returns_json() {
    let api_route = Route {
        pattern: "/api/hello".to_string(),
        file_path: "api/hello.ts".into(),
        abs_path: "/fake/pages/api/hello.ts".into(),
        dynamic_segments: vec![],
        page_type: PageType::Api,
        specificity: 100,
    };

    let app = TestAppBuilder::new()
        .api_routes(vec![api_route])
        .extra_bundle(TEST_API_RUNTIME)
        .custom_router(|state| {
            Router::new()
                .route("/api/{*path}", any(api_handler))
                .with_state(state)
        })
        .build();

    let resp = app
        .oneshot(Request::get("/api/hello").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp.into_body()).await;
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["message"], "hello from api");
}

#[tokio::test]
async fn test_api_handler_post_with_body() {
    let api_route = Route {
        pattern: "/api/hello".to_string(),
        file_path: "api/hello.ts".into(),
        abs_path: "/fake/pages/api/hello.ts".into(),
        dynamic_segments: vec![],
        page_type: PageType::Api,
        specificity: 100,
    };

    let app = TestAppBuilder::new()
        .api_routes(vec![api_route])
        .extra_bundle(TEST_API_RUNTIME)
        .custom_router(|state| {
            Router::new()
                .route("/api/{*path}", any(api_handler))
                .with_state(state)
        })
        .build();

    let resp = app
        .oneshot(
            Request::post("/api/hello")
                .header("Content-Type", "application/json")
                .body(Body::from(r#"{"name":"rex"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp.into_body()).await;
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["echo"]["name"], "rex");
}

#[tokio::test]
async fn test_api_handler_no_route_404() {
    let app = TestAppBuilder::new()
        .extra_bundle(TEST_API_RUNTIME)
        .custom_router(|state| {
            Router::new()
                .route("/api/{*path}", any(api_handler))
                .with_state(state)
        })
        .build();

    let resp = app
        .oneshot(
            Request::get("/api/nonexistent")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ── Custom error page tests ─────────────────────────────────────────

#[tokio::test]
async fn test_custom_404_page() {
    let app = TestAppBuilder::new()
        .routes(
            vec![
                make_route("/", "index.tsx", vec![]),
                make_route("/404", "404.tsx", vec![]),
            ],
            vec![
                (
                    "index",
                    "function Index() { return React.createElement('div', null, 'Home'); }",
                    None,
                ),
                (
                    "404",
                    "function NotFound() { return React.createElement('h1', null, 'Custom 404'); }",
                    None,
                ),
            ],
        )
        .custom_404()
        .build();

    let resp = app
        .oneshot(Request::get("/nonexistent").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let html = body_string(resp.into_body()).await;
    assert!(
        html.contains("Custom 404"),
        "custom 404 page not rendered: {html}"
    );
}

#[tokio::test]
async fn test_custom_error_page_on_gssp_failure() {
    let app = TestAppBuilder::new()
        .routes(
            vec![
                make_route("/broken", "broken.tsx", vec![]),
                make_route("/_error", "_error.tsx", vec![]),
            ],
            vec![
                (
                    "broken",
                    "function Broken() { return React.createElement('div'); }",
                    Some("function(ctx) { throw new Error('boom'); }"),
                ),
                (
                    "_error",
                    "function ErrorPage(props) { return React.createElement('h1', null, 'Error ' + props.statusCode); }",
                    None,
                ),
            ],
        )
        .custom_error()
        .build();

    let resp = app
        .oneshot(Request::get("/broken").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let html = body_string(resp.into_body()).await;
    assert!(
        html.contains("Error 500"),
        "custom error page not rendered: {html}"
    );
}

// ── Dev mode error overlay tests ────────────────────────────────────

#[test]
fn test_dev_error_overlay_escapes_html() {
    let overlay = dev_error_overlay("Test Error", "<script>alert('xss')</script>", None);
    assert!(overlay.contains("&lt;script&gt;"));
    assert!(!overlay.contains("<script>alert"));
    assert!(overlay.contains("Test Error"));
}

#[test]
fn test_dev_error_overlay_with_file_section() {
    let overlay = dev_error_overlay("Build Error", "some error", Some("pages/index.tsx"));
    assert!(overlay.contains("pages/index.tsx"));
    assert!(overlay.contains("Build Error"));
}

#[test]
fn test_dev_error_overlay_hmr_script() {
    let overlay = dev_error_overlay("Error", "msg", None);
    assert!(
        overlay.contains("/_rex/hmr"),
        "should include HMR WebSocket"
    );
    assert!(
        overlay.contains("WebSocket"),
        "should include WebSocket reconnect"
    );
}

// ── Data handler edge cases ─────────────────────────────────────────

#[tokio::test]
async fn test_data_handler_no_gssp_returns_empty_props() {
    let app = TestAppBuilder::new()
        .routes(
            vec![make_route("/static-page", "static.tsx", vec![])],
            vec![(
                "static",
                "function Static() { return React.createElement('div', null, 'Static'); }",
                None, // No GSSP
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

// ── Middleware rewrite tests ────────────────────────────────────────

#[tokio::test]
async fn test_middleware_rewrite() {
    let app = TestAppBuilder::new()
        .routes(
            vec![make_route("/", "index.tsx", vec![])],
            vec![(
                "index",
                "function Index() { return React.createElement('h1', null, 'Home'); }",
                None,
            )],
        )
        .middleware(TEST_MIDDLEWARE_REWRITE, vec!["/rewrite-me".to_string()])
        .build();

    let resp = app
        .oneshot(Request::get("/rewrite-me").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let html = body_string(resp.into_body()).await;
    assert!(
        html.contains("Home"),
        "rewrite should serve index page: {html}"
    );
}

// ── Config rewrite with dynamic params ──────────────────────────────

#[tokio::test]
async fn test_config_rewrite_with_params() {
    let config = rex_core::ProjectConfig {
        rewrites: vec![rex_core::RewriteRule {
            source: "/articles/:slug".to_string(),
            destination: "/blog/:slug".to_string(),
        }],
        ..Default::default()
    };

    let app = TestAppBuilder::new()
        .routes(
            vec![make_route(
                "/blog/:slug",
                "blog/[slug].tsx",
                vec![DynamicSegment::Single("slug".into())],
            )],
            vec![(
                "blog/[slug]",
                "function Post(props) { return React.createElement('h1', null, props.slug); }",
                Some("function(ctx) { return { props: { slug: ctx.params.slug } }; }"),
            )],
        )
        .config(config)
        .build();

    let resp = app
        .oneshot(
            Request::get("/articles/my-post")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let html = body_string(resp.into_body()).await;
    assert!(
        html.contains("<h1>my-post</h1>"),
        "rewrite with params should render: {html}"
    );
}

// ── Catch-all route tests ───────────────────────────────────────────

#[tokio::test]
async fn test_catch_all_route() {
    let app = TestAppBuilder::new()
        .routes(
            vec![make_route(
                "/docs/*path",
                "docs/[...path].tsx",
                vec![DynamicSegment::CatchAll("path".into())],
            )],
            vec![(
                "docs/[...path]",
                "function Docs(props) { return React.createElement('p', null, props.path); }",
                Some("function(ctx) { return { props: { path: ctx.params.path } }; }"),
            )],
        )
        .build();

    let resp = app
        .oneshot(
            Request::get("/docs/getting-started/install")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let html = body_string(resp.into_body()).await;
    assert!(
        html.contains("getting-started/install"),
        "catch-all param not rendered: {html}"
    );
}

// ── Dev mode GSSP error shows overlay ───────────────────────────────

#[tokio::test]
async fn test_dev_mode_gssp_error_shows_overlay() {
    let app = TestAppBuilder::new()
        .routes(
            vec![make_route("/broken", "broken.tsx", vec![])],
            vec![(
                "broken",
                "function Broken() { return React.createElement('div'); }",
                Some("function(ctx) { throw new Error('gssp boom'); }"),
            )],
        )
        .dev_mode()
        .build();

    let resp = app
        .oneshot(Request::get("/broken").body(Body::empty()).unwrap())
        .await
        .unwrap();

    let html = body_string(resp.into_body()).await;
    assert!(
        html.contains("gssp boom"),
        "dev overlay should show error message: {html}"
    );
}

// ── check_redirects unit tests ──────────────────────────────────────

#[test]
fn test_check_redirects_no_match() {
    let config = rex_core::ProjectConfig::default();
    assert!(check_redirects("/anything", &config).is_none());
}

#[test]
fn test_check_redirects_match() {
    let config = rex_core::ProjectConfig {
        redirects: vec![rex_core::RedirectRule {
            source: "/old".to_string(),
            destination: "/new".to_string(),
            status_code: 301,
            permanent: false,
        }],
        ..Default::default()
    };
    let resp = check_redirects("/old", &config).unwrap();
    assert_eq!(resp.status(), 301);
}
