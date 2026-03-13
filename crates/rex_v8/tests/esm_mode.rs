//! Tests for SsrIsolate in ESM mode (ssr_isolate_esm.rs).
//!
//! Uses inline JS to simulate the dep IIFE and user modules without needing
//! rolldown or node_modules. User sources set globals directly since they are
//! wrapped in IIFEs by compile_and_evaluate_esm.
#![allow(clippy::unwrap_used)]

mod common;

use rex_v8::EsmModuleRegistry;
use rex_v8::SsrIsolate;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Minimal dep IIFE: mock React + renderToString on globalThis.
const MOCK_DEP_IIFE: &str = r#"
globalThis.__React = {
    createElement: function(type, props) {
        var children = Array.prototype.slice.call(arguments, 2);
        return { type: type, props: props || {}, children: children };
    },
    Suspense: Symbol.for('react.suspense')
};
var React = globalThis.__React;
globalThis.__rex_React = React;
globalThis.__rex_createElement = React.createElement;

function renderElement(el) {
    if (el === null || el === undefined) return '';
    if (typeof el === 'string') return el;
    if (typeof el === 'number') return String(el);
    if (Array.isArray(el)) return el.map(renderElement).join('');
    if (typeof el.type === 'function') {
        var merged = Object.assign({}, el.props);
        if (el.children.length > 0) merged.children = el.children.length === 1 ? el.children[0] : el.children;
        return renderElement(el.type(merged));
    }
    if (typeof el.type === 'string') {
        var attrs = '';
        var p = el.props || {};
        for (var k in p) {
            if (k === 'children' || k === 'dangerouslySetInnerHTML') continue;
            if (!p.hasOwnProperty(k)) continue;
            var v = p[k];
            if (typeof v === 'function' || v === null || v === undefined) continue;
            var attrName = k === 'className' ? 'class' : k;
            if (v === true) { attrs += ' ' + attrName; continue; }
            if (v === false) continue;
            attrs += ' ' + attrName + '="' + String(v) + '"';
        }
        var inner = '';
        if (p.children) inner += renderElement(p.children);
        inner += el.children.map(renderElement).join('');
        if (!inner) return '<' + el.type + attrs + '/>';
        return '<' + el.type + attrs + '>' + inner + '</' + el.type + '>';
    }
    return '';
}

globalThis.__ReactDOMServer = {
    renderToString: function(el) { return renderElement(el); }
};
globalThis.__rex_renderToString = globalThis.__ReactDOMServer.renderToString;
"#;

/// SSR runtime: page registry + render/gssp/gsp functions.
/// In ESM mode, the entry source sets up __rex_pages via `import` statements,
/// but those imports fail when evaluated as scripts. So our user source files
/// register pages directly on globalThis.__rex_pages. The SSR runtime defines
/// the functions that the isolate extracts as global handles.
const MOCK_SSR_RUNTIME: &str = r#"
globalThis.__rex_pages = globalThis.__rex_pages || {};

globalThis.__rex_head_elements = [];
globalThis.__rex_head_component = function Head(props) { return null; };

globalThis.__rex_render_page = function(routeKey, propsJson) {
    var React = globalThis.__React;
    var ReactDOMServer = globalThis.__ReactDOMServer;
    if (!React || !ReactDOMServer) throw new Error('React/ReactDOMServer not loaded');
    var page = globalThis.__rex_pages[routeKey];
    if (!page) throw new Error('Page not found: ' + routeKey);
    var Component = page.default;
    if (!Component) throw new Error('No default export: ' + routeKey);
    var props = JSON.parse(propsJson);
    globalThis.__rex_head_elements = [];
    var element = React.createElement(Component, props);
    var bodyHtml = ReactDOMServer.renderToString(element);
    var headHtml = '';
    return JSON.stringify({ body: bodyHtml, head: headHtml });
};

globalThis.__rex_gssp_resolved = null;
globalThis.__rex_gssp_rejected = null;

