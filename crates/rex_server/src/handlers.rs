use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Response};
use rex_core::{ServerSidePropsContext, ServerSidePropsResult};
use rex_router::RouteTrie;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, error, info};

use crate::document::{assemble_document, DocumentDescriptor};

/// Shared application state
pub struct AppState {
    pub route_trie: RouteTrie,
    pub api_route_trie: RouteTrie,
    pub isolate_pool: rex_v8::IsolatePool,
    pub manifest: rex_build::AssetManifest,
    pub build_id: String,
    pub is_dev: bool,
    pub has_custom_404: bool,
    pub has_custom_error: bool,
    pub has_custom_document: bool,
}

/// Render a custom error page (404 or _error) via SSR, returning the full HTML document.
async fn render_error_page(
    state: &Arc<AppState>,
    page_key: &str,
    status: StatusCode,
    props: &str,
) -> Response {
    let key = page_key.to_string();
    let props_clone = props.to_string();
    let ssr_result = state
        .isolate_pool
        .execute(move |iso| iso.render_page(&key, &props_clone))
        .await;

    let render = match ssr_result {
        Ok(Ok(r)) => r,
        _ => return (status, Html(format!("{} Error", status.as_u16()))).into_response(),
    };

    // Render _document if present
    let doc_desc = get_document_descriptor(&state).await;

    let document = assemble_document(
        &render.body,
        &render.head,
        props,
        &state.manifest.vendor_scripts,
        &[],
        &state.manifest.global_css,
        state.manifest.app_script.as_deref(),
        &state.build_id,
        state.is_dev,
        doc_desc.as_ref(),
    );

    (status, Html(document)).into_response()
}

/// Get document descriptor from _document rendering, if present.
async fn get_document_descriptor(state: &Arc<AppState>) -> Option<DocumentDescriptor> {
    if !state.has_custom_document {
        return None;
    }
    let result = state
        .isolate_pool
        .execute(move |iso| iso.render_document())
        .await;
    match result {
        Ok(Ok(Some(json))) => {
            serde_json::from_str(&json).ok()
        }
        _ => None,
    }
}

/// API response from V8 handler execution
#[derive(serde::Deserialize)]
struct ApiResponse {
    #[serde(rename = "statusCode")]
    status_code: u16,
    #[serde(default)]
    headers: HashMap<String, String>,
    #[serde(default)]
    body: String,
}

