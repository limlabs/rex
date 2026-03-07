#![allow(clippy::unwrap_used)]

use super::*;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::post;
use axum::Router;
use rex_core::DynamicSegment;
use test_support::*;
use tower::ServiceExt;

#[tokio::test]
async fn test_page_returns_html_with_ssr() {
    let app = TestAppBuilder::new()
        .routes(
            vec![make_route("/", "index.tsx", vec![])],
            vec![(
                "index",
                "function Index() { return React.createElement('h1', null, 'Home'); }",
                None,
            )],
        )
        .build();

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
    let app = TestAppBuilder::new()
        .routes(
            vec![make_route("/", "index.tsx", vec![])],
            vec![(
                "index",
                "function Index() { return React.createElement('h1', null, 'Home'); }",
                None,
            )],
        )
        .build();

    let resp = app
        .oneshot(Request::get("/nonexistent").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_page_with_gssp_props() {
    let app = TestAppBuilder::new()
        .routes(
            vec![make_route("/", "index.tsx", vec![])],
            vec![(
                "index",
                "function Index(props) { return React.createElement('p', null, props.msg); }",
                Some("function(ctx) { return { props: { msg: 'hello from gssp' } }; }"),
            )],
        )
        .build();

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
    let app = TestAppBuilder::new()
        .routes(
            vec![make_route("/old", "old.tsx", vec![])],
            vec![(
                "old",
                "function Old() { return React.createElement('div'); }",
                Some("function(ctx) { return { redirect: { destination: '/new', statusCode: 307 } }; }"),
            )],
        )
        .build();

    let resp = app
        .oneshot(Request::get("/old").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(resp.headers().get("location").unwrap(), "/new");
}

#[tokio::test]
async fn test_page_gssp_not_found() {
    let app = TestAppBuilder::new()
        .routes(
            vec![make_route("/hidden", "hidden.tsx", vec![])],
            vec![(
                "hidden",
                "function Hidden() { return React.createElement('div'); }",
                Some("function(ctx) { return { notFound: true }; }"),
            )],
        )
        .build();

    let resp = app
        .oneshot(Request::get("/hidden").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_dynamic_route_params() {
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
        .build();

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

    let app = TestAppBuilder::new()
        .routes(
            vec![make_route("/new-page", "new.tsx", vec![])],
            vec![(
                "new",
                "function New() { return React.createElement('div', null, 'New'); }",
                None,
            )],
        )
        .config(config)
        .build();

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

    let app = TestAppBuilder::new()
        .routes(
            vec![make_route("/", "index.tsx", vec![])],
            vec![(
                "index",
                "function Index() { return React.createElement('div'); }",
                None,
            )],
        )
        .config(config)
        .build();

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

    let app = TestAppBuilder::new()
        .routes(vec![], vec![])
        .config(config)
        .build();

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

    let app = TestAppBuilder::new()
        .routes(
            vec![make_route("/", "index.tsx", vec![])],
            vec![(
                "index",
                "function Index() { return React.createElement('h1', null, 'Home'); }",
                None,
            )],
        )
        .config(config)
        .build();

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

    let app = TestAppBuilder::new()
        .routes(
            vec![make_route("/", "index.tsx", vec![])],
            vec![(
                "index",
                "function Index() { return React.createElement('div', null, 'Hi'); }",
                None,
            )],
        )
        .config(config)
        .build();

    let resp = app
        .oneshot(Request::get("/").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers().get("x-custom").unwrap(), "hello");
    assert_eq!(resp.headers().get("x-frame-options").unwrap(), "DENY");
}

#[tokio::test]
async fn test_middleware_redirect() {
    let app = TestAppBuilder::new()
        .routes(
            vec![
                make_route("/", "index.tsx", vec![]),
                make_route("/login", "login.tsx", vec![]),
                make_route("/protected", "protected.tsx", vec![]),
            ],
            vec![
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
        )
        .middleware(TEST_MIDDLEWARE_REDIRECT, vec!["/protected".to_string()])
        .build();

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
    let app = TestAppBuilder::new()
        .routes(
            vec![make_route("/", "index.tsx", vec![])],
            vec![(
                "index",
                "function Index() { return React.createElement('div', null, 'Home'); }",
                None,
            )],
        )
        .middleware(TEST_MIDDLEWARE_REDIRECT, vec!["/".to_string()])
        .build();

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

#[tokio::test]
async fn test_server_action_stale_build_id() {
    let app = TestAppBuilder::new()
        .extra_bundle(TEST_ACTION_RUNTIME)
        .custom_router(|state| {
            Router::new()
                .route(
                    "/_rex/action/{build_id}/{action_id}",
                    post(server_action_handler),
                )
                .with_state(state)
        })
        .build();

    let resp = app
        .oneshot(
            Request::post("/_rex/action/wrong-build-id/test_action_id")
                .header("Content-Type", "application/json")
                .body(Body::from("[42]"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_server_action_success() {
    let app = TestAppBuilder::new()
        .extra_bundle(TEST_ACTION_RUNTIME)
        .custom_router(|state| {
            Router::new()
                .route(
                    "/_rex/action/{build_id}/{action_id}",
                    post(server_action_handler),
                )
                .with_state(state)
        })
        .build();

    let resp = app
        .oneshot(
            Request::post("/_rex/action/test-build-id/test_action_id")
                .header("Content-Type", "application/json")
                .body(Body::from("[42]"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp.into_body()).await;
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["result"], 43);
}

#[tokio::test]
async fn test_server_action_not_found() {
    let app = TestAppBuilder::new()
        .extra_bundle(TEST_ACTION_RUNTIME)
        .custom_router(|state| {
            Router::new()
                .route(
                    "/_rex/action/{build_id}/{action_id}",
                    post(server_action_handler),
                )
                .with_state(state)
        })
        .build();

    let resp = app
        .oneshot(
            Request::post("/_rex/action/test-build-id/nonexistent")
                .header("Content-Type", "application/json")
                .body(Body::from("[]"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp.into_body()).await;
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(parsed["error"].as_str().unwrap().contains("not found"));
}

#[tokio::test]
async fn test_server_action_invalid_utf8() {
    let app = TestAppBuilder::new()
        .extra_bundle(TEST_ACTION_RUNTIME)
        .custom_router(|state| {
            Router::new()
                .route(
                    "/_rex/action/{build_id}/{action_id}",
                    post(server_action_handler),
                )
                .with_state(state)
        })
        .build();

    let resp = app
        .oneshot(
            Request::post("/_rex/action/test-build-id/test_action_id")
                .header("Content-Type", "application/json")
                .body(Body::from(vec![0xFF, 0xFE]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
