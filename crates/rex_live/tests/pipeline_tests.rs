#![allow(clippy::unwrap_used)]

//! Full pipeline integration tests: build + SSR + handler.
//!
//! These tests exercise compile_project, ensure_built, and the live handler
//! with real rolldown compilation and V8 SSR using mock node_modules.

use rex_live::cache::BuildCache;
use rex_live::source::LocalSource;
use std::fs;
use std::time::SystemTime;
use tempfile::TempDir;

/// Create mock node_modules with minimal React stubs (mirrors rex_build tests).
fn setup_mock_node_modules(root: &std::path::Path) {
    let nm = root.join("node_modules");

    // react
    let react_dir = nm.join("react");
    fs::create_dir_all(&react_dir).unwrap();
    fs::write(
        react_dir.join("package.json"),
        r#"{"name":"react","version":"19.0.0","main":"index.js"}"#,
    )
    .unwrap();
    fs::write(
        react_dir.join("index.js"),
        "export function createElement(type, props, ...children) { return { type, props, children }; }\nexport const Suspense = Symbol.for('react.suspense');\nexport default { createElement, Suspense };\n",
    )
    .unwrap();
    fs::write(
        react_dir.join("jsx-runtime.js"),
        "export function jsx(type, props) { return { type, props }; }\nexport function jsxs(type, props) { return { type, props }; }\nexport const Fragment = 'Fragment';\nexport const Suspense = Symbol.for('react.suspense');\n",
    )
    .unwrap();
    fs::write(
        react_dir.join("jsx-dev-runtime.js"),
        "export function jsxDEV(type, props) { return { type, props }; }\nexport const Fragment = 'Fragment';\nexport const Suspense = Symbol.for('react.suspense');\n",
    )
    .unwrap();

    // react-dom
    let react_dom_dir = nm.join("react-dom");
    fs::create_dir_all(&react_dom_dir).unwrap();
    fs::write(
        react_dom_dir.join("package.json"),
        r#"{"name":"react-dom","version":"19.0.0","main":"index.js","exports":{".":{  "default":"./index.js"},"./client":{"default":"./client.js"},"./server":{"default":"./server.js"}}}"#,
    )
    .unwrap();
    fs::write(react_dom_dir.join("index.js"), "export default {};\n").unwrap();
    fs::write(
        react_dom_dir.join("client.js"),
        "export function hydrateRoot() {}\nexport function createRoot() {}\n",
    )
    .unwrap();
    fs::write(
        react_dom_dir.join("server.js"),
        r#"
var _Suspense = Symbol.for('react.suspense');
function renderEl(el) {
  if (el == null || el === false) return '';
  if (typeof el === 'string') return el;
  if (typeof el === 'number') return '' + el;
  if (Array.isArray(el)) return el.map(renderEl).join('');
  if (typeof el === 'object' && el.type) {
    if (el.type === _Suspense) {
      try {
        var inner = '';
        if (el.children && el.children.length) inner += el.children.map(renderEl).join('');
        var pr = el.props || {};
        if (pr.children != null && !(el.children && el.children.length)) inner += renderEl(pr.children);
        return inner;
      } catch (e) {
        if (e && typeof e.then === 'function') {
          return renderEl(el.props && el.props.fallback);
        }
        throw e;
      }
    }
    if (typeof el.type === 'function') {
      var p = Object.assign({}, el.props || {});
      if (el.children && el.children.length) p.children = el.children.length === 1 ? el.children[0] : el.children;
      return renderEl(el.type(p));
    }
    var tag = el.type;
    var attrs = '';
    var pr = el.props || {};
    for (var k in pr) {
      if (k === 'children' || k === 'key' || k === 'ref') continue;
      if (typeof pr[k] === 'function' || typeof pr[k] === 'object') continue;
      if (k === 'className') attrs += ' class="' + pr[k] + '"';
      else attrs += ' ' + k + '="' + pr[k] + '"';
    }
    var ch = '';
    if (el.children && el.children.length) ch += el.children.map(renderEl).join('');
    if (pr.children != null && !(el.children && el.children.length)) ch += renderEl(pr.children);
    return '<' + tag + attrs + '>' + ch + '</' + tag + '>';
  }
  return '' + el;
}
export function renderToString(el) { return renderEl(el); }
"#,
    )
    .unwrap();
}

