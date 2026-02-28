use anyhow::{Context, Result};
use serde::Deserialize;
use tracing::debug;

/// Result of SSR page rendering, containing both body HTML and head elements.
#[derive(Debug, Clone, Deserialize)]
pub struct RenderResult {
    pub body: String,
    #[serde(default)]
    pub head: String,
}

/// An SSR isolate that owns a V8 isolate and can render pages.
/// Must be used on the same OS thread that created it (V8 isolates are !Send).
pub struct SsrIsolate {
    isolate: v8::OwnedIsolate,
    context: v8::Global<v8::Context>,
    render_fn: v8::Global<v8::Function>,
    gssp_fn: v8::Global<v8::Function>,
    gsp_fn: v8::Global<v8::Function>,
    api_handler_fn: Option<v8::Global<v8::Function>>,
    document_fn: Option<v8::Global<v8::Function>>,
}

/// Evaluate a script in the given scope, using TryCatch for error handling.
/// The scope must already be a ContextScope. Returns the result value.
macro_rules! v8_eval {
    ($scope:expr, $code:expr, $filename:expr) => {{
        // Create a TryCatch scope
        v8::tc_scope!(tc, $scope);

        let source = v8::String::new(tc, $code)
            .ok_or_else(|| anyhow::anyhow!("Failed to create V8 string"))?;
        let name = v8::String::new(tc, $filename)
            .ok_or_else(|| anyhow::anyhow!("Failed to create V8 filename string"))?;
        let origin = v8::ScriptOrigin::new(
            tc,
            name.into(),
            0,
            0,
            false,
            0,
            None,
            false,
            false,
            false,
            None,
        );

        match v8::Script::compile(tc, source, Some(&origin)) {
            Some(script) => match script.run(tc) {
                Some(val) => Ok::<v8::Local<v8::Value>, anyhow::Error>(val),
                None => {
                    let msg = tc
                        .exception()
                        .map(|e| e.to_rust_string_lossy(tc))
                        .unwrap_or_else(|| "Unknown error".into());
                    Err(anyhow::anyhow!("V8 error in {}: {}", $filename, msg))
                }
            },
            None => {
                let msg = tc
                    .exception()
                    .map(|e| e.to_rust_string_lossy(tc))
                    .unwrap_or_else(|| "Unknown compile error".into());
                Err(anyhow::anyhow!(
                    "V8 compile error in {}: {}",
                    $filename,
                    msg
                ))
            }
        }
    }};
}

/// Call a V8 function with args, using TryCatch for error handling.
macro_rules! v8_call {
    ($scope:expr, $func:expr, $recv:expr, $args:expr) => {{
        v8::tc_scope!(tc, $scope);

        match $func.call(tc, $recv, $args) {
            Some(val) => Ok::<v8::Local<v8::Value>, anyhow::Error>(val),
            None => {
                let msg = tc
                    .exception()
                    .map(|e| e.to_rust_string_lossy(tc))
                    .unwrap_or_else(|| "Unknown call error".into());
                Err(anyhow::anyhow!("{}", msg))
            }
        }
    }};
}

/// Look up a required global function by name.
macro_rules! v8_get_global_fn {
    ($scope:expr, $global:expr, $name:expr) => {{
        let k = v8::String::new($scope, $name)
            .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed for '{}'", $name))?;
        let v = $global
            .get($scope, k.into())
            .ok_or_else(|| anyhow::anyhow!("{} not found", $name))?;
        v8::Local::<v8::Function>::try_from(v)
            .map_err(|_| anyhow::anyhow!("{} is not a function", $name))
    }};
}

/// Look up an optional global function by name.
macro_rules! v8_get_optional_fn {
    ($scope:expr, $global:expr, $name:expr) => {{
        v8::String::new($scope, $name)
            .and_then(|k| $global.get($scope, k.into()))
            .and_then(|v| v8::Local::<v8::Function>::try_from(v).ok())
            .map(|f| v8::Global::new($scope, f))
    }};
}

