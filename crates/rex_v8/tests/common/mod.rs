#![allow(clippy::unwrap_used, dead_code)]

use rex_v8::SsrIsolate;

/// Minimal React stub: provides React.createElement and ReactDOMServer.renderToString
/// without needing node_modules. Renders elements as simple HTML strings.
pub const MOCK_REACT_RUNTIME: &str = r#"
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
pub struct TestPage<'a> {
    pub key: &'a str,
    pub component: &'a str,
    pub gssp: Option<&'a str>,
    pub gsp: Option<&'a str>,
}

/// Build a minimal server bundle JS with given page definitions.
pub fn make_server_bundle(pages: &[(&str, &str, Option<&str>)]) -> String {
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

pub fn make_server_bundle_ext(pages: &[TestPage]) -> String {
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

pub fn make_isolate(pages: &[(&str, &str, Option<&str>)]) -> SsrIsolate {
    rex_v8::init_v8();
    let bundle = format!("{}\n{}", MOCK_REACT_RUNTIME, make_server_bundle(pages));
    SsrIsolate::new(&bundle, None).expect("failed to create isolate")
}

pub fn make_isolate_ext(pages: &[TestPage]) -> SsrIsolate {
    rex_v8::init_v8();
    let bundle = format!("{}\n{}", MOCK_REACT_RUNTIME, make_server_bundle_ext(pages));
    SsrIsolate::new(&bundle, None).expect("failed to create isolate")
}
