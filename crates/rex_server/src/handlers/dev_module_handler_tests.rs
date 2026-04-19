use super::test_support::*;
use super::*;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::Router;
use std::collections::HashMap;
use std::path::PathBuf;
use tower::ServiceExt;

/// Helper to build a dev-mode app with src_handler mounted.
fn build_src_app(project_root: PathBuf) -> Router {
    TestAppBuilder::new()
        .routes(
            vec![make_route("/", "index.tsx", vec![])],
            vec![(
                "index",
                "function Index() { return React.createElement('h1', null, 'Home'); }",
                None,
            )],
        )
        .dev_mode()
        .project_root(project_root)
        .custom_router(|state| {
            Router::new()
                .route("/_rex/src/{*path}", get(src_handler))
                .with_state(state)
        })
        .build()
}

/// Helper to build a dev-mode app with entry_handler mounted.
fn build_entry_app(project_root: PathBuf, route_paths: HashMap<String, PathBuf>) -> Router {
    TestAppBuilder::new()
        .routes(
            vec![make_route("/", "index.tsx", vec![])],
            vec![(
                "index",
                "function Index() { return React.createElement('h1', null, 'Home'); }",
                None,
            )],
        )
        .dev_mode()
        .project_root(project_root)
        .route_paths(route_paths)
        .custom_router(|state| {
            Router::new()
                .route("/_rex/entry/{*pattern}", get(entry_handler))
                .with_state(state)
        })
        .build()
}

// ---- src_handler tests ----

#[tokio::test]
async fn src_handler_transforms_tsx_file() {
    let tmp = tempfile::tempdir().unwrap();
    let pages_dir = tmp.path().join("pages");
    std::fs::create_dir_all(&pages_dir).unwrap();
    std::fs::write(
        pages_dir.join("index.tsx"),
        "const x: number = 42;\nexport default function Page() { return <div>hello</div>; }\n",
    )
    .unwrap();

    let app = build_src_app(tmp.path().to_path_buf());

    let resp = app
        .oneshot(
            Request::get("/_rex/src/pages/index.tsx")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp.into_body()).await;
    // TypeScript annotations should be stripped
    assert!(
        !body.contains(": number"),
        "TS type annotations should be stripped: {body}"
    );
    // JSX should be transformed
    assert!(!body.contains("<div>"), "JSX should be transformed: {body}");
    // Should still contain the component logic
    assert!(body.contains("42"), "Should contain the constant: {body}");
}

#[tokio::test]
async fn src_handler_returns_404_for_nonexistent_file() {
    let tmp = tempfile::tempdir().unwrap();
    let app = build_src_app(tmp.path().to_path_buf());

    let resp = app
        .oneshot(
            Request::get("/_rex/src/pages/nope.tsx")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn src_handler_css_returns_empty_module() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("styles.css"), "body { color: red; }").unwrap();

    let app = build_src_app(tmp.path().to_path_buf());

    let resp = app
        .oneshot(
            Request::get("/_rex/src/styles.css")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp.into_body()).await;
    assert_eq!(body, "export default {};");
}

#[tokio::test]
async fn src_handler_css_module_returns_proxy() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("app.module.css"),
        ".container { display: flex; }\n.header-bar { color: blue; }\n",
    )
    .unwrap();

    let app = build_src_app(tmp.path().to_path_buf());

    let resp = app
        .oneshot(
            Request::get("/_rex/src/app.module.css")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp.into_body()).await;
    assert!(
        body.contains("export default"),
        "Should be a JS module: {body}"
    );
    assert!(
        body.contains("container"),
        "Should contain class name: {body}"
    );
    assert!(
        body.contains("headerBar"),
        "Should camelCase kebab-case names: {body}"
    );
}

#[tokio::test]
async fn src_handler_image_returns_url_export() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("logo.png"), "fake png data").unwrap();

    let app = build_src_app(tmp.path().to_path_buf());

    let resp = app
        .oneshot(
            Request::get("/_rex/src/logo.png")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp.into_body()).await;
    assert!(
        body.contains("/_rex/static/logo.png"),
        "Should export static URL: {body}"
    );
    assert!(
        body.contains("export default"),
        "Should be an ES module: {body}"
    );
}