globalThis.__rex_get_server_side_props = function(routeKey, contextJson) {
    var page = globalThis.__rex_pages[routeKey];
    if (!page || !page.getServerSideProps) return JSON.stringify({ props: {} });
    var context = JSON.parse(contextJson);
    var result = page.getServerSideProps(context);
    if (result && typeof result.then === 'function') {
        globalThis.__rex_gssp_resolved = null;
        globalThis.__rex_gssp_rejected = null;
        result.then(
            function(v) { globalThis.__rex_gssp_resolved = v; },
            function(e) { globalThis.__rex_gssp_rejected = e; }
        );
        return '__REX_ASYNC__';
    }
    return JSON.stringify(result);
};

globalThis.__rex_resolve_gssp = function() {
    if (globalThis.__rex_gssp_rejected) throw globalThis.__rex_gssp_rejected;
    if (globalThis.__rex_gssp_resolved !== null) return JSON.stringify(globalThis.__rex_gssp_resolved);
    throw new Error('GSSP promise did not resolve');
};

globalThis.__rex_gsp_resolved = null;
globalThis.__rex_gsp_rejected = null;

globalThis.__rex_get_static_props = function(routeKey, contextJson) {
    var page = globalThis.__rex_pages[routeKey];
    if (!page || !page.getStaticProps) return JSON.stringify({ props: {} });
    var context = JSON.parse(contextJson);
    var result = page.getStaticProps(context);
    if (result && typeof result.then === 'function') {
        globalThis.__rex_gsp_resolved = null;
        globalThis.__rex_gsp_rejected = null;
        result.then(
            function(v) { globalThis.__rex_gsp_resolved = v; },
            function(e) { globalThis.__rex_gsp_rejected = e; }
        );
        return '__REX_GSP_ASYNC__';
    }
    return JSON.stringify(result);
};

globalThis.__rex_resolve_gsp = function() {
    if (globalThis.__rex_gsp_rejected) throw globalThis.__rex_gsp_rejected;
    if (globalThis.__rex_gsp_resolved !== null) return JSON.stringify(globalThis.__rex_gsp_resolved);
    throw new Error('GSP promise did not resolve');
};
"#;

/// Build a user source that stashes page exports on a per-module global.
/// The source is wrapped in an IIFE by compile_and_evaluate_esm (runs before the entry).
/// The SSR runtime (appended to entry source) copies these into __rex_pages.
fn make_page_source(module_name: &str, component_body: &str) -> String {
    format!(
        r#"var React = globalThis.__React;
globalThis.__rex_esm_export_{module_name} = (function() {{
    var exports = {{}};
    exports.default = {component_body};
    return exports;
}})();"#
    )
}

/// Build the page-registration preamble for the SSR runtime.
/// This runs as part of the entry source, AFTER `build_entry_source()` clears
/// `__rex_pages = {}`, so it can safely copy the per-module globals into the registry.
fn make_page_registration(names: &[&str]) -> String {
    let mut js = String::new();
    for name in names {
        js.push_str(&format!(
            "globalThis.__rex_pages['{name}'] = globalThis.__rex_esm_export_{name};\n"
        ));
    }
    js
}

fn make_esm_isolate(pages: &[(&str, &str)]) -> SsrIsolate {
    rex_v8::init_v8();

    let mut sources: HashMap<PathBuf, String> = HashMap::new();
    let names: Vec<&str> = pages.iter().map(|(n, _)| *n).collect();

    for (name, component) in pages {
        let path = PathBuf::from(format!("/project/pages/{name}.tsx"));
        let source = make_page_source(name, component);
        sources.insert(path.clone(), source);
    }

    let registry = EsmModuleRegistry::new(
        Arc::new(MOCK_DEP_IIFE.to_string()),
        sources,
        HashMap::new(),
        PathBuf::from("/project"),
    );

    // Pass empty page_sources so build_entry_source generates no import statements.
    // The page registration preamble (prepended to SSR runtime) copies page exports
    // into __rex_pages after the entry source clears it.
    let ssr_with_registration = format!("{}{}", make_page_registration(&names), MOCK_SSR_RUNTIME);

    SsrIsolate::new_esm(registry, &ssr_with_registration, &[], None)
        .expect("failed to create ESM isolate")
}

