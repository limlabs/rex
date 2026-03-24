#![allow(clippy::unwrap_used)]

use super::*;
use crate::prerender::PrerenderedPage;
use axum::routing::get;
use axum::Router;
use rex_core::{DataStrategy, DynamicSegment, Fallback, PageType, Route};
use rex_router::RouteTrie;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

/// Same minimal React stub as rex_v8 tests.
pub(super) const MOCK_REACT_RUNTIME: &str = r#"
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

pub(super) fn make_server_bundle(pages: &[(&str, &str, Option<&str>)]) -> String {
    let mut bundle = String::new();
    bundle.push_str("'use strict';\n");
    bundle.push_str("globalThis.__rex_pages = globalThis.__rex_pages || {};\n\n");

    for (key, component, gssp) in pages {
        bundle.push_str(&format!(
            "globalThis.__rex_pages['{}'] = (function() {{\n  var exports = {{}};\n",
            key
        ));
        bundle.push_str(&format!("  exports.default = {};\n", component));
        if let Some(gssp_code) = gssp {
            // Check if code is tagged for getStaticProps
            if let Some(gsp_code) = gssp_code.strip_prefix("GSP:") {
                bundle.push_str(&format!("  exports.getStaticProps = {gsp_code};\n"));
            } else {
                bundle.push_str(&format!("  exports.getServerSideProps = {gssp_code};\n"));
            }
        }
        bundle.push_str("  return exports;\n})();\n\n");
    }

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
    if (!React || !ReactDOMServer) throw new Error('React not loaded');
    var page = globalThis.__rex_pages[routeKey];
    if (!page) throw new Error('Page not found: ' + routeKey);
    var Component = page.default;
    if (!Component) throw new Error('No default export: ' + routeKey);
    var props = JSON.parse(propsJson);

    globalThis.__rex_head_elements = [];
    var bodyHtml = ReactDOMServer.renderToString(React.createElement(Component, props));
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

globalThis.__rex_gsp_paths_resolved = null;
globalThis.__rex_gsp_paths_rejected = null;

globalThis.__rex_get_static_paths = function(routeKey) {
    var page = globalThis.__rex_pages[routeKey];
    if (!page || !page.getStaticPaths) return JSON.stringify({ paths: [], fallback: false });
    var result = page.getStaticPaths();
    if (result && typeof result.then === 'function') {
        globalThis.__rex_gsp_paths_resolved = null;
        globalThis.__rex_gsp_paths_rejected = null;
        result.then(
            function(v) { globalThis.__rex_gsp_paths_resolved = v; },
            function(e) { globalThis.__rex_gsp_paths_rejected = e; }
        );
        return '__REX_GSP_PATHS_ASYNC__';
    }
    return JSON.stringify(result);
};

globalThis.__rex_resolve_static_paths = function() {
    if (globalThis.__rex_gsp_paths_rejected) throw globalThis.__rex_gsp_paths_rejected;
    if (globalThis.__rex_gsp_paths_resolved !== null) return JSON.stringify(globalThis.__rex_gsp_paths_resolved);
    throw new Error('getStaticPaths promise did not resolve');
};
"#,
    );
    bundle
}

pub(super) fn make_route(pattern: &str, file_path: &str, segments: Vec<DynamicSegment>) -> Route {
    let specificity = if segments.is_empty() { 100 } else { 50 };
    Route {
        pattern: pattern.to_string(),
        file_path: PathBuf::from(file_path),
        abs_path: PathBuf::from(format!("/fake/pages/{file_path}")),
        dynamic_segments: segments,
        page_type: PageType::Regular,
        specificity,
    }
}