#[tokio::test]
async fn src_handler_not_available_in_production() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("test.tsx"),
        "export default function() { return null; }\n",
    )
    .unwrap();

    // Build with is_dev = false (default)
    let app = TestAppBuilder::new()
        .routes(
            vec![make_route("/", "index.tsx", vec![])],
            vec![(
                "index",
                "function Index() { return React.createElement('h1', null, 'Home'); }",
                None,
            )],
        )
        .project_root(tmp.path().to_path_buf())
        .custom_router(|state| {
            Router::new()
                .route("/_rex/src/{*path}", get(src_handler))
                .with_state(state)
        })
        .build();

    let resp = app
        .oneshot(
            Request::get("/_rex/src/test.tsx")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn src_handler_caches_transform_result() {
    let tmp = tempfile::tempdir().unwrap();
    let pages_dir = tmp.path().join("pages");
    std::fs::create_dir_all(&pages_dir).unwrap();
    std::fs::write(
        pages_dir.join("cached.tsx"),
        "const val: string = 'cached';\nexport default val;\n",
    )
    .unwrap();

    // Build the state so we can reuse it for two requests
    rex_v8::init_v8();
    let bundle = format!(
        "{}\n{}",
        MOCK_REACT_RUNTIME,
        make_server_bundle(&[(
            "index",
            "function Index() { return React.createElement('h1', null, 'Home'); }",
            None,
        )])
    );
    let pool = rex_v8::IsolatePool::new(1, Arc::new(bundle), None).expect("pool");
    let trie = rex_router::RouteTrie::from_routes(&[make_route("/", "index.tsx", vec![])]);
    let mut manifest = rex_core::AssetManifest::new("test-build-id".to_string());
    manifest.add_page("/", "test.js", rex_core::DataStrategy::None, false);
    let build_id = "test-build-id".to_string();
    let manifest_json = crate::state::HotState::compute_manifest_json(&build_id, &manifest);

    let state = Arc::new(crate::state::AppState {
        isolate_pool: pool,
        is_dev: true,
        project_root: tmp.path().to_path_buf(),
        image_cache: rex_image::ImageCache::new(tmp.path().join(".rex-cache")),
        esm: None,
        client_deps: std::sync::OnceLock::new(),
        browser_transform_cache: std::sync::OnceLock::new(),
        lazy_init: tokio::sync::OnceCell::const_new_with(()),
        lazy_init_ctx: std::sync::Mutex::new(None),
        hot: std::sync::RwLock::new(Arc::new(crate::state::HotState {
            route_trie: trie,
            api_route_trie: rex_router::RouteTrie::from_routes(&[]),
            manifest,
            build_id,
            has_custom_404: false,
            has_custom_error: false,
            has_custom_document: false,
            project_config: rex_core::ProjectConfig::default(),
            manifest_json,
            document_descriptor: None,
            has_middleware: false,
            middleware_matchers: None,
            app_route_trie: None,
            app_api_route_trie: None,
            has_mcp_tools: false,
            prerendered: std::collections::HashMap::new(),
            prerendered_app: std::collections::HashMap::new(),
            import_map_json: None,
            route_paths: std::collections::HashMap::new(),
        })),
    });

    let app = Router::new()
        .route("/_rex/src/{*path}", get(src_handler))
        .with_state(state);

    // First request — populates cache
    let resp = app
        .clone()
        .oneshot(
            Request::get("/_rex/src/pages/cached.tsx")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body1 = body_string(resp.into_body()).await;

    // Second request — should hit cache (same content)
    let resp = app
        .oneshot(
            Request::get("/_rex/src/pages/cached.tsx")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body2 = body_string(resp.into_body()).await;

    assert_eq!(body1, body2, "Cached response should match");
    assert!(
        body1.contains("cached"),
        "Should contain the string value: {body1}"
    );
}

#[tokio::test]
async fn src_handler_svg_returns_url_export() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("icon.svg"), "<svg></svg>").unwrap();

    let app = build_src_app(tmp.path().to_path_buf());

    let resp = app
        .oneshot(
            Request::get("/_rex/src/icon.svg")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp.into_body()).await;
    assert!(
        body.contains("/_rex/static/icon.svg"),
        "Should export static URL for SVG: {body}"
    );
}

// ---- entry_handler tests ----

#[tokio::test]
async fn entry_handler_returns_page_entry_module() {
    let tmp = tempfile::tempdir().unwrap();
    let pages_dir = tmp.path().join("pages");
    std::fs::create_dir_all(&pages_dir).unwrap();
    std::fs::write(
        pages_dir.join("about.tsx"),
        "export default function About() { return <div>About</div>; }",
    )
    .unwrap();

    let mut route_paths = HashMap::new();
    route_paths.insert("/about".to_string(), pages_dir.join("about.tsx"));

    let app = build_entry_app(tmp.path().to_path_buf(), route_paths);

    let resp = app
        .oneshot(
            Request::get("/_rex/entry//about")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp.into_body()).await;
    assert!(
        body.contains("hydrateRoot"),
        "Should contain hydration code: {body}"
    );
    assert!(body.contains("__REX_PAGES"), "Should register page: {body}");
    assert!(
        body.contains("pages/about.tsx"),
        "Should reference source path: {body}"
    );
}

#[tokio::test]
async fn entry_handler_returns_index_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let pages_dir = tmp.path().join("pages");
    std::fs::create_dir_all(&pages_dir).unwrap();
    std::fs::write(
        pages_dir.join("index.tsx"),
        "export default function Index() { return <div>Home</div>; }",
    )
    .unwrap();

    let mut route_paths = HashMap::new();
    route_paths.insert("/".to_string(), pages_dir.join("index.tsx"));

    let app = build_entry_app(tmp.path().to_path_buf(), route_paths);

    let resp = app
        .oneshot(Request::get("/_rex/entry//").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp.into_body()).await;
    assert!(
        body.contains("hydrateRoot"),
        "Should contain hydration code: {body}"
    );
    assert!(
        body.contains("'/'"),
        "Should reference root pattern: {body}"
    );
}

