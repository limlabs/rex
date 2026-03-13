#![allow(clippy::unwrap_used)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use rex_build::transform::TransformCache;
use rex_dev::dev_middleware::DevMiddleware;
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

fn setup_middleware(files: &[(&str, &str)], deps: &[(&str, &str)]) -> (DevMiddleware, TempDir) {
    let dir = TempDir::new().unwrap();
    for (path, content) in files {
        let file_path = dir.path().join(path);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&file_path, content).unwrap();
    }

    let client_deps: HashMap<String, String> = deps
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    let mw = DevMiddleware::new(
        Arc::new(TransformCache::new()),
        client_deps,
        dir.path().to_path_buf(),
        HashMap::new(),
    );
    (mw, dir)
}

#[tokio::test]
async fn serve_tsx_source() {
    let (mw, _dir) = setup_middleware(
        &[("pages/index.tsx", "const x: number = 1; export default x;")],
        &[],
    );
    let router = mw.into_router();

    let resp = router
        .oneshot(
            Request::builder()
                .uri("/_rex/dev/pages/index.tsx")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(ct.contains("application/javascript"));

    let cc = resp
        .headers()
        .get("cache-control")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(cc.contains("no-cache"));

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let js = String::from_utf8(body.to_vec()).unwrap();

    // TypeScript annotation should be stripped
    assert!(!js.contains(": number"));
    // Variable declaration should remain
    assert!(js.contains("const x"));
}

#[tokio::test]
async fn serve_source_not_found() {
    let (mw, _dir) = setup_middleware(&[], &[]);
    let router = mw.into_router();

    let resp = router
        .oneshot(
            Request::builder()
                .uri("/_rex/dev/pages/missing.tsx")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn serve_dep() {
    let react_js = "export const createElement = () => {};";
    let (mw, _dir) = setup_middleware(&[], &[("react.js", react_js)]);
    let router = mw.into_router();

    let resp = router
        .oneshot(
            Request::builder()
                .uri("/_rex/deps/react.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(ct.contains("application/javascript"));

    let cc = resp
        .headers()
        .get("cache-control")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(cc.contains("immutable"));

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(String::from_utf8(body.to_vec()).unwrap(), react_js);
}

#[tokio::test]
async fn serve_dep_not_found() {
    let (mw, _dir) = setup_middleware(&[], &[]);
    let router = mw.into_router();

    let resp = router
        .oneshot(
            Request::builder()
                .uri("/_rex/deps/nonexistent.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn serve_jsx_transforms_correctly() {
    let jsx_source = r#"
export default function App() {
    return React.createElement("div", null, "hello");
}
"#;
    let (mw, _dir) = setup_middleware(&[("components/App.jsx", jsx_source)], &[]);
    let router = mw.into_router();

    let resp = router
        .oneshot(
            Request::builder()
                .uri("/_rex/dev/components/App.jsx")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let js = String::from_utf8(body.to_vec()).unwrap();
    assert!(js.contains("function App"));
}

#[tokio::test]
async fn transform_cache_shared_across_requests() {
    let cache = Arc::new(TransformCache::new());
    let dir = TempDir::new().unwrap();
    let src = "const x: string = 'hello'; export default x;";
    std::fs::create_dir_all(dir.path().join("pages")).unwrap();
    std::fs::write(dir.path().join("pages/test.ts"), src).unwrap();

    let mw = DevMiddleware::new(
        cache.clone(),
        HashMap::new(),
        dir.path().to_path_buf(),
        HashMap::new(),
    );
    let router = mw.into_router();

    // First request populates cache
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/_rex/dev/pages/test.ts")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Cache should now have the entry
    let cached = cache
        .get_cached(dir.path().join("pages/test.ts").as_path())
        .unwrap();
    assert!(!cached.contains(": string"));
    assert!(cached.contains("const x"));
}

#[tokio::test]
async fn serve_dev_entry() {
    let mut page_entries = HashMap::new();
    page_entries.insert("/".to_string(), "pages/index.tsx".to_string());
    page_entries.insert("/about".to_string(), "pages/about.tsx".to_string());

    let mw = DevMiddleware::new(
        Arc::new(TransformCache::new()),
        HashMap::new(),
        std::env::temp_dir(),
        page_entries,
    );
    let router = mw.into_router();

    let resp = router
        .oneshot(
            Request::builder()
                .uri("/_rex/dev-entry/pages/index.tsx")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let js = String::from_utf8(body.to_vec()).unwrap();

    // Should import from /_rex/dev/ path
    assert!(js.contains("/_rex/dev/pages/index.tsx"));
    // Should register on __REX_PAGES with route pattern
    assert!(js.contains("__REX_PAGES['/']"));
    // Should include hydration code
    assert!(js.contains("hydrateRoot"));
}

#[tokio::test]
async fn serve_dev_entry_not_found() {
    let mw = DevMiddleware::new(
        Arc::new(TransformCache::new()),
        HashMap::new(),
        std::env::temp_dir(),
        HashMap::new(),
    );
    let router = mw.into_router();

    let resp = router
        .oneshot(
            Request::builder()
                .uri("/_rex/dev-entry/pages/missing.tsx")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