pub(super) async fn body_string(body: axum::body::Body) -> String {
    use http_body_util::BodyExt;
    let bytes = body.collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// Unified test app builder — replaces the 3 separate `build_test_app*` functions.
pub(super) struct TestAppBuilder {
    routes: Vec<Route>,
    pages: Vec<(&'static str, &'static str, Option<&'static str>)>,
    api_routes: Vec<Route>,
    app_routes: Vec<Route>,
    project_config: rex_core::ProjectConfig,
    project_root: Option<PathBuf>,
    middleware_js: Option<String>,
    middleware_matchers: Option<Vec<String>>,
    app_api_routes: Vec<Route>,
    extra_bundle: Option<String>,
    custom_router: Option<Box<dyn FnOnce(Arc<AppState>) -> Router>>,
    is_dev: bool,
    has_custom_404: bool,
    has_custom_error: bool,
    prerendered: std::collections::HashMap<String, PrerenderedPage>,
    static_paths_pages: Vec<(String, Fallback)>,
    strategy_overrides: Vec<(String, DataStrategy)>,
}

impl TestAppBuilder {
    pub fn new() -> Self {
        Self {
            routes: Vec::new(),
            pages: Vec::new(),
            api_routes: Vec::new(),
            app_routes: Vec::new(),
            app_api_routes: Vec::new(),
            project_config: rex_core::ProjectConfig::default(),
            project_root: None,
            middleware_js: None,
            middleware_matchers: None,
            extra_bundle: None,
            custom_router: None,
            is_dev: false,
            has_custom_404: false,
            has_custom_error: false,
            prerendered: std::collections::HashMap::new(),
            static_paths_pages: Vec::new(),
            strategy_overrides: Vec::new(),
        }
    }

    pub fn routes(
        mut self,
        routes: Vec<Route>,
        pages: Vec<(&'static str, &'static str, Option<&'static str>)>,
    ) -> Self {
        self.routes = routes;
        self.pages = pages;
        self
    }

    pub fn config(mut self, config: rex_core::ProjectConfig) -> Self {
        self.project_config = config;
        self
    }

    pub fn api_routes(mut self, routes: Vec<Route>) -> Self {
        self.api_routes = routes;
        self
    }

    pub fn app_routes(mut self, routes: Vec<Route>) -> Self {
        self.app_routes = routes;
        self
    }

    pub fn app_api_routes(mut self, routes: Vec<Route>) -> Self {
        self.app_api_routes = routes;
        self
    }

    pub fn project_root(mut self, path: PathBuf) -> Self {
        self.project_root = Some(path);
        self
    }

    pub fn dev_mode(mut self) -> Self {
        self.is_dev = true;
        self
    }

    pub fn custom_404(mut self) -> Self {
        self.has_custom_404 = true;
        self
    }

    pub fn custom_error(mut self) -> Self {
        self.has_custom_error = true;
        self
    }

    pub fn middleware(mut self, js: &str, matchers: Vec<String>) -> Self {
        self.middleware_js = Some(js.to_string());
        self.middleware_matchers = Some(matchers);
        self
    }

    pub fn prerendered(mut self, path: &str, html: &str, props_json: &str) -> Self {
        self.prerendered.insert(
            path.to_string(),
            PrerenderedPage {
                html: html.to_string(),
                props_json: props_json.to_string(),
            },
        );
        self
    }

    pub fn page_strategy(mut self, pattern: &str, strategy: DataStrategy) -> Self {
        self.strategy_overrides
            .push((pattern.to_string(), strategy));
        self
    }

    pub fn static_paths_page(mut self, pattern: &str, fallback: Fallback) -> Self {
        self.static_paths_pages
            .push((pattern.to_string(), fallback));
        self
    }

    pub fn extra_bundle(mut self, js: &str) -> Self {
        self.extra_bundle = Some(js.to_string());
        self
    }

    pub fn custom_router(mut self, f: impl FnOnce(Arc<AppState>) -> Router + 'static) -> Self {
        self.custom_router = Some(Box::new(f));
        self
    }

    pub fn build(self) -> Router {
        rex_v8::init_v8();

        let mut bundle = format!(
            "{}\n{}",
            MOCK_REACT_RUNTIME,
            make_server_bundle(&self.pages)
        );
        if let Some(mw_js) = &self.middleware_js {
            bundle.push_str(mw_js);
        }
        if let Some(extra) = &self.extra_bundle {
            bundle.push('\n');
            bundle.push_str(extra);
        }

        let pool =
            rex_v8::IsolatePool::new(1, Arc::new(bundle), None).expect("failed to create pool");

        let trie = RouteTrie::from_routes(&self.routes);
        let mut manifest = rex_core::AssetManifest::new("test-build-id".to_string());

        for (route, (_, _, gssp)) in self.routes.iter().zip(self.pages.iter()) {
            let strategy = if gssp.is_some() {
                DataStrategy::GetServerSideProps
            } else {
                DataStrategy::None
            };
            let has_dynamic = !route.dynamic_segments.is_empty();
            manifest.add_page(&route.pattern, "test.js", strategy, has_dynamic);
        }

        // Apply data strategy overrides
        for (pattern, strategy) in &self.strategy_overrides {
            if let Some(page) = manifest.pages.get_mut(pattern.as_str()) {
                page.data_strategy = strategy.clone();
            }
        }

        // Mark pages with getStaticPaths metadata
        for (pattern, fallback) in &self.static_paths_pages {
            if let Some(page) = manifest.pages.get_mut(pattern.as_str()) {
                page.has_static_paths = true;
                page.fallback = *fallback;
            }
        }

        let build_id = "test-build-id".to_string();
        let manifest_json = HotState::compute_manifest_json(&build_id, &manifest);

        let has_middleware = self.middleware_js.is_some();
        let middleware_matchers = if has_middleware {
            self.middleware_matchers
        } else {
            None
        };

        let api_trie = RouteTrie::from_routes(&self.api_routes);
        let app_route_trie = if self.app_routes.is_empty() {
            None
        } else {
            Some(RouteTrie::from_routes(&self.app_routes))
        };

        let project_root = self
            .project_root
            .unwrap_or_else(|| PathBuf::from("/tmp/rex-test"));

        let state = Arc::new(AppState {
            isolate_pool: pool,
            is_dev: self.is_dev,
            image_cache: rex_image::ImageCache::new(project_root.join(".rex-cache")),
            project_root,
            esm: None,
            client_deps: std::sync::OnceLock::new(),
            browser_transform_cache: std::sync::OnceLock::new(),
            lazy_init: tokio::sync::OnceCell::const_new_with(()),
            lazy_init_ctx: std::sync::Mutex::new(None),
            hot: RwLock::new(Arc::new(HotState {
                route_trie: trie,
                api_route_trie: api_trie,
                manifest,
                build_id,
                has_custom_404: self.has_custom_404,
                has_custom_error: self.has_custom_error,
                has_custom_document: false,
                project_config: self.project_config,
                manifest_json,
                document_descriptor: None,
                has_middleware,
                middleware_matchers,
                app_route_trie,
                app_api_route_trie: if self.app_api_routes.is_empty() {
                    None
                } else {
                    Some(RouteTrie::from_routes(&self.app_api_routes))
                },
                has_mcp_tools: false,
                prerendered: self.prerendered,
                prerendered_app: std::collections::HashMap::new(),
                import_map_json: None,
            })),
        });

        if let Some(custom) = self.custom_router {
            return custom(state);
        }

        Router::new()
            .route("/_rex/data/{build_id}/{*path}", get(data_handler))
            .fallback(page_handler)
            .with_state(state)
    }
}

/// Minimal middleware runtime for tests (mirrors MIDDLEWARE_RUNTIME from bundler).
pub(super) const TEST_MIDDLEWARE_REDIRECT: &str = r#"
    globalThis.__rex_run_middleware = function(reqJson) {
        var req = JSON.parse(reqJson);
        if (req.pathname === '/protected') {
            return JSON.stringify({
                action: 'redirect',
                url: '/login',
                status: 302,
                request_headers: {},
                response_headers: {}
            });
        }
        return JSON.stringify({
            action: 'next',
            url: null,
            status: 307,
            request_headers: {},
            response_headers: {}
        });
    };
"#;

/// API handler runtime for tests.
pub(super) const TEST_API_RUNTIME: &str = r#"
    globalThis.__rex_api_routes = globalThis.__rex_api_routes || {};
    globalThis.__rex_api_routes['api/hello'] = function(req) {
        var parsed = JSON.parse(req);
        if (parsed.method === 'POST') {
            return JSON.stringify({
                statusCode: 200,
                headers: { 'content-type': 'application/json' },
                body: JSON.stringify({ echo: parsed.body })
            });
        }
        return JSON.stringify({
            statusCode: 200,
            headers: { 'content-type': 'application/json' },
            body: JSON.stringify({ message: 'hello from api' })
        });
    };
    globalThis.__rex_call_api_handler = function(routeKey, reqJson) {
        var handler = globalThis.__rex_api_routes[routeKey];
        if (!handler) throw new Error('API route not found: ' + routeKey);
        return handler(reqJson);
    };
"#;

/// Middleware runtime that rewrites /old-path to /new-path.
pub(super) const TEST_MIDDLEWARE_REWRITE: &str = r#"
    globalThis.__rex_run_middleware = function(reqJson) {
        var req = JSON.parse(reqJson);
        if (req.pathname === '/rewrite-me') {
            return JSON.stringify({
                action: 'rewrite',
                url: '/',
                status: 200,
                request_headers: {},
                response_headers: { 'x-rewritten': 'true' }
            });
        }
        return JSON.stringify({
            action: 'next',
            url: null,
            status: 307,
            request_headers: {},
            response_headers: {}
        });
    };
"#;

/// RSC flight runtime for tests.
pub(super) const TEST_RSC_FLIGHT_RUNTIME: &str = r#"
    globalThis.__rex_render_flight = function(routeKey, propsJson) {
        var props = JSON.parse(propsJson);
        return JSON.stringify({
            route: routeKey,
            params: props.params || {},
            searchParams: props.searchParams || {}
        });
    };
"#;

/// App route handler runtime for tests (route.ts).
pub(super) const TEST_APP_ROUTE_HANDLER_RUNTIME: &str = r#"
    globalThis.__rex_app_route_handlers = globalThis.__rex_app_route_handlers || {};
    globalThis.__rex_app_route_handlers['/api/hello'] = {
        GET: function(req) {
            return { statusCode: 200, headers: { 'content-type': 'application/json' }, body: JSON.stringify({ message: 'hello from route handler' }) };
        },
        POST: function(req) {
            return { statusCode: 201, headers: { 'content-type': 'application/json' }, body: JSON.stringify({ created: true }) };
        }
    };

    globalThis.__rex_call_app_route_handler = function(routePattern, reqJson) {
        var handlers = globalThis.__rex_app_route_handlers;
        if (!handlers) throw new Error('No app route handlers registered');
        var routeModule = handlers[routePattern];
        if (!routeModule) throw new Error('App route handler not found: ' + routePattern);
        var reqData = JSON.parse(reqJson);
        var method = (reqData.method || 'GET').toUpperCase();
        var handlerFn = routeModule[method];
        if (!handlerFn) {
            var allowed = ['GET','HEAD','POST','PUT','DELETE','PATCH','OPTIONS'].filter(function(m) { return typeof routeModule[m] === 'function'; });
            return JSON.stringify({ statusCode: 405, headers: { allow: allowed.join(', ') }, body: 'Method Not Allowed' });
        }
        var result = handlerFn(reqData);
        return JSON.stringify(result);
    };
"#;

/// App route handler that throws (for testing V8 error path).
pub(super) const TEST_APP_ROUTE_HANDLER_THROWS: &str = r#"
    globalThis.__rex_call_app_route_handler = function(routePattern, reqJson) {
        throw new Error('handler exploded');
    };
"#;

/// App route handler that returns invalid JSON (for testing parse error path).
pub(super) const TEST_APP_ROUTE_HANDLER_BAD_JSON: &str = r#"
    globalThis.__rex_call_app_route_handler = function(routePattern, reqJson) {
        return 'not valid json {{{';
    };
"#;

/// Action runtime for server action tests.
pub(super) const TEST_ACTION_RUNTIME: &str = r#"
    globalThis.__rex_server_actions = {
        "test_action_id": function(x) { return x + 1; }
    };
    globalThis.__rex_call_server_action = function(actionId, argsJson) {
        var actions = globalThis.__rex_server_actions || {};
        var fn = actions[actionId];
        if (!fn) return JSON.stringify({ error: "Server action not found: " + actionId });
        var args = JSON.parse(argsJson);
        try {
            var result = fn.apply(null, args);
            return JSON.stringify({ result: result });
        } catch (e) {
            return JSON.stringify({ error: String(e) });
        }
    };
"#;
