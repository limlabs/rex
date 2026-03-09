#![allow(clippy::unwrap_used)]

use crate::handlers::test_support::*;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use rex_core::PageType;
use tower::ServiceExt;

fn build_app_route_test_app() -> axum::Router {
    let route = rex_core::Route {
        pattern: "/api/hello".to_string(),
        file_path: std::path::PathBuf::from("route.ts"),
        abs_path: std::path::PathBuf::from("/fake/app/api/hello/route.ts"),
        dynamic_segments: vec![],
        page_type: PageType::AppApi,
        specificity: 100,
    };
    TestAppBuilder::new()
        .routes(
            vec![make_route("/", "index.tsx", vec![])],
            vec![(
                "index",
                "function I() { return React.createElement('div'); }",
                None,
            )],
        )
        .app_api_routes(vec![route])
        .extra_bundle(TEST_APP_ROUTE_HANDLER_RUNTIME)
        .build()
}

fn build_app_route_error_app(runtime: &'static str, dev: bool) -> axum::Router {
    let route = rex_core::Route {
        pattern: "/api/err".to_string(),
        file_path: std::path::PathBuf::from("route.ts"),
        abs_path: std::path::PathBuf::from("/fake/app/api/err/route.ts"),
        dynamic_segments: vec![],
        page_type: PageType::AppApi,
        specificity: 100,
    };
    let mut builder = TestAppBuilder::new()
        .routes(
            vec![make_route("/", "index.tsx", vec![])],
            vec![(
                "index",
                "function I() { return React.createElement('div'); }",
                None,
            )],
        )
        .app_api_routes(vec![route])
        .extra_bundle(runtime);
    if dev {
        builder = builder.dev_mode();
    }
    builder.build()
}

#[tokio::test]
async fn test_app_route_handler_get() {
    let resp = build_app_route_test_app()
        .oneshot(Request::get("/api/hello").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp.into_body()).await;
    assert!(body.contains("hello from route handler"), "body: {body}");
}

#[tokio::test]
async fn test_app_route_handler_post() {
    let resp = build_app_route_test_app()
        .oneshot(Request::post("/api/hello").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = body_string(resp.into_body()).await;
    assert!(body.contains("\"created\":true"), "body: {body}");
}

#[tokio::test]
async fn test_app_route_handler_method_not_allowed() {
    let resp = build_app_route_test_app()
        .oneshot(Request::delete("/api/hello").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn test_app_route_handler_with_query_params() {
    let resp = build_app_route_test_app()
        .oneshot(
            Request::get("/api/hello?foo=bar")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_app_route_handler_with_json_body() {
    let resp = build_app_route_test_app()
        .oneshot(
            Request::post("/api/hello")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"key":"value"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn test_app_route_handler_with_text_body() {
    let resp = build_app_route_test_app()
        .oneshot(
            Request::post("/api/hello")
                .body(Body::from("some text body"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn test_page_renders_when_api_trie_present_but_path_unmatched() {
    // When app_api_route_trie exists but path doesn't match an API route,
    // the request should fall through to page rendering.
    let resp = build_app_route_test_app()
        .oneshot(Request::get("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp.into_body()).await;
    assert!(body.contains("<div"), "should render page HTML: {body}");
}

#[tokio::test]
async fn test_app_route_handler_v8_error() {
    let resp = build_app_route_error_app(TEST_APP_ROUTE_HANDLER_THROWS, false)
        .oneshot(Request::get("/api/err").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = body_string(resp.into_body()).await;
    assert!(body.contains("Route handler error"), "body: {body}");
}

#[tokio::test]
async fn test_app_route_handler_v8_error_dev_mode() {
    let resp = build_app_route_error_app(TEST_APP_ROUTE_HANDLER_THROWS, true)
        .oneshot(Request::get("/api/err").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let body = body_string(resp.into_body()).await;
    assert!(
        body.contains("handler exploded"),
        "dev overlay should show error: {body}"
    );
}

#[tokio::test]
async fn test_app_route_handler_bad_json_response() {
    let resp = build_app_route_error_app(TEST_APP_ROUTE_HANDLER_BAD_JSON, false)
        .oneshot(Request::get("/api/err").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = body_string(resp.into_body()).await;
    assert!(body.contains("Internal error"), "body: {body}");
}
