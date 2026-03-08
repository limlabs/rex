#![allow(clippy::unwrap_used)]
#![allow(dead_code)]

use rex_build::build_bundles;
use rex_build::bundler::BuildResult;
use rex_core::{PageType, ProjectConfig, RexConfig, Route};
use rex_router::ScanResult;
use std::fs;
use std::path::PathBuf;

pub fn setup_test_project(
    pages: &[(&str, &str)],
    app_source: Option<&str>,
) -> (tempfile::TempDir, RexConfig, ScanResult) {
    setup_test_project_full(pages, app_source, None)
}

/// Create a temp project directory with page files, returning (config, scan)
pub fn setup_test_project_full(
    pages: &[(&str, &str)],
    app_source: Option<&str>,
    doc_source: Option<&str>,
) -> (tempfile::TempDir, RexConfig, ScanResult) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();

    // Create mock node_modules so rolldown can resolve React imports
    setup_mock_node_modules(&root);

    // Create pages directory
    let pages_dir = root.join("pages");
    fs::create_dir_all(&pages_dir).unwrap();

    let mut routes = Vec::new();
    for (rel_path, source) in pages {
        let abs = pages_dir.join(rel_path);
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&abs, source).unwrap();

        let file_path = PathBuf::from(rel_path);
        let module_name = file_path
            .with_extension("")
            .to_string_lossy()
            .replace('\\', "/");
        let pattern = if module_name == "index" {
            "/".to_string()
        } else {
            format!("/{}", module_name.replace("[slug]", ":slug"))
        };

        routes.push(Route {
            pattern,
            file_path,
            abs_path: abs,
            dynamic_segments: vec![],
            page_type: PageType::Regular,
            specificity: 10,
        });
    }

    let app = app_source.map(|src| {
        let abs = pages_dir.join("_app.tsx");
        fs::write(&abs, src).unwrap();
        Route {
            pattern: String::new(),
            file_path: PathBuf::from("_app.tsx"),
            abs_path: abs,
            dynamic_segments: vec![],
            page_type: PageType::App,
            specificity: 0,
        }
    });

    let document = doc_source.map(|src| {
        let abs = pages_dir.join("_document.tsx");
        fs::write(&abs, src).unwrap();
        Route {
            pattern: String::new(),
            file_path: PathBuf::from("_document.tsx"),
            abs_path: abs,
            dynamic_segments: vec![],
            page_type: PageType::Document,
            specificity: 0,
        }
    });

    let config = RexConfig::new(root).with_dev(true);
    let scan = ScanResult {
        routes,
        api_routes: vec![],
        app,
        document,
        error: None,
        not_found: None,
        middleware: None,
        app_scan: None,
        mcp_tools: vec![],
    };

    (tmp, config, scan)
}

/// Create mock node_modules with minimal React stubs so rolldown can resolve imports.
pub fn setup_mock_node_modules(root: &std::path::Path) {
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

/// Build a project and load the server bundle into V8 for SSR testing.
pub async fn build_and_load(
    config: &RexConfig,
    scan: &ScanResult,
) -> (BuildResult, rex_v8::IsolatePool) {
    let result = build_bundles(config, scan, &ProjectConfig::default())
        .await
        .expect("build failed");
    let bundle = fs::read_to_string(&result.server_bundle_path).expect("read bundle");
    rex_v8::init_v8();
    let pool = rex_v8::IsolatePool::new(1, std::sync::Arc::new(bundle), None).expect("create pool");
    (result, pool)
}

/// Extend mock node_modules with react-server-dom-webpack stubs for RSC tests.
pub fn setup_rsc_mock_node_modules(root: &std::path::Path) {
    // Start with standard mocks
    setup_mock_node_modules(root);

    let nm = root.join("node_modules");

    // react-server-dom-webpack
    let rsdw_dir = nm.join("react-server-dom-webpack");
    fs::create_dir_all(&rsdw_dir).unwrap();
    fs::write(
        rsdw_dir.join("package.json"),
        r#"{"name":"react-server-dom-webpack","version":"19.0.0","main":"index.js","exports":{".":{"default":"./index.js"},"./client":{"default":"./client.js"},"./server":{"default":"./server.js"}}}"#,
    )
    .unwrap();
    fs::write(rsdw_dir.join("index.js"), "export default {};\n").unwrap();
    fs::write(
        rsdw_dir.join("client.js"),
        "export function createFromReadableStream(s) { return {}; }\nexport function createServerReference(id, callServer) { return function(...args) { return callServer(id, args); }; }\n",
    )
    .unwrap();
    fs::write(
        rsdw_dir.join("server.js"),
        "export function renderToReadableStream(el, config) { return new ReadableStream(); }\nexport function registerServerReference(fn, id, name) { return fn; }\n",
    )
    .unwrap();
}

/// Build and load with fs polyfill enabled (project_root passed to isolate pool).
pub async fn build_and_load_with_root(
    config: &RexConfig,
    scan: &ScanResult,
) -> (BuildResult, rex_v8::IsolatePool) {
    let result = build_bundles(config, scan, &ProjectConfig::default())
        .await
        .expect("build failed");
    let bundle = fs::read_to_string(&result.server_bundle_path).expect("read bundle");
    rex_v8::init_v8();
    let root_str = config.project_root.to_string_lossy().to_string();
    let pool = rex_v8::IsolatePool::new(
        1,
        std::sync::Arc::new(bundle),
        Some(std::sync::Arc::new(root_str)),
    )
    .expect("create pool");
    (result, pool)
}