#[tokio::test]
async fn entry_handler_returns_404_for_unknown_route() {
    let tmp = tempfile::tempdir().unwrap();
    let app = build_entry_app(tmp.path().to_path_buf(), HashMap::new());

    let resp = app
        .oneshot(
            Request::get("/_rex/entry//nonexistent")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn entry_handler_not_available_in_production() {
    let tmp = tempfile::tempdir().unwrap();
    // Build without dev_mode
    let app = TestAppBuilder::new()
        .routes(
            vec![make_route("/", "index.tsx", vec![])],
            vec![(
                "index",
                "function Index() { return React.createElement('h1', null, 'Home'); }",
                None,
            )],
        )
        .project_root(tmp.path().to_path_buf())
        .custom_router(|state| {
            Router::new()
                .route("/_rex/entry/{*pattern}", get(entry_handler))
                .with_state(state)
        })
        .build();

    let resp = app
        .oneshot(
            Request::get("/_rex/entry//about")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn entry_handler_app_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let pages_dir = tmp.path().join("pages");
    std::fs::create_dir_all(&pages_dir).unwrap();
    std::fs::write(
        pages_dir.join("_app.tsx"),
        "export default function App({ Component, pageProps }) { return <Component {...pageProps} />; }",
    )
    .unwrap();

    let mut route_paths = HashMap::new();
    route_paths.insert("/_app".to_string(), pages_dir.join("_app.tsx"));

    let app = build_entry_app(tmp.path().to_path_buf(), route_paths);

    let resp = app
        .oneshot(
            Request::get("/_rex/entry/_app")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp.into_body()).await;
    assert!(
        body.contains("__REX_APP__"),
        "Should set __REX_APP__: {body}"
    );
    assert!(
        body.contains("pages/_app.tsx"),
        "Should reference _app source: {body}"
    );
}

#[tokio::test]
async fn entry_handler_app_returns_404_when_no_app() {
    let tmp = tempfile::tempdir().unwrap();
    // No /_app in route_paths
    let app = build_entry_app(tmp.path().to_path_buf(), HashMap::new());

    let resp = app
        .oneshot(
            Request::get("/_rex/entry/_app")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn entry_handler_pattern_without_leading_slash() {
    let tmp = tempfile::tempdir().unwrap();
    let pages_dir = tmp.path().join("pages");
    std::fs::create_dir_all(&pages_dir).unwrap();
    std::fs::write(
        pages_dir.join("about.tsx"),
        "export default function About() { return <div>About</div>; }",
    )
    .unwrap();

    let mut route_paths = HashMap::new();
    route_paths.insert("/about".to_string(), pages_dir.join("about.tsx"));

    let app = build_entry_app(tmp.path().to_path_buf(), route_paths);

    // Pattern "about" without leading slash — handler should normalize
    let resp = app
        .oneshot(
            Request::get("/_rex/entry/about")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp.into_body()).await;
    assert!(body.contains("/about"), "Should normalize pattern: {body}");
}
