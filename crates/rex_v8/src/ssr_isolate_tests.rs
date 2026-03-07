use super::*;

/// Minimal React stub: provides React.createElement and ReactDOMServer.renderToString
/// without needing node_modules. Renders elements as simple HTML strings.
const MOCK_REACT_RUNTIME: &str = r#"
    globalThis.__React = {
        createElement: function(type, props) {
            var children = Array.prototype.slice.call(arguments, 2);
            return { type: type, props: props || {}, children: children };
        },
        Suspense: Symbol.for('react.suspense')
    };
    var React = globalThis.__React;

    function renderElement(el) {
        if (el === null || el === undefined) return '';
        if (typeof el === 'string') return el;
        if (typeof el === 'number') return String(el);
        if (Array.isArray(el)) return el.map(renderElement).join('');
        if (el.type === globalThis.__React.Suspense) {
            try {
                var inner = '';
                if (el.props && el.props.children) inner += renderElement(el.props.children);
                inner += el.children.map(renderElement).join('');
                return inner;
            } catch (e) {
                if (e && typeof e.then === 'function') {
                    return renderElement(el.props && el.props.fallback);
                }
                throw e;
            }
        }
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
                // Skip event handlers and undefined/null
                if (typeof v === 'function' || v === null || v === undefined) continue;
                // Convert React prop names to HTML attributes
                var attrName = k;
                if (k === 'className') attrName = 'class';
                else if (k === 'htmlFor') attrName = 'for';
                else if (k === 'fetchPriority') attrName = 'fetchpriority';
                // Serialize style objects to CSS strings
                if (k === 'style' && typeof v === 'object') {
                    var css = '';
                    for (var sk in v) {
                        if (v.hasOwnProperty(sk)) {
                            // Convert camelCase to kebab-case
                            var prop = sk.replace(/([A-Z])/g, '-$1').toLowerCase();
                            var sv = v[sk];
                            if (typeof sv === 'number' && sv !== 0 && prop !== 'opacity' && prop !== 'z-index' && prop !== 'font-weight' && prop !== 'line-height' && prop !== 'flex' && prop !== 'order') sv = sv + 'px';
                            css += prop + ':' + sv + ';';
                        }
                    }
                    attrs += ' style="' + css + '"';
                    continue;
                }
                // Boolean attributes
                if (v === true) { attrs += ' ' + attrName; continue; }
                if (v === false) continue;
                attrs += ' ' + attrName + '="' + String(v).replace(/&/g,'&amp;').replace(/"/g,'&quot;') + '"';
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
"#;

/// Page definition for test server bundle.
struct TestPage<'a> {
    key: &'a str,
    component: &'a str,
    gssp: Option<&'a str>,
    gsp: Option<&'a str>,
}

/// Build a minimal server bundle JS with given page definitions.
/// Each page entry: (route_key, component_js, gssp_js)
fn make_server_bundle(pages: &[(&str, &str, Option<&str>)]) -> String {
    let test_pages: Vec<TestPage> = pages
        .iter()
        .map(|(key, component, gssp)| TestPage {
            key,
            component,
            gssp: *gssp,
            gsp: None,
        })
        .collect();
    make_server_bundle_ext(&test_pages)
}

fn make_server_bundle_ext(pages: &[TestPage]) -> String {
    let mut bundle = String::new();
    bundle.push_str("'use strict';\n");
    bundle.push_str("globalThis.__rex_pages = globalThis.__rex_pages || {};\n\n");

    for page in pages {
        bundle.push_str(&format!(
            "globalThis.__rex_pages['{}'] = (function() {{\n  var exports = {{}};\n",
            page.key
        ));
        bundle.push_str(&format!("  exports.default = {};\n", page.component));
        if let Some(gssp_code) = page.gssp {
            bundle.push_str(&format!("  exports.getServerSideProps = {};\n", gssp_code));
        }
        if let Some(gsp_code) = page.gsp {
            bundle.push_str(&format!("  exports.getStaticProps = {};\n", gsp_code));
        }
        bundle.push_str("  return exports;\n})();\n\n");
    }

    // SSR runtime (same as bundler.rs produces)
    bundle.push_str(
        r#"
globalThis.__rex_head_elements = [];
globalThis.__rex_head_component = function Head(props) {
    if (props.children) {
        var children = Array.isArray(props.children) ? props.children : [props.children];
        for (var i = 0; i < children.length; i++) {
            if (children[i]) globalThis.__rex_head_elements.push(children[i]);
        }
    }
    return null;
};

globalThis.__rex_render_page = function(routeKey, propsJson) {
    var React = globalThis.__React;
    var ReactDOMServer = globalThis.__ReactDOMServer;
    if (!React || !ReactDOMServer) {
        throw new Error('React/ReactDOMServer not loaded');
    }
    var page = globalThis.__rex_pages[routeKey];
    if (!page) throw new Error('Page not found: ' + routeKey);
    var Component = page.default;
    if (!Component) throw new Error('No default export: ' + routeKey);
    var props = JSON.parse(propsJson);

    globalThis.__rex_head_elements = [];
    var element = React.createElement(Component, props);
    var bodyHtml = ReactDOMServer.renderToString(element);

    var headHtml = '';
    for (var i = 0; i < globalThis.__rex_head_elements.length; i++) {
        headHtml += ReactDOMServer.renderToString(globalThis.__rex_head_elements[i]);
    }

    return JSON.stringify({ body: bodyHtml, head: headHtml });
};

globalThis.__rex_gssp_resolved = null;
globalThis.__rex_gssp_rejected = null;

globalThis.__rex_get_server_side_props = function(routeKey, contextJson) {
    var page = globalThis.__rex_pages[routeKey];
    if (!page || !page.getServerSideProps) {
        return JSON.stringify({ props: {} });
    }
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
"#,
    );
    bundle
}

fn make_isolate(pages: &[(&str, &str, Option<&str>)]) -> SsrIsolate {
    crate::init_v8();
    let bundle = format!("{}\n{}", MOCK_REACT_RUNTIME, make_server_bundle(pages));
    SsrIsolate::new(&bundle, None).expect("failed to create isolate")
}

#[test]
fn test_render_simple_page() {
    let mut iso = make_isolate(&[(
        "index",
        "function Index() { return React.createElement('h1', null, 'Hello'); }",
        None,
    )]);
    let result = iso.render_page("index", "{}").unwrap();
    assert_eq!(result.body, "<h1>Hello</h1>");
    assert_eq!(result.head, "");
}

#[test]
fn test_render_with_props() {
    let mut iso = make_isolate(&[(
        "greet",
        "function Greet(props) { return React.createElement('p', null, 'Hi ' + props.name); }",
        None,
    )]);
    let result = iso.render_page("greet", r#"{"name":"Rex"}"#).unwrap();
    assert_eq!(result.body, "<p>Hi Rex</p>");
}

#[test]
fn test_render_nested_elements() {
    let mut iso = make_isolate(&[(
        "nested",
        r#"function Page() {
            return React.createElement('div', {class: 'wrapper'},
                React.createElement('h1', null, 'Title'),
                React.createElement('p', null, 'Body')
            );
        }"#,
        None,
    )]);
    let result = iso.render_page("nested", "{}").unwrap();
    assert_eq!(
        result.body,
        r#"<div class="wrapper"><h1>Title</h1><p>Body</p></div>"#
    );
}

#[test]
fn test_render_missing_page() {
    let mut iso = make_isolate(&[]);
    let err = iso.render_page("nonexistent", "{}").unwrap_err();
    assert!(
        err.to_string().contains("Page not found"),
        "expected 'Page not found', got: {err}"
    );
}

#[test]
fn test_render_component_throws() {
    let mut iso = make_isolate(&[(
        "bad",
        "function Bad() { throw new Error('component broke'); }",
        None,
    )]);
    let err = iso.render_page("bad", "{}").unwrap_err();
    assert!(
        err.to_string().contains("component broke"),
        "expected 'component broke', got: {err}"
    );
}

#[test]
fn test_gssp_sync() {
    let mut iso = make_isolate(&[(
        "page",
        "function Page(props) { return React.createElement('span', null, props.title); }",
        Some("function(ctx) { return { props: { title: 'from gssp' } }; }"),
    )]);
    let json = iso
        .get_server_side_props("page", r#"{"params":{},"query":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(val["props"]["title"], "from gssp");
}

#[test]
fn test_gssp_no_gssp_returns_empty_props() {
    let mut iso = make_isolate(&[(
        "page",
        "function Page() { return React.createElement('div', null, 'hi'); }",
        None,
    )]);
    let json = iso
        .get_server_side_props("page", r#"{"params":{},"query":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(val["props"], serde_json::json!({}));
}

#[test]
fn test_gssp_receives_context() {
    let mut iso = make_isolate(&[(
        "page",
        "function Page() { return React.createElement('div'); }",
        Some(
            "function(ctx) { return { props: { slug: ctx.params.slug, url: ctx.resolved_url } }; }",
        ),
    )]);
    let context = r#"{"params":{"slug":"hello"},"query":{},"resolved_url":"/blog/hello","headers":{},"cookies":{}}"#;
    let json = iso.get_server_side_props("page", context).unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(val["props"]["slug"], "hello");
    assert_eq!(val["props"]["url"], "/blog/hello");
}

#[test]
fn test_gssp_async() {
    let mut iso = make_isolate(&[(
        "page",
        "function Page() { return React.createElement('div'); }",
        Some("function(ctx) { return Promise.resolve({ props: { async: true } }); }"),
    )]);
    let json = iso
        .get_server_side_props("page", r#"{"params":{},"query":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(val["props"]["async"], true);
}

#[test]
fn test_gssp_throws() {
    let mut iso = make_isolate(&[(
        "page",
        "function Page() { return React.createElement('div'); }",
        Some("function(ctx) { throw new Error('gssp failed'); }"),
    )]);
    let err = iso
        .get_server_side_props("page", r#"{"params":{},"query":{}}"#)
        .unwrap_err();
    assert!(
        err.to_string().contains("gssp failed"),
        "expected 'gssp failed', got: {err}"
    );
}

#[test]
fn test_gssp_missing_page() {
    let mut iso = make_isolate(&[]);
    let json = iso
        .get_server_side_props("nonexistent", r#"{"params":{},"query":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(val["props"], serde_json::json!({}));
}

#[test]
fn test_reload_replaces_pages() {
    let mut iso = make_isolate(&[(
        "page",
        "function Page() { return React.createElement('p', null, 'v1'); }",
        None,
    )]);
    assert_eq!(iso.render_page("page", "{}").unwrap().body, "<p>v1</p>");

    let new_bundle = make_server_bundle(&[(
        "page",
        "function Page() { return React.createElement('p', null, 'v2'); }",
        None,
    )]);
    iso.reload(&new_bundle).unwrap();
    assert_eq!(iso.render_page("page", "{}").unwrap().body, "<p>v2</p>");
}

#[test]
fn test_reload_adds_new_pages() {
    let mut iso = make_isolate(&[(
        "page",
        "function Page() { return React.createElement('p', null, 'original'); }",
        None,
    )]);

    let new_bundle = make_server_bundle(&[
        (
            "page",
            "function Page() { return React.createElement('p', null, 'original'); }",
            None,
        ),
        (
            "about",
            "function About() { return React.createElement('h1', null, 'About'); }",
            None,
        ),
    ]);
    iso.reload(&new_bundle).unwrap();
    assert_eq!(
        iso.render_page("about", "{}").unwrap().body,
        "<h1>About</h1>"
    );
}

#[test]
fn test_invalid_server_bundle() {
    crate::init_v8();
    let result = SsrIsolate::new("this is not valid javascript {{{{", None);
    assert!(result.is_err());
}

#[test]
fn test_multiple_renders_same_isolate() {
    let mut iso = make_isolate(&[(
        "page",
        "function Page(props) { return React.createElement('b', null, props.n); }",
        None,
    )]);
    for i in 0..5 {
        let result = iso.render_page("page", &format!(r#"{{"n":{i}}}"#)).unwrap();
        assert_eq!(result.body, format!("<b>{i}</b>"));
    }
}

#[test]
fn test_render_with_head_elements() {
    let mut iso = make_isolate(&[(
        "seo",
        r#"function SeoPage(props) {
            var Head = globalThis.__rex_head_component;
            return React.createElement('div', null,
                React.createElement(Head, null,
                    React.createElement('title', null, props.title),
                    React.createElement('meta', { name: 'description', content: 'A test page' })
                ),
                React.createElement('h1', null, props.title)
            );
        }"#,
        None,
    )]);
    let result = iso.render_page("seo", r#"{"title":"My Page"}"#).unwrap();
    assert!(
        result.body.contains("<h1>My Page</h1>"),
        "body should have h1: {}",
        result.body
    );
    assert!(
        !result.body.contains("<title>"),
        "body should NOT contain title: {}",
        result.body
    );
    assert!(
        result.head.contains("<title>My Page</title>"),
        "head should contain title: {}",
        result.head
    );
    assert!(
        result.head.contains("description"),
        "head should contain meta description: {}",
        result.head
    );
}

fn make_isolate_ext(pages: &[TestPage]) -> SsrIsolate {
    crate::init_v8();
    let bundle = format!("{}\n{}", MOCK_REACT_RUNTIME, make_server_bundle_ext(pages));
    SsrIsolate::new(&bundle, None).expect("failed to create isolate")
}

#[test]
fn test_gsp_sync() {
    let mut iso = make_isolate_ext(&[TestPage {
        key: "page",
        component:
            "function Page(props) { return React.createElement('span', null, props.title); }",
        gssp: None,
        gsp: Some("function(ctx) { return { props: { title: 'from gsp' } }; }"),
    }]);
    let json = iso.get_static_props("page", r#"{"params":{}}"#).unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(val["props"]["title"], "from gsp");
}

#[test]
fn test_gsp_async() {
    let mut iso = make_isolate_ext(&[TestPage {
        key: "page",
        component: "function Page() { return React.createElement('div'); }",
        gssp: None,
        gsp: Some("function(ctx) { return Promise.resolve({ props: { async: true } }); }"),
    }]);
    let json = iso.get_static_props("page", r#"{"params":{}}"#).unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(val["props"]["async"], true);
}

#[test]
fn test_render_suspense_renders_children() {
    let mut iso = make_isolate(&[(
        "page",
        r#"function Page() {
            return React.createElement(React.Suspense, { fallback: 'Loading' },
                React.createElement('div', null, 'Suspense child content')
            );
        }"#,
        None,
    )]);
    let result = iso.render_page("page", "{}").unwrap();
    assert!(
        result.body.contains("Suspense child content"),
        "should render children, not fallback: {}",
        result.body
    );
    assert!(
        !result.body.contains("Loading"),
        "should NOT render fallback when children render normally: {}",
        result.body
    );
}

#[test]
fn test_render_suspense_fallback_on_throw() {
    let mut iso = make_isolate(&[(
        "page",
        r#"function Page() {
            function Thrower() { throw Promise.resolve(); }
            return React.createElement(React.Suspense, { fallback: 'Loading...' },
                React.createElement(Thrower)
            );
        }"#,
        None,
    )]);
    let result = iso.render_page("page", "{}").unwrap();
    assert!(
        result.body.contains("Loading..."),
        "should render fallback when child throws a promise: {}",
        result.body
    );
}

#[test]
fn test_head_reset_between_renders() {
    let mut iso = make_isolate(&[
        (
            "page1",
            r#"function Page1() {
                var Head = globalThis.__rex_head_component;
                return React.createElement('div', null,
                    React.createElement(Head, null, React.createElement('title', null, 'Page 1'))
                );
            }"#,
            None,
        ),
        (
            "page2",
            r#"function Page2() {
                return React.createElement('div', null, 'No head');
            }"#,
            None,
        ),
    ]);
    let r1 = iso.render_page("page1", "{}").unwrap();
    assert!(
        r1.head.contains("<title>Page 1</title>"),
        "page1 should have title"
    );

    let r2 = iso.render_page("page2", "{}").unwrap();
    assert_eq!(
        r2.head, "",
        "page2 should have empty head (no leak from page1)"
    );
}

#[test]
fn test_reload_updates_pages() {
    // Start with a page that renders "v1"
    let mut iso = make_isolate(&[(
        "index",
        "function Index() { return React.createElement('p', null, 'v1'); }",
        None,
    )]);
    let r1 = iso.render_page("index", "{}").unwrap();
    assert_eq!(r1.body, "<p>v1</p>");

    // Reload with a new bundle that renders "v2"
    let new_bundle = format!(
        "{}\n{}",
        MOCK_REACT_RUNTIME,
        make_server_bundle(&[(
            "index",
            "function Index() { return React.createElement('p', null, 'v2'); }",
            None,
        )])
    );
    iso.reload(&new_bundle).unwrap();
    let r2 = iso.render_page("index", "{}").unwrap();
    assert_eq!(r2.body, "<p>v2</p>");
}

#[test]
fn test_reload_bad_bundle_restores_previous() {
    let mut iso = make_isolate(&[(
        "index",
        "function Index() { return React.createElement('p', null, 'ok'); }",
        None,
    )]);
    let r1 = iso.render_page("index", "{}").unwrap();
    assert_eq!(r1.body, "<p>ok</p>");

    // Reload with syntactically invalid JS — should fail and restore previous
    let result = iso.reload("this is not valid javascript {{{{");
    assert!(result.is_err(), "reload should fail for bad JS");

    // Previous bundle should still work
    let r2 = iso.render_page("index", "{}").unwrap();
    assert_eq!(r2.body, "<p>ok</p>");
}

#[path = "ssr_isolate_tests_ext.rs"]
mod ext;

#[path = "ssr_isolate_tests_crypto.rs"]
mod crypto;