/// API route handler - handles all HTTP methods for /api/* routes
pub async fn api_handler(
    State(state): State<Arc<AppState>>,
    method: axum::http::Method,
    uri: Uri,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let path = uri.path();
    info!(path, method = %method, "Handling API request");

    let route_match = match state.api_route_trie.match_path(path) {
        Some(m) => m,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    let route_key = route_match.route.module_name();

    // Parse query string
    let query: HashMap<String, String> = uri
        .query()
        .map(|q| {
            url::form_urlencoded::parse(q.as_bytes())
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        })
        .unwrap_or_default();

    // Parse body based on content-type
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let body_value = if content_type.starts_with("application/json") {
        serde_json::from_slice::<serde_json::Value>(&body).unwrap_or(serde_json::Value::Null)
    } else if !body.is_empty() {
        serde_json::Value::String(String::from_utf8_lossy(&body).into_owned())
    } else {
        serde_json::Value::Null
    };

    // Build request JSON for V8
    let req_data = serde_json::json!({
        "method": method.as_str(),
        "url": path,
        "headers": headers.iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect::<HashMap<String, String>>(),
        "query": query,
        "body": body_value,
        "cookies": {},
    });
    let req_json = serde_json::to_string(&req_data).unwrap();

    // Execute in V8
    let result = state
        .isolate_pool
        .execute(move |iso| iso.call_api_handler(&route_key, &req_json))
        .await;

    match result {
        Ok(Ok(json)) => {
            let api_res: ApiResponse = match serde_json::from_str(&json) {
                Ok(r) => r,
                Err(e) => {
                    error!("Failed to parse API response: {e}");
                    return (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response();
                }
            };

            let status = StatusCode::from_u16(api_res.status_code).unwrap_or(StatusCode::OK);
            let mut builder = Response::builder().status(status);
            for (k, v) in &api_res.headers {
                builder = builder.header(k.as_str(), v.as_str());
            }
            builder.body(Body::from(api_res.body)).unwrap()
        }
        Ok(Err(e)) => {
            error!("API handler V8 error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, format!("API error: {e}")).into_response()
        }
        Err(e) => {
            error!("API handler pool error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Main page handler - catches all routes and performs SSR
pub async fn page_handler(
    State(state): State<Arc<AppState>>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let path = uri.path();
    info!(path, "Handling page request");

    // Try to match the route
    let route_match = match state.route_trie.match_path(path) {
        Some(m) => m,
        None => {
            debug!(path, "No route matched");
            if state.has_custom_404 {
                return render_error_page(&state, "404", StatusCode::NOT_FOUND, "{}").await;
            }
            return (StatusCode::NOT_FOUND, Html("404 - Page Not Found".to_string())).into_response();
        }
    };

    let route_key = route_match.route.module_name();
    let params = route_match.params.clone();

    // Parse query string
    let query: HashMap<String, String> = uri
        .query()
        .map(|q| {
            url::form_urlencoded::parse(q.as_bytes())
                .map(|(k, v): (std::borrow::Cow<str>, std::borrow::Cow<str>)| {
                    (k.to_string(), v.to_string())
                })
                .collect()
        })
        .unwrap_or_default();

    // Extract headers
    let header_map: HashMap<String, String> = headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();

    // Build GSSP context
    let context = ServerSidePropsContext {
        params,
        query,
        resolved_url: path.to_string(),
        headers: header_map,
        cookies: HashMap::new(),
    };
    let context_json = serde_json::to_string(&context).unwrap();

    // Execute getServerSideProps
    let route_key_clone = route_key.clone();
    let gssp_result = state
        .isolate_pool
        .execute(move |iso| iso.get_server_side_props(&route_key_clone, &context_json))
        .await;

    let props_json = match gssp_result {
        Ok(Ok(json)) => json,
        Ok(Err(e)) => {
            error!("GSSP error: {e}");
            if state.has_custom_error {
                let err_props = serde_json::json!({ "statusCode": 500 }).to_string();
                return render_error_page(&state, "_error", StatusCode::INTERNAL_SERVER_ERROR, &err_props).await;
            }
            return (StatusCode::INTERNAL_SERVER_ERROR, Html(format!("Server error: {e}"))).into_response();
        }
        Err(e) => {
            error!("Isolate pool error: {e}");
            if state.has_custom_error {
                let err_props = serde_json::json!({ "statusCode": 500 }).to_string();
                return render_error_page(&state, "_error", StatusCode::INTERNAL_SERVER_ERROR, &err_props).await;
            }
            return (StatusCode::INTERNAL_SERVER_ERROR, Html("Internal server error".to_string())).into_response();
        }
    };

    // Parse GSSP result to check for redirect/notFound
    match serde_json::from_str::<ServerSidePropsResult>(&props_json) {
        Ok(ServerSidePropsResult::Redirect { redirect }) => {
            let status = if redirect.permanent { 301 } else { redirect.status_code };
            debug!(destination = %redirect.destination, status, "Redirecting");
            return Response::builder()
                .status(status)
                .header("Location", &redirect.destination)
                .body(Body::empty())
                .unwrap();
        }
        Ok(ServerSidePropsResult::NotFound { not_found: true }) => {
            if state.has_custom_404 {
                return render_error_page(&state, "404", StatusCode::NOT_FOUND, "{}").await;
            }
            return (StatusCode::NOT_FOUND, Html("404 - Not Found".to_string())).into_response();
        }
        _ => {}
    }

    // Extract just the props value for rendering
    let render_props = match serde_json::from_str::<serde_json::Value>(&props_json) {
        Ok(val) => {
            if let Some(props) = val.get("props") {
                serde_json::to_string(props).unwrap()
            } else {
                "{}".to_string()
            }
        }
        Err(_) => "{}".to_string(),
    };

    // Render the page
    let route_key_clone = route_key.clone();
    let render_props_clone = render_props.clone();
    let ssr_result = state
        .isolate_pool
        .execute(move |iso| iso.render_page(&route_key_clone, &render_props_clone))
        .await;

    let render = match ssr_result {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            error!("SSR render error: {e}");
            if state.is_dev {
                let err_html = format!("<div style=\"color:red;font-family:monospace;padding:20px;\"><h2>SSR Error</h2><pre>{e}</pre></div>");
                let document = assemble_document(&err_html, "", &render_props, &state.manifest.vendor_scripts, &[], &state.manifest.global_css, state.manifest.app_script.as_deref(), &state.build_id, state.is_dev, None);
                return Html(document).into_response();
            } else if state.has_custom_error {
                let err_props = serde_json::json!({ "statusCode": 500 }).to_string();
                return render_error_page(&state, "_error", StatusCode::INTERNAL_SERVER_ERROR, &err_props).await;
            } else {
                return (StatusCode::INTERNAL_SERVER_ERROR, Html("Internal server error".to_string())).into_response();
            }
        }
        Err(e) => {
            error!("Isolate pool error: {e}");
            if state.has_custom_error {
                let err_props = serde_json::json!({ "statusCode": 500 }).to_string();
                return render_error_page(&state, "_error", StatusCode::INTERNAL_SERVER_ERROR, &err_props).await;
            }
            return (StatusCode::INTERNAL_SERVER_ERROR, Html("Internal server error".to_string())).into_response();
        }
    };

    // Look up client assets for this route
    let page_assets = state.manifest.pages.get(&route_match.route.pattern);
    let client_scripts: Vec<String> = page_assets
        .map(|assets| vec![assets.js.clone()])
        .unwrap_or_default();

    // Collect CSS: global (from _app) + per-page
    let mut css_files = state.manifest.global_css.clone();
    if let Some(assets) = page_assets {
        css_files.extend(assets.css.iter().cloned());
    }

    // Render _document if present
    let doc_desc = get_document_descriptor(&state).await;

    let document = assemble_document(
        &render.body,
        &render.head,
        &render_props,
        &state.manifest.vendor_scripts,
        &client_scripts,
        &css_files,
        state.manifest.app_script.as_deref(),
        &state.build_id,
        state.is_dev,
        doc_desc.as_ref(),
    );

    Html(document).into_response()
}

/// Data endpoint: GET /_rex/data/{buildId}/{path}.json
/// Returns GSSP result as JSON for client-side navigation
pub async fn data_handler(
    State(state): State<Arc<AppState>>,
    Path((build_id, page_path)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    // Build ID mismatch = stale client
    if build_id != state.build_id {
        return StatusCode::NOT_FOUND.into_response();
    }

    let path = format!("/{}", page_path.trim_end_matches(".json"));

    let route_match = match state.route_trie.match_path(&path) {
        Some(m) => m,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    let route_key = route_match.route.module_name();
    let context = ServerSidePropsContext {
        params: route_match.params,
        query: HashMap::new(),
        resolved_url: path,
        headers: headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect(),
        cookies: HashMap::new(),
    };
    let context_json = serde_json::to_string(&context).unwrap();

    let result = state
        .isolate_pool
        .execute(move |iso| iso.get_server_side_props(&route_key, &context_json))
        .await;

    match result {
        Ok(Ok(json)) => Response::builder()
            .header("Content-Type", "application/json")
            .body(Body::from(json))
            .unwrap(),
        Ok(Err(e)) => {
            error!("Data endpoint GSSP error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
        Err(e) => {
            error!("Data endpoint pool error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use axum::routing::get;
    use axum::Router;
    use http_body_util::BodyExt;
    use rex_core::{DynamicSegment, PageType, Route};
    use std::path::PathBuf;
    use tower::ServiceExt;

    /// Same minimal React stub as rex_v8 tests.
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

    fn make_server_bundle(pages: &[(&str, &str, Option<&str>)]) -> String {
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
                bundle.push_str(&format!(
                    "  exports.getServerSideProps = {};\n",
                    gssp_code
                ));
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
"#,
        );
        bundle
    }

    fn make_route(pattern: &str, file_path: &str, segments: Vec<DynamicSegment>) -> Route {
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

    /// Build a test app with router, isolate pool, and routes wired up.
    fn build_test_app(
        routes: Vec<Route>,
        pages: &[(&str, &str, Option<&str>)],
    ) -> Router {
        rex_v8::init_v8();
        let bundle = format!("{}\n{}", MOCK_REACT_RUNTIME, make_server_bundle(pages));
        let pool = rex_v8::IsolatePool::new(
            1,
            Arc::new(bundle),
        )
        .expect("failed to create pool");

        let trie = RouteTrie::from_routes(&routes);
        let manifest = rex_build::AssetManifest::new("test-build-id".to_string());

        let state = Arc::new(AppState {
            route_trie: trie,
            api_route_trie: RouteTrie::from_routes(&[]),
            isolate_pool: pool,
            manifest,
            build_id: "test-build-id".to_string(),
            is_dev: false,
            has_custom_404: false,
            has_custom_error: false,
            has_custom_document: false,
        });

        Router::new()
            .route(
                "/_rex/data/{build_id}/{*path}",
                get(data_handler),
            )
            .fallback(page_handler)
            .with_state(state)
    }

    async fn body_string(body: Body) -> String {
        let bytes = body.collect().await.unwrap().to_bytes();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn test_page_returns_html_with_ssr() {
        let app = build_test_app(
            vec![make_route("/", "index.tsx", vec![])],
            &[(
                "index",
                "function Index() { return React.createElement('h1', null, 'Home'); }",
                None,
            )],
        );

        let resp = app
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let html = body_string(resp.into_body()).await;
        assert!(html.contains("<h1>Home</h1>"), "missing SSR content: {html}");
        assert!(html.contains("<!DOCTYPE html>"), "missing doctype: {html}");
        assert!(html.contains("__REX_DATA__"), "missing data script: {html}");
    }

    #[tokio::test]
    async fn test_page_404_no_route() {
        let app = build_test_app(
            vec![make_route("/", "index.tsx", vec![])],
            &[(
                "index",
                "function Index() { return React.createElement('h1', null, 'Home'); }",
                None,
            )],
        );

        let resp = app
            .oneshot(Request::get("/nonexistent").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_page_with_gssp_props() {
        let app = build_test_app(
            vec![make_route("/", "index.tsx", vec![])],
            &[(
                "index",
                "function Index(props) { return React.createElement('p', null, props.msg); }",
                Some("function(ctx) { return { props: { msg: 'hello from gssp' } }; }"),
            )],
        );

        let resp = app
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let html = body_string(resp.into_body()).await;
        assert!(
            html.contains("<p>hello from gssp</p>"),
            "GSSP props not rendered: {html}"
        );
    }

    #[tokio::test]
    async fn test_page_gssp_redirect() {
        let app = build_test_app(
            vec![make_route("/old", "old.tsx", vec![])],
            &[(
                "old",
                "function Old() { return React.createElement('div'); }",
                Some("function(ctx) { return { redirect: { destination: '/new', statusCode: 307 } }; }"),
            )],
        );

        let resp = app
            .oneshot(Request::get("/old").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(resp.headers().get("location").unwrap(), "/new");
    }

    #[tokio::test]
    async fn test_page_gssp_not_found() {
        let app = build_test_app(
            vec![make_route("/hidden", "hidden.tsx", vec![])],
            &[(
                "hidden",
                "function Hidden() { return React.createElement('div'); }",
                Some("function(ctx) { return { notFound: true }; }"),
            )],
        );

        let resp = app
            .oneshot(Request::get("/hidden").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_dynamic_route_params() {
        let app = build_test_app(
            vec![make_route(
                "/blog/:slug",
                "blog/[slug].tsx",
                vec![DynamicSegment::Single("slug".into())],
            )],
            &[(
                "blog/[slug]",
                "function Post(props) { return React.createElement('h1', null, props.slug); }",
                Some("function(ctx) { return { props: { slug: ctx.params.slug } }; }"),
            )],
        );

        let resp = app
            .oneshot(Request::get("/blog/my-post").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let html = body_string(resp.into_body()).await;
        assert!(
            html.contains("<h1>my-post</h1>"),
            "dynamic param not passed: {html}"
        );
    }

    #[tokio::test]
    async fn test_data_handler_returns_json() {
        let app = build_test_app(
            vec![make_route("/about", "about.tsx", vec![])],
            &[(
                "about",
                "function About() { return React.createElement('div'); }",
                Some("function(ctx) { return { props: { title: 'data test' } }; }"),
            )],
        );

        let resp = app
            .oneshot(
                Request::get("/_rex/data/test-build-id/about.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/json"
        );
        let json = body_string(resp.into_body()).await;
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["props"]["title"], "data test");
    }

    #[tokio::test]
    async fn test_data_handler_stale_build_id() {
        let app = build_test_app(
            vec![make_route("/", "index.tsx", vec![])],
            &[(
                "index",
                "function Index() { return React.createElement('div'); }",
                None,
            )],
        );

        let resp = app
            .oneshot(
                Request::get("/_rex/data/wrong-build-id/index.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_data_handler_no_route() {
        let app = build_test_app(
            vec![make_route("/", "index.tsx", vec![])],
            &[(
                "index",
                "function Index() { return React.createElement('div'); }",
                None,
            )],
        );

        let resp = app
            .oneshot(
                Request::get("/_rex/data/test-build-id/nonexistent.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
