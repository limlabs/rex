#![allow(clippy::unwrap_used)]

use crate::handlers::test_support::*;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use rex_core::{DataStrategy, DynamicSegment, Fallback};
use tower::ServiceExt;

#[tokio::test]
async fn test_prerendered_static_path_page_served() {
    let app = TestAppBuilder::new()
        .routes(
            vec![make_route(
                "/posts/:id",
                "posts/[id].tsx",
                vec![DynamicSegment::Single("id".into())],
            )],
            vec![(
                "posts/[id]",
                "function Post(props) { return React.createElement('p', null, props.id); }",
                None,
            )],
        )
        .static_paths_page("/posts/:id", Fallback::False)
        .prerendered(
            "/posts/first",
            "<!DOCTYPE html><html><body><p>first</p></body></html>",
            r#"{"id":"first"}"#,
        )
        .build();

    let resp = app
        .oneshot(Request::get("/posts/first").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers().get("x-rex-render-mode").unwrap(), "static");
    let html = body_string(resp.into_body()).await;
    assert!(
        html.contains("<p>first</p>"),
        "should serve pre-rendered: {html}"
    );
}

#[tokio::test]
async fn test_static_paths_fallback_false_returns_404() {
    let app = TestAppBuilder::new()
        .routes(
            vec![make_route(
                "/posts/:id",
                "posts/[id].tsx",
                vec![DynamicSegment::Single("id".into())],
            )],
            vec![(
                "posts/[id]",
                "function Post(props) { return React.createElement('p', null, props.id); }",
                None,
            )],
        )
        .static_paths_page("/posts/:id", Fallback::False)
        .prerendered(
            "/posts/first",
            "<!DOCTYPE html><html><body><p>first</p></body></html>",
            r#"{"id":"first"}"#,
        )
        .build();

    // Request a path that wasn't pre-rendered — should 404 with fallback: false
    let resp = app
        .oneshot(Request::get("/posts/unknown").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_static_paths_fallback_blocking_renders_ssr() {
    let app = TestAppBuilder::new()
        .routes(
            vec![make_route(
                "/posts/:id",
                "posts/[id].tsx",
                vec![DynamicSegment::Single("id".into())],
            )],
            vec![(
                "posts/[id]",
                "function Post(props) { return React.createElement('p', null, props.id); }",
                Some("function(ctx) { return { props: { id: ctx.params.id } }; }"),
            )],
        )
        .static_paths_page("/posts/:id", Fallback::Blocking)
        .prerendered(
            "/posts/first",
            "<!DOCTYPE html><html><body><p>first</p></body></html>",
            r#"{"id":"first"}"#,
        )
        .build();

    // Request a path not pre-rendered — fallback: blocking should SSR it
    let resp = app
        .oneshot(Request::get("/posts/dynamic").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let html = body_string(resp.into_body()).await;
    assert!(
        html.contains("<p>dynamic</p>"),
        "blocking fallback should SSR the page: {html}"
    );
}

#[tokio::test]
async fn test_static_paths_fallback_false_custom_404() {
    let app = TestAppBuilder::new()
        .routes(
            vec![
                make_route(
                    "/posts/:id",
                    "posts/[id].tsx",
                    vec![DynamicSegment::Single("id".into())],
                ),
                make_route("/404", "404.tsx", vec![]),
            ],
            vec![
                (
                    "posts/[id]",
                    "function Post(props) { return React.createElement('p', null, props.id); }",
                    None,
                ),
                (
                    "404",
                    "function NotFound() { return React.createElement('h1', null, 'Custom 404'); }",
                    None,
                ),
            ],
        )
        .static_paths_page("/posts/:id", Fallback::False)
        .custom_404()
        .build();

    let resp = app
        .oneshot(Request::get("/posts/unknown").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let html = body_string(resp.into_body()).await;
    assert!(
        html.contains("Custom 404"),
        "should render custom 404 page: {html}"
    );
}

#[tokio::test]
async fn test_page_with_get_static_props() {
    let app = TestAppBuilder::new()
        .routes(
            vec![make_route("/", "index.tsx", vec![])],
            vec![(
                "index",
                "function Index(props) { return React.createElement('p', null, props.msg); }",
                Some("GSP:function(ctx) { return { props: { msg: 'from gsp' } }; }"),
            )],
        )
        .page_strategy("/", DataStrategy::GetStaticProps)
        .build();

    let resp = app
        .oneshot(Request::get("/").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let html = body_string(resp.into_body()).await;
    assert!(
        html.contains("<p>from gsp</p>"),
        "GSP props not rendered: {html}"
    );
}

#[tokio::test]
async fn test_page_gsp_redirect() {
    let app = TestAppBuilder::new()
        .routes(
            vec![make_route("/redir", "redir.tsx", vec![])],
            vec![(
                "redir",
                "function Redir() { return React.createElement('div'); }",
                Some("GSP:function(ctx) { return { redirect: { destination: '/target', permanent: false } }; }"),
            )],
        )
        .page_strategy("/redir", DataStrategy::GetStaticProps)
        .build();

    let resp = app
        .oneshot(Request::get("/redir").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(resp.headers().get("location").unwrap(), "/target");
}

#[tokio::test]
async fn test_page_gsp_not_found() {
    let app = TestAppBuilder::new()
        .routes(
            vec![make_route("/gone", "gone.tsx", vec![])],
            vec![(
                "gone",
                "function Gone() { return React.createElement('div'); }",
                Some("GSP:function(ctx) { return { notFound: true }; }"),
            )],
        )
        .page_strategy("/gone", DataStrategy::GetStaticProps)
        .build();

    let resp = app
        .oneshot(Request::get("/gone").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_static_paths_dev_mode_skips_fallback_false() {
    // In dev mode, getStaticPaths pages should always SSR on demand,
    // even with fallback: false (matches Next.js dev behavior).
    let app = TestAppBuilder::new()
        .routes(
            vec![make_route(
                "/posts/:id",
                "posts/[id].tsx",
                vec![DynamicSegment::Single("id".into())],
            )],
            vec![(
                "posts/[id]",
                "function Post(props) { return React.createElement('p', null, props.id); }",
                Some("GSP:function(ctx) { return { props: { id: ctx.params.id } }; }"),
            )],
        )
        .page_strategy("/posts/:id", DataStrategy::GetStaticProps)
        .static_paths_page("/posts/:id", Fallback::False)
        .dev_mode()
        .build();

    // In prod this would 404, but dev mode should SSR it
    let resp = app
        .oneshot(Request::get("/posts/unknown").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let html = body_string(resp.into_body()).await;
    assert!(
        html.contains("<p>unknown</p>"),
        "dev mode should SSR getStaticPaths pages on demand: {html}"
    );
}

#[tokio::test]
async fn test_static_paths_fallback_blocking_with_gsp() {
    let app = TestAppBuilder::new()
        .routes(
            vec![make_route(
                "/posts/:id",
                "posts/[id].tsx",
                vec![DynamicSegment::Single("id".into())],
            )],
            vec![(
                "posts/[id]",
                "function Post(props) { return React.createElement('p', null, props.id); }",
                Some("GSP:function(ctx) { return { props: { id: ctx.params.id } }; }"),
            )],
        )
        .page_strategy("/posts/:id", DataStrategy::GetStaticProps)
        .static_paths_page("/posts/:id", Fallback::Blocking)
        .build();

    let resp = app
        .oneshot(
            Request::get("/posts/on-demand")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let html = body_string(resp.into_body()).await;
    assert!(
        html.contains("<p>on-demand</p>"),
        "blocking fallback with GSP should SSR: {html}"
    );
}