#[test]
fn test_esm_mode_creates_isolate() {
    let _iso = make_esm_isolate(&[(
        "index",
        "function Index() { return React.createElement('h1', null, 'Hello ESM'); }",
    )]);
    // If we get here without panicking, the isolate was created successfully
}

#[test]
fn test_esm_mode_renders_page() {
    let mut iso = make_esm_isolate(&[(
        "index",
        "function Index() { return React.createElement('h1', null, 'Hello ESM'); }",
    )]);
    let result = iso.render_page("index", "{}").unwrap();
    assert_eq!(result.body, "<h1>Hello ESM</h1>");
}

#[test]
fn test_esm_mode_renders_multiple_pages() {
    let mut iso = make_esm_isolate(&[
        (
            "index",
            "function Index() { return React.createElement('h1', null, 'Home'); }",
        ),
        (
            "about",
            "function About() { return React.createElement('p', null, 'About page'); }",
        ),
    ]);
    assert_eq!(
        iso.render_page("index", "{}").unwrap().body,
        "<h1>Home</h1>"
    );
    assert_eq!(
        iso.render_page("about", "{}").unwrap().body,
        "<p>About page</p>"
    );
}

#[test]
fn test_esm_mode_invalidate_module() {
    rex_v8::init_v8();

    let index_path = PathBuf::from("/project/pages/index.tsx");
    let mut sources: HashMap<PathBuf, String> = HashMap::new();
    sources.insert(
        index_path.clone(),
        make_page_source(
            "index",
            "function Index() { return React.createElement('p', null, 'v1'); }",
        ),
    );

    let registry = EsmModuleRegistry::new(
        Arc::new(MOCK_DEP_IIFE.to_string()),
        sources,
        HashMap::new(),
        PathBuf::from("/project"),
    );

    let ssr_with_registration =
        format!("{}{}", make_page_registration(&["index"]), MOCK_SSR_RUNTIME);

    let mut iso = SsrIsolate::new_esm(registry, &ssr_with_registration, &[], None).unwrap();

    // Verify initial render
    assert_eq!(iso.render_page("index", "{}").unwrap().body, "<p>v1</p>");

    // Invalidate with updated source (pass empty page_sources to avoid import statements)
    let new_source = make_page_source(
        "index",
        "function Index() { return React.createElement('p', null, 'v2'); }",
    );
    iso.invalidate_module(index_path, new_source, &[]).unwrap();

    // Verify updated render
    assert_eq!(iso.render_page("index", "{}").unwrap().body, "<p>v2</p>");
}

#[test]
fn test_esm_mode_reload_fails_in_esm() {
    let mut iso = make_esm_isolate(&[(
        "index",
        "function Index() { return React.createElement('div', null, 'hi'); }",
    )]);
    let err = iso.reload("// any bundle").unwrap_err();
    assert!(
        err.to_string().contains("ESM mode"),
        "reload should fail in ESM mode, got: {err}"
    );
}

#[test]
fn test_esm_mode_invalidate_fails_in_bundled() {
    rex_v8::init_v8();

    let bundle = format!(
        "{}\n{}",
        common::MOCK_REACT_RUNTIME,
        common::make_server_bundle(&[(
            "index",
            "function Index() { return React.createElement('div', null, 'hi'); }",
            None,
        )])
    );
    let mut iso = SsrIsolate::new(&bundle, None).unwrap();

    let err = iso
        .invalidate_module(
            PathBuf::from("/project/pages/index.tsx"),
            "var x = 1;".to_string(),
            &[],
        )
        .unwrap_err();
    assert!(
        err.to_string().contains("Bundled mode"),
        "invalidate should fail in Bundled mode, got: {err}"
    );
}
