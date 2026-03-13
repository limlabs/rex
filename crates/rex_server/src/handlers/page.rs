use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Response};
use futures::stream::{self, StreamExt};
use rex_core::{DataStrategy, MiddlewareAction, ServerSidePropsContext};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, error, info};

use crate::document::{assemble_body_tail, assemble_head_shell};

use super::action::parse_form_action_fields;
use super::{
    check_redirects, check_rewrites, collect_headers, dev_error_overlay, execute_middleware,
    render_error_page, rsc::render_app_route, should_run_middleware, AppState, HotState,
};
use crate::state::snapshot;

/// Main page handler - catches all routes and performs SSR
pub async fn page_handler(
    State(state): State<Arc<AppState>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let path = uri.path();
    info!(path, method = %method, "Handling page request");

    let hot = snapshot(&state);

    // Run middleware before anything else
    let mut mw_response_headers: Vec<(String, String)> = Vec::new();
    if should_run_middleware(path, &hot) {
        let header_map: HashMap<String, String> = headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        match execute_middleware(&state, path, method.as_str(), &header_map).await {
            Ok(Some(mw)) => match mw.action {
                MiddlewareAction::Redirect => {
                    let url = mw.url.as_deref().unwrap_or("/");
                    let status =
                        StatusCode::from_u16(mw.status).unwrap_or(StatusCode::TEMPORARY_REDIRECT);
                    return Response::builder()
                        .status(status)
                        .header("location", url)
                        .body(Body::empty())
                        .expect("response build");
                }
                MiddlewareAction::Rewrite => {
                    if let Some(url) = &mw.url {
                        // Parse the URL to get just the path
                        let rewrite_path = if let Ok(parsed) = url::Url::parse(url) {
                            parsed.path().to_string()
                        } else {
                            url.clone()
                        };
                        mw_response_headers = mw.response_headers.into_iter().collect();
                        let custom_headers = collect_headers(&rewrite_path, &hot.project_config);
                        let mut response = page_handler_inner(
                            &state,
                            &hot,
                            &rewrite_path,
                            &uri,
                            &headers,
                            &method,
                            &body,
                        )
                        .await;
                        for (key, value) in custom_headers.into_iter().chain(mw_response_headers) {
                            if let (Ok(name), Ok(val)) = (
                                axum::http::header::HeaderName::from_bytes(key.as_bytes()),
                                axum::http::header::HeaderValue::from_str(&value),
                            ) {
                                response.headers_mut().insert(name, val);
                            }
                        }
                        return response;
                    }
                }
                MiddlewareAction::Next => {
                    mw_response_headers = mw.response_headers.into_iter().collect();
                }
            },
            Ok(None) => {}
            Err(e) => {
                error!("Middleware error: {e}");
                if state.is_dev {
                    return Html(dev_error_overlay("Middleware Error", &e, None)).into_response();
                }
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        }
    }

    // Check redirect rules first
    if let Some(redirect_response) = check_redirects(path, &hot.project_config) {
        return redirect_response;
    }

    // Check rewrite rules (transparently serve a different path)
    let effective_path;
    let path = if let Some(rewritten) = check_rewrites(path, &hot.project_config) {
        effective_path = rewritten;
        &effective_path
    } else {
        path
    };

    // Collect custom headers to add to the response
    let custom_headers = collect_headers(path, &hot.project_config);

    let mut response = page_handler_inner(&state, &hot, path, &uri, &headers, &method, &body).await;

    // Apply custom headers + middleware response headers
    for (key, value) in custom_headers.into_iter().chain(mw_response_headers) {
        if let (Ok(name), Ok(val)) = (
            axum::http::header::HeaderName::from_bytes(key.as_bytes()),
            axum::http::header::HeaderValue::from_str(&value),
        ) {
            response.headers_mut().insert(name, val);
        }
    }

    response
}

pub(super) async fn page_handler_inner(
    state: &Arc<AppState>,
    hot: &Arc<HotState>,
    path: &str,
    uri: &Uri,
    headers: &HeaderMap,
    method: &Method,
    body: &axum::body::Bytes,
) -> Response {
    // Check app/ route handlers first (route.ts — API-style, no rendering)
    if let Some(ref app_api_trie) = hot.app_api_route_trie {
        if let Some(api_match) = app_api_trie.match_path(path) {
            return handle_app_route_handler(state, &api_match, method, uri, headers, body).await;
        }
    }

    // Check app/ page routes (RSC)
    if let Some(ref app_trie) = hot.app_route_trie {
        if let Some(app_match) = app_trie.match_path(path) {
            // Progressive enhancement: handle POST form submissions with $ACTION_ID_*
            if *method == Method::POST {
                if let Some(fields) = parse_form_action_fields(headers, body) {
                    if fields.iter().any(|(k, _)| k.starts_with("$ACTION_ID_")) {
                        // CSRF protection: validate Origin header against Host
                        if let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) {
                            if let Some(host) = headers.get("host").and_then(|v| v.to_str().ok()) {
                                let origin_host = origin
                                    .trim_start_matches("https://")
                                    .trim_start_matches("http://");
                                if origin_host != host {
                                    return Response::builder()
                                        .status(StatusCode::FORBIDDEN)
                                        .body(Body::from("CSRF validation failed"))
                                        .expect("response build");
                                }
                            }
                        }

                        // Build request context for V8
                        let header_map: HashMap<String, String> = headers
                            .iter()
                            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                            .collect();
                        let headers_json =
                            serde_json::to_string(&header_map).unwrap_or_else(|_| "{}".to_string());
                        let cookies: HashMap<String, String> = headers
                            .get("cookie")
                            .and_then(|v| v.to_str().ok())
                            .map(|cookie_str| {
                                cookie_str
                                    .split(';')
                                    .filter_map(|pair| {
                                        let mut parts = pair.splitn(2, '=');
                                        Some((
                                            parts.next()?.trim().to_string(),
                                            parts.next()?.trim().to_string(),
                                        ))
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        let cookies_json =
                            serde_json::to_string(&cookies).unwrap_or_else(|_| "{}".to_string());

                        let fields_json =
                            serde_json::to_string(&fields).unwrap_or_else(|_| "[]".to_string());
                        match state
                            .isolate_pool
                            .execute(move |iso| {
                                let _ = iso.set_request_context(&headers_json, &cookies_json);
                                let r = iso.call_form_action("", &fields_json);
                                let _ = iso.clear_request_context();
                                r
                            })
                            .await
                        {
                            Ok(Ok(json_result)) => {
                                if let Ok(parsed) =
                                    serde_json::from_str::<serde_json::Value>(&json_result)
                                {
                                    if let Some(redirect_url) =
                                        parsed.get("redirect").and_then(|v| v.as_str())
                                    {
                                        let status = parsed
                                            .get("redirectStatus")
                                            .and_then(|v| v.as_u64())
                                            .unwrap_or(303)
                                            as u16;
                                        return Response::builder()
                                            .status(
                                                StatusCode::from_u16(status)
                                                    .unwrap_or(StatusCode::SEE_OTHER),
                                            )
                                            .header("Location", redirect_url)
                                            .body(Body::empty())
                                            .expect("response build");
                                    }
                                    if parsed
                                        .get("notFound")
                                        .and_then(|v| v.as_bool())
                                        .unwrap_or(false)
                                    {
                                        return Response::builder()
                                            .status(StatusCode::NOT_FOUND)
                                            .body(Body::from("404 - Not Found"))
                                            .expect("response build");
                                    }
                                    if parsed.get("error").is_some() {
                                        error!("Form action error: {json_result}");
                                        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                                    }
                                }
                                // Success — fall through to render the page with updated state
                            }
                            Ok(Err(e)) => {
                                error!("Form action error: {e}");
                                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                            }
                            Err(e) => {
                                error!("Form action pool error: {e}");
                                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                            }
                        }
                    }
                }
            }
            // Automatic static optimization: serve pre-rendered app route HTML
            if let Some(cached) = hot.prerendered_app.get(&app_match.route.pattern) {
                debug!(path, "Serving pre-rendered static app route");
                return Response::builder()
                    .header("content-type", "text/html; charset=utf-8")
                    .header("x-rex-render-mode", "static")
                    .body(Body::from(cached.html.clone()))
                    .expect("response build");
            }
            return render_app_route(state, hot, &app_match, path, uri, Some(headers)).await;
        }
    }

    // Try to match the route
    let route_match = match hot.route_trie.match_path(path) {
        Some(m) => m,
        None => {
            debug!(path, "No route matched");
            if hot.has_custom_404 {
                return render_error_page(state, hot, "404", StatusCode::NOT_FOUND, "{}").await;
            }
            return (
                StatusCode::NOT_FOUND,
                Html("404 - Page Not Found".to_string()),
            )
                .into_response();
        }
    };

    // Automatic static optimization: serve pre-rendered HTML without V8
    if let Some(page) = hot.prerendered.get(&route_match.route.pattern) {
        debug!(path, "Serving pre-rendered static page");
        return Response::builder()
            .header("content-type", "text/html; charset=utf-8")
            .header("x-rex-render-mode", "static")
            .body(Body::from(page.html.clone()))
            .expect("response build");
    }

    let route_key = route_match.route.module_name();
    let params = route_match.params.clone();

    // Look up data strategy from build manifest (detected at build time)
    let strategy = hot
        .manifest
        .pages
        .get(&route_match.route.pattern)
        .map(|p| &p.data_strategy)
        .cloned()
        .unwrap_or_default();

    // Fetch data props based on strategy
    let route_key_clone = route_key.clone();
    let gssp_result = match strategy {
        DataStrategy::None => {
            // No data function — skip V8 entirely
            Ok(Ok(r#"{"props":{}}"#.to_string()))
        }
        DataStrategy::GetStaticProps => {
            let ctx_json = serde_json::json!({ "params": params }).to_string();
            state
                .isolate_pool
                .execute(move |iso| iso.get_static_props(&route_key_clone, &ctx_json))
                .await
        }
        DataStrategy::GetServerSideProps => {
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

            let header_map: HashMap<String, String> = headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                .collect();

            let context = ServerSidePropsContext {
                params,
                query,
                resolved_url: path.to_string(),
                headers: header_map,
                cookies: HashMap::new(),
            };
            let context_json = serde_json::to_string(&context).expect("JSON serialization");

            state
                .isolate_pool
                .execute(move |iso| iso.get_server_side_props(&route_key_clone, &context_json))
                .await
        }
    };

    let props_json = match gssp_result {
        Ok(Ok(json)) => json,
        Ok(Err(e)) => {
            error!("GSSP error: {e}");
            if state.is_dev {
                return Html(dev_error_overlay(
                    "Server Props Error",
                    &e.to_string(),
                    None,
                ))
                .into_response();
            } else if hot.has_custom_error {
                let err_props = serde_json::json!({ "statusCode": 500 }).to_string();
                return render_error_page(
                    state,
                    hot,
                    "_error",
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &err_props,
                )
                .await;
            }
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html(format!("Server error: {e}")),
            )
                .into_response();
        }
        Err(e) => {
            error!("Isolate pool error: {e}");
            if state.is_dev {
                return Html(dev_error_overlay("Runtime Error", &e.to_string(), None))
                    .into_response();
            } else if hot.has_custom_error {
                let err_props = serde_json::json!({ "statusCode": 500 }).to_string();
                return render_error_page(
                    state,
                    hot,
                    "_error",
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &err_props,
                )
                .await;
            }
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html("Internal server error".to_string()),
            )
                .into_response();
        }
    };

    // Single-parse: parse once into Value, check redirect/notFound, extract props
    let parsed = match serde_json::from_str::<serde_json::Value>(&props_json) {
        Ok(val) => val,
        Err(_) => serde_json::json!({"props": {}}),
    };

    // Check for redirect
    if let Some(redirect) = parsed.get("redirect") {
        let destination = redirect
            .get("destination")
            .and_then(|d| d.as_str())
            .unwrap_or("/");
        let permanent = redirect
            .get("permanent")
            .and_then(|p| p.as_bool())
            .unwrap_or(false);
        let status_code = redirect
            .get("statusCode")
            .and_then(|s| s.as_u64())
            .unwrap_or(307) as u16;
        let status = if permanent { 301 } else { status_code };
        debug!(destination, status, "Redirecting");
        return Response::builder()
            .status(status)
            .header("Location", destination)
            .body(Body::empty())
            .expect("response build");
    }

    // Check for notFound
    if parsed
        .get("notFound")
        .and_then(|n| n.as_bool())
        .unwrap_or(false)
    {
        if hot.has_custom_404 {
            return render_error_page(state, hot, "404", StatusCode::NOT_FOUND, "{}").await;
        }
        return (StatusCode::NOT_FOUND, Html("404 - Not Found".to_string())).into_response();
    }

    // Extract props for rendering (already parsed, no re-parse needed)
    let render_props = match parsed.get("props") {
        Some(props) => serde_json::to_string(props).expect("JSON serialization"),
        None => "{}".to_string(),
    };

    // Look up client assets for this route
    let page_assets = hot.manifest.pages.get(&route_match.route.pattern);
    let client_scripts: Vec<String> = page_assets
        .map(|assets| vec![assets.js.clone()])
        .unwrap_or_default();

    // Collect CSS: global (from _app) + per-page
    let mut css_files = hot.manifest.global_css.clone();
    if let Some(assets) = page_assets {
        css_files.extend(assets.css.iter().cloned());
    }

    let manifest_json = hot.manifest_json.clone();
    let shared_chunks = hot.manifest.shared_chunks.clone();
    let app_script = hot.manifest.app_script.clone();
    let is_dev = state.is_dev;

    // Build the HTML head shell: doctype, <head> with CSS + JS modulepreload hints, opening <body>.
    // This is flushed to the browser immediately so it can start fetching resources
    // while V8 renders the page body.
    let shell = assemble_head_shell(
        &css_files,
        &hot.manifest.css_contents,
        &shared_chunks,
        app_script.as_deref(),
        &client_scripts,
        hot.document_descriptor.as_ref(),
        &hot.manifest.font_preloads,
        None,
    );

    // Stream response in two chunks: shell (immediate) → render + tail (after V8)
    let state_clone = state.clone();
    let route_key_clone = route_key.clone();
    let render_props_clone = render_props.clone();

    let shell_chunk = stream::once(async { Ok::<_, std::convert::Infallible>(shell) });

    let tail_chunk = stream::once(async move {
        let ssr_result = state_clone
            .isolate_pool
            .execute(move |iso| iso.render_page(&route_key_clone, &render_props_clone))
            .await;

        let (body_html, head_html) = match ssr_result {
            Ok(Ok(r)) => (r.body, r.head),
            Ok(Err(e)) => {
                error!("SSR render error: {e}");
                let msg = e.to_string().replace('<', "&lt;").replace('>', "&gt;");
                if is_dev {
                    (format!("<pre style=\"padding:20px;color:#e63946;font-family:monospace\">SSR Error: {msg}</pre>"), String::new())
                } else {
                    ("<h1>Internal Server Error</h1>".to_string(), String::new())
                }
            }
            Err(e) => {
                error!("Isolate pool error: {e}");
                ("<h1>Internal Server Error</h1>".to_string(), String::new())
            }
        };

        let tail = assemble_body_tail(
            &body_html,
            &head_html,
            &render_props,
            &client_scripts,
            app_script.as_deref(),
            is_dev,
            Some(&manifest_json),
            false,
        );

        Ok::<_, std::convert::Infallible>(tail)
    });

    let body = Body::from_stream(shell_chunk.chain(tail_chunk));

    Response::builder()
        .header("content-type", "text/html; charset=utf-8")
        .body(body)
        .expect("response build")
}

/// Handle an app router route handler (route.ts) request.
///
/// Dispatches to the V8 runtime which calls the correct HTTP method export
/// (GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS) on the route module.
pub(super) async fn handle_app_route_handler(
    state: &Arc<AppState>,
    route_match: &rex_core::RouteMatch,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: &axum::body::Bytes,
) -> Response {
    let route_pattern = route_match.route.pattern.clone();
    let params = route_match.params.clone();

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
        serde_json::from_slice::<serde_json::Value>(body).unwrap_or(serde_json::Value::Null)
    } else if !body.is_empty() {
        serde_json::Value::String(String::from_utf8_lossy(body).into_owned())
    } else {
        serde_json::Value::Null
    };

    let req_data = serde_json::json!({
        "method": method.as_str(),
        "url": uri.path(),
        "headers": headers.iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect::<HashMap<String, String>>(),
        "query": query,
        "body": body_value,
        "params": params,
    });
    let req_json = serde_json::to_string(&req_data).expect("JSON serialization");

    let result = state
        .isolate_pool
        .execute(move |iso| iso.call_app_route_handler(&route_pattern, &req_json))
        .await;

    match result {
        Ok(Ok(json)) => {
            let api_res: super::api::ApiResponse = match serde_json::from_str(&json) {
                Ok(r) => r,
                Err(e) => {
                    error!("Failed to parse app route handler response: {e}");
                    return (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response();
                }
            };

            let status = StatusCode::from_u16(api_res.status_code()).unwrap_or(StatusCode::OK);
            let mut builder = Response::builder().status(status);
            for (k, v) in api_res.headers() {
                builder = builder.header(k.as_str(), v.as_str());
            }
            builder
                .body(Body::from(api_res.body().to_string()))
                .expect("response build")
        }
        Ok(Err(e)) => {
            error!("App route handler V8 error: {e}");
            if state.is_dev {
                return axum::response::Html(dev_error_overlay(
                    "Route Handler Error",
                    &e.to_string(),
                    None,
                ))
                .into_response();
            }
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Route handler error: {e}"),
            )
                .into_response()
        }
        Err(e) => {
            error!("App route handler pool error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[cfg(test)]
#[path = "page_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "app_route_tests.rs"]
mod app_route_tests;