/// Create a temp project with mock node_modules and page files.
fn setup_live_test_project(pages: &[(&str, &str)]) -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup_mock_node_modules(tmp.path());

    let pages_dir = tmp.path().join("pages");
    fs::create_dir_all(&pages_dir).unwrap();
    for (path, content) in pages {
        let file_path = pages_dir.join(path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(file_path, content).unwrap();
    }

    tmp
}

/// Read an axum response body to a string.
async fn body_to_string(resp: axum::response::Response) -> String {
    use http_body_util::BodyExt;
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

// ─────────────────────── compile_project tests ───────────────────────────────

#[tokio::test]
async fn compile_project_builds_from_source() {
    let tmp = setup_live_test_project(&[(
        "index.tsx",
        "export default function Index() { return <div>Hello Live</div>; }",
    )]);
    let source = LocalSource::new(tmp.path().to_path_buf()).unwrap();
    let cache = BuildCache::new();

    let build = rex_live::compiler::compile_project(&source, &cache, 1)
        .await
        .unwrap();

    assert!(!build.server_bundle_js.is_empty());
    assert!(!build.scan.routes.is_empty());
    assert!(build.source_mtime > SystemTime::UNIX_EPOCH);
    assert!(cache.get().is_some());
}

#[tokio::test]
async fn compile_project_returns_cached_build() {
    let tmp = setup_live_test_project(&[(
        "index.tsx",
        "export default function Index() { return <div>Hello</div>; }",
    )]);
    let source = LocalSource::new(tmp.path().to_path_buf()).unwrap();
    let cache = BuildCache::new();

    let build1 = rex_live::compiler::compile_project(&source, &cache, 1)
        .await
        .unwrap();
    let build2 = rex_live::compiler::compile_project(&source, &cache, 1)
        .await
        .unwrap();

    // Same build_number means cache hit
    assert_eq!(build1.build_number, build2.build_number);
}

#[tokio::test]
async fn compile_project_fails_without_pages() {
    let tmp = TempDir::new().unwrap();
    setup_mock_node_modules(tmp.path());
    // No pages/ directory
    let source = LocalSource::new(tmp.path().to_path_buf()).unwrap();
    let cache = BuildCache::new();

    let result = rex_live::compiler::compile_project(&source, &cache, 1).await;
    assert!(result.is_err());
}

// ─────────────────────── ensure_built tests ──────────────────────────────────

#[tokio::test]
async fn ensure_built_initializes_pool_and_routes() {
    let tmp = setup_live_test_project(&[(
        "index.tsx",
        "export default function Index() { return <div>Hello</div>; }",
    )]);

    let project = rex_live::project::LiveProject::new(rex_live::project::LiveProjectConfig {
        prefix: "/".to_string(),
        source_path: tmp.path().to_path_buf(),
        workers: 1,
    })
    .unwrap();

    // Before build
    assert!(project.route_trie().is_none());
    assert!(project.manifest_json().is_none());

    let build = project.ensure_built().await.unwrap();

    // After build
    assert!(!build.server_bundle_js.is_empty());
    assert!(project.route_trie().is_some());
    assert!(project.manifest_json().is_some());
    assert!(project.api_route_trie().is_some());
}

#[tokio::test]
async fn ensure_built_returns_cached_on_second_call() {
    let tmp = setup_live_test_project(&[(
        "index.tsx",
        "export default function Index() { return <div>Hello</div>; }",
    )]);

    let project = rex_live::project::LiveProject::new(rex_live::project::LiveProjectConfig {
        prefix: "/".to_string(),
        source_path: tmp.path().to_path_buf(),
        workers: 1,
    })
    .unwrap();

    let build1 = project.ensure_built().await.unwrap();
    let build2 = project.ensure_built().await.unwrap();

    assert_eq!(build1.build_number, build2.build_number);
}

// ─────────────────── Handler + Router integration tests ──────────────────────

#[tokio::test]
async fn live_handler_renders_index_page() {
    use tower::ServiceExt;

    let tmp = setup_live_test_project(&[(
        "index.tsx",
        "export default function Index() { return <div>Hello Live</div>; }",
    )]);

    let server = rex_live::server::LiveServer::new(&rex_live::server::LiveServerConfig {
        mounts: vec![rex_live::server::MountConfig {
            prefix: "/".to_string(),
            source: tmp.path().to_path_buf(),
        }],
        port: 0,
        host: std::net::Ipv4Addr::LOCALHOST.into(),
        workers_per_project: 1,
    })
    .unwrap();

    let router = server.build_router();
    let req = axum::http::Request::builder()
        .uri("/")
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let body = body_to_string(resp).await;
    assert!(
        body.contains("Hello Live"),
        "Body should contain rendered content: {body}"
    );
    assert!(
        body.contains("</html>"),
        "Body should be a full HTML document"
    );
}

#[tokio::test]
async fn live_handler_404_no_project_match() {
    use tower::ServiceExt;

    let tmp = setup_live_test_project(&[(
        "index.tsx",
        "export default function Index() { return <div>Hello</div>; }",
    )]);

    let server = rex_live::server::LiveServer::new(&rex_live::server::LiveServerConfig {
        mounts: vec![rex_live::server::MountConfig {
            prefix: "/app".to_string(),
            source: tmp.path().to_path_buf(),
        }],
        port: 0,
        host: std::net::Ipv4Addr::LOCALHOST.into(),
        workers_per_project: 1,
    })
    .unwrap();

    let router = server.build_router();
    let req = axum::http::Request::builder()
        .uri("/other")
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    let body = body_to_string(resp).await;
    assert!(body.contains("no project mounted"));
}

#[tokio::test]
async fn live_handler_404_no_route_match() {
    use tower::ServiceExt;

    let tmp = setup_live_test_project(&[(
        "index.tsx",
        "export default function Index() { return <div>Hello</div>; }",
    )]);

    let server = rex_live::server::LiveServer::new(&rex_live::server::LiveServerConfig {
        mounts: vec![rex_live::server::MountConfig {
            prefix: "/".to_string(),
            source: tmp.path().to_path_buf(),
        }],
        port: 0,
        host: std::net::Ipv4Addr::LOCALHOST.into(),
        workers_per_project: 1,
    })
    .unwrap();

    let router = server.build_router();
    let req = axum::http::Request::builder()
        .uri("/nonexistent-page")
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn status_endpoint_returns_json() {
    use tower::ServiceExt;

    let tmp = setup_live_test_project(&[(
        "index.tsx",
        "export default function Index() { return <div>Hello</div>; }",
    )]);

    let server = rex_live::server::LiveServer::new(&rex_live::server::LiveServerConfig {
        mounts: vec![rex_live::server::MountConfig {
            prefix: "/".to_string(),
            source: tmp.path().to_path_buf(),
        }],
        port: 0,
        host: std::net::Ipv4Addr::LOCALHOST.into(),
        workers_per_project: 1,
    })
    .unwrap();

    let router = server.build_router();
    let req = axum::http::Request::builder()
        .uri("/_rex/live/status")
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let body = body_to_string(resp).await;
    assert!(
        body.contains("\"status\":\"ok\""),
        "Should contain status: {body}"
    );
    assert!(
        body.contains("\"mode\":\"live\""),
        "Should contain mode: {body}"
    );
}

#[tokio::test]
async fn live_handler_renders_multiple_pages() {
    use tower::ServiceExt;

    let tmp = setup_live_test_project(&[
        (
            "index.tsx",
            "export default function Home() { return <div>Home Page</div>; }",
        ),
        (
            "about.tsx",
            "export default function About() { return <div>About Page</div>; }",
        ),
    ]);

    let server = rex_live::server::LiveServer::new(&rex_live::server::LiveServerConfig {
        mounts: vec![rex_live::server::MountConfig {
            prefix: "/".to_string(),
            source: tmp.path().to_path_buf(),
        }],
        port: 0,
        host: std::net::Ipv4Addr::LOCALHOST.into(),
        workers_per_project: 1,
    })
    .unwrap();

    // Test home page (triggers compilation)
    let router = server.build_router();
    let req = axum::http::Request::builder()
        .uri("/")
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let body = body_to_string(resp).await;
    assert!(body.contains("Home Page"));

    // Test about page (reuses cached build)
    let router = server.build_router();
    let req = axum::http::Request::builder()
        .uri("/about")
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let body = body_to_string(resp).await;
    assert!(body.contains("About Page"));
}