impl SsrIsolate {
    /// Create a new SSR isolate and evaluate the server bundle.
    ///
    /// The bundle is self-contained: it includes V8 polyfills, React,
    /// all pages, and SSR runtime functions in a single IIFE.
    pub fn new(server_bundle_js: &str) -> Result<Self> {
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());

        let (context, render_fn, gssp_fn, gsp_fn, api_handler_fn, document_fn) = {
            v8::scope!(scope, &mut isolate);

            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);

            // Install console + globalThis
            {
                let global = context.global(scope);

                let console = v8::Object::new(scope);

                let t = v8::FunctionTemplate::new(scope, console_log);
                let f = t.get_function(scope).ok_or_else(|| anyhow::anyhow!("Failed to create console.log"))?;
                let k = v8::String::new(scope, "log").ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
                console.set(scope, k.into(), f.into());

                let t = v8::FunctionTemplate::new(scope, console_warn);
                let f = t.get_function(scope).ok_or_else(|| anyhow::anyhow!("Failed to create console.warn"))?;
                let k = v8::String::new(scope, "warn").ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
                console.set(scope, k.into(), f.into());

                let t = v8::FunctionTemplate::new(scope, console_error);
                let f = t.get_function(scope).ok_or_else(|| anyhow::anyhow!("Failed to create console.error"))?;
                let k = v8::String::new(scope, "error").ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
                console.set(scope, k.into(), f.into());

                let t = v8::FunctionTemplate::new(scope, console_log);
                let f = t.get_function(scope).ok_or_else(|| anyhow::anyhow!("Failed to create console.info"))?;
                let k = v8::String::new(scope, "info").ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
                console.set(scope, k.into(), f.into());

                let k = v8::String::new(scope, "console").ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
                global.set(scope, k.into(), console.into());

                let k = v8::String::new(scope, "globalThis").ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
                global.set(scope, k.into(), global.into());
            }

            // Evaluate the self-contained server bundle
            v8_eval!(scope, server_bundle_js, "server-bundle.js")
                .context("Failed to evaluate server bundle")?;

            // Get global functions
            let ctx = scope.get_current_context();
            let global = ctx.global(scope);

            let render_fn = v8_get_global_fn!(scope, global, "__rex_render_page")?;
            let gssp_fn = v8_get_global_fn!(scope, global, "__rex_get_server_side_props")?;
            let gsp_fn = v8_get_global_fn!(scope, global, "__rex_get_static_props")?;

            // API handler is optional — only present when api/ routes exist
            let api_handler_fn = v8_get_optional_fn!(scope, global, "__rex_call_api_handler");

            // Document renderer is optional — only present when _document exists
            let document_fn = v8_get_optional_fn!(scope, global, "__rex_render_document");

            (
                v8::Global::new(scope, context),
                v8::Global::new(scope, render_fn),
                v8::Global::new(scope, gssp_fn),
                v8::Global::new(scope, gsp_fn),
                api_handler_fn,
                document_fn,
            )
        };

        Ok(Self {
            isolate,
            context,
            render_fn,
            gssp_fn,
            gsp_fn,
            api_handler_fn,
            document_fn,
        })
    }

    /// Call __rex_render_page(routeKey, propsJson) and return the rendered body + head HTML.
    pub fn render_page(&mut self, route_key: &str, props_json: &str) -> Result<RenderResult> {
        v8::scope_with_context!(scope, &mut self.isolate, &self.context);

        let func = v8::Local::new(scope, &self.render_fn);
        let undef = v8::undefined(scope);
        let arg0 = v8::String::new(scope, route_key)
            .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
        let arg1 = v8::String::new(scope, props_json)
            .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;

        let result = v8_call!(scope, func, undef.into(), &[arg0.into(), arg1.into()])
            .map_err(|e| anyhow::anyhow!("SSR render error: {e}"))?;

        let json_str = result.to_rust_string_lossy(scope);
        serde_json::from_str(&json_str).context("Failed to parse render result JSON")
    }

    /// Call __rex_get_server_side_props(routeKey, contextJson) and return JSON.
    /// Handles async GSSP functions by pumping V8's microtask queue.
    pub fn get_server_side_props(&mut self, route_key: &str, context_json: &str) -> Result<String> {
        let result_str = {
            v8::scope_with_context!(scope, &mut self.isolate, &self.context);

            let func = v8::Local::new(scope, &self.gssp_fn);
            let undef = v8::undefined(scope);
            let arg0 = v8::String::new(scope, route_key)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
            let arg1 = v8::String::new(scope, context_json)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;

            let result = v8_call!(scope, func, undef.into(), &[arg0.into(), arg1.into()])
                .map_err(|e| anyhow::anyhow!("GSSP error: {e}"))?;

            result.to_rust_string_lossy(scope)
        };

        if result_str == "__REX_ASYNC__" {
            // GSSP returned a promise — pump V8's microtask queue to resolve it
            self.isolate.perform_microtask_checkpoint();

            v8::scope_with_context!(scope, &mut self.isolate, &self.context);
            let resolve_result =
                v8_eval!(scope, "globalThis.__rex_resolve_gssp()", "<gssp-resolve>")
                    .map_err(|e| anyhow::anyhow!("GSSP error: {e}"))?;
            Ok(resolve_result.to_rust_string_lossy(scope))
        } else {
            Ok(result_str)
        }
    }

    /// Call __rex_get_static_props(routeKey, contextJson) and return JSON.
    /// Handles async GSP functions by pumping V8's microtask queue.
    pub fn get_static_props(&mut self, route_key: &str, context_json: &str) -> Result<String> {
        let result_str = {
            v8::scope_with_context!(scope, &mut self.isolate, &self.context);

            let func = v8::Local::new(scope, &self.gsp_fn);
            let undef = v8::undefined(scope);
            let arg0 = v8::String::new(scope, route_key)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
            let arg1 = v8::String::new(scope, context_json)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;

            let result = v8_call!(scope, func, undef.into(), &[arg0.into(), arg1.into()])
                .map_err(|e| anyhow::anyhow!("GSP error: {e}"))?;

            result.to_rust_string_lossy(scope)
        };

        if result_str == "__REX_GSP_ASYNC__" {
            self.isolate.perform_microtask_checkpoint();

            v8::scope_with_context!(scope, &mut self.isolate, &self.context);
            let resolve_result = v8_eval!(scope, "globalThis.__rex_resolve_gsp()", "<gsp-resolve>")
                .map_err(|e| anyhow::anyhow!("GSP error: {e}"))?;
            Ok(resolve_result.to_rust_string_lossy(scope))
        } else {
            Ok(result_str)
        }
    }

    /// Call __rex_call_api_handler(routeKey, reqJson) and return JSON response.
    /// Handles async handlers by pumping V8's microtask queue.
    pub fn call_api_handler(&mut self, route_key: &str, req_json: &str) -> Result<String> {
        let api_fn = self
            .api_handler_fn
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("API handlers not loaded"))?;

        let result_str = {
            v8::scope_with_context!(scope, &mut self.isolate, &self.context);

            let func = v8::Local::new(scope, api_fn);
            let undef = v8::undefined(scope);
            let arg0 = v8::String::new(scope, route_key)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
            let arg1 = v8::String::new(scope, req_json)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;

            let result = v8_call!(scope, func, undef.into(), &[arg0.into(), arg1.into()])
                .map_err(|e| anyhow::anyhow!("API handler error: {e}"))?;

            result.to_rust_string_lossy(scope)
        };

        if result_str == "__REX_API_ASYNC__" {
            self.isolate.perform_microtask_checkpoint();

            v8::scope_with_context!(scope, &mut self.isolate, &self.context);
            let resolve_result = v8_eval!(scope, "globalThis.__rex_resolve_api()", "<api-resolve>")
                .map_err(|e| anyhow::anyhow!("API handler error: {e}"))?;
            Ok(resolve_result.to_rust_string_lossy(scope))
        } else {
            Ok(result_str)
        }
    }

    /// Call __rex_render_document() to get document descriptor JSON.
    /// Returns None if no custom _document is loaded.
    pub fn render_document(&mut self) -> Result<Option<String>> {
        let doc_fn = match &self.document_fn {
            Some(f) => f,
            None => return Ok(None),
        };

        v8::scope_with_context!(scope, &mut self.isolate, &self.context);

        let func = v8::Local::new(scope, doc_fn);
        let undef = v8::undefined(scope);

        let result = v8_call!(scope, func, undef.into(), &[])
            .map_err(|e| anyhow::anyhow!("Document render error: {e}"))?;

        Ok(Some(result.to_rust_string_lossy(scope)))
    }

    /// Reload the server bundle (for dev mode hot reload)
    pub fn reload(&mut self, server_bundle_js: &str) -> Result<()> {
        v8::scope_with_context!(scope, &mut self.isolate, &self.context);

        v8_eval!(scope, "globalThis.__rex_pages = {};", "<reload>")?;
        v8_eval!(scope, server_bundle_js, "server-bundle.js")
            .context("Failed to evaluate updated server bundle")?;

        let ctx = scope.get_current_context();
        let global = ctx.global(scope);

        let render_fn = v8_get_global_fn!(scope, global, "__rex_render_page")?;
        let gssp_fn = v8_get_global_fn!(scope, global, "__rex_get_server_side_props")?;
        let gsp_fn = v8_get_global_fn!(scope, global, "__rex_get_static_props")?;

        self.render_fn = v8::Global::new(scope, render_fn);
        self.gssp_fn = v8::Global::new(scope, gssp_fn);
        self.gsp_fn = v8::Global::new(scope, gsp_fn);

        // Re-lookup API handler (may be added/removed on reload)
        self.api_handler_fn = v8_get_optional_fn!(scope, global, "__rex_call_api_handler");

        // Re-lookup document renderer (may be added/removed on reload)
        self.document_fn = v8_get_optional_fn!(scope, global, "__rex_render_document");

        debug!("SSR isolate reloaded");
        Ok(())
    }
}

fn format_args(scope: &mut v8::PinScope, args: &v8::FunctionCallbackArguments) -> String {
    let mut parts = Vec::new();
    for i in 0..args.length() {
        let arg = args.get(i);
        parts.push(arg.to_rust_string_lossy(scope));
    }
    parts.join(" ")
}

fn console_log(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _: v8::ReturnValue) {
    tracing::info!(target: "v8::console", "{}", format_args(scope, &args));
}

fn console_warn(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _: v8::ReturnValue) {
    tracing::warn!(target: "v8::console", "{}", format_args(scope, &args));
}

fn console_error(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _: v8::ReturnValue,
) {
    tracing::error!(target: "v8::console", "{}", format_args(scope, &args));
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Minimal React stub: provides React.createElement and ReactDOMServer.renderToString
    /// without needing node_modules. Renders elements as simple HTML strings.
    const MOCK_REACT_RUNTIME: &str = r#"
        globalThis.__React = {
            createElement: function(type, props) {
                var children = Array.prototype.slice.call(arguments, 2);
                return { type: type, props: props || {}, children: children };
            }
        };
        var React = globalThis.__React;

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
                    if (k === 'children') continue;
                    if (p.hasOwnProperty(k)) attrs += ' ' + k + '="' + p[k] + '"';
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
        SsrIsolate::new(&bundle).expect("failed to create isolate")
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
            Some("function(ctx) { return { props: { slug: ctx.params.slug, url: ctx.resolved_url } }; }"),
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
        let result = SsrIsolate::new("this is not valid javascript {{{{");
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
        SsrIsolate::new(&bundle).expect("failed to create isolate")
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
}
