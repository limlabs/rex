use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Response};
use futures::stream::{self, StreamExt};
use rex_core::{DataStrategy, MiddlewareAction, ServerSidePropsContext};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, error, info};

use crate::document::{
    assemble_body_tail, assemble_head_shell, assemble_rsc_body_tail, assemble_rsc_head_shell,
};

use super::{
    check_redirects, check_rewrites, collect_headers, dev_error_overlay, execute_middleware,
    render_error_page, should_run_middleware, AppState, HotState,
};
use crate::state::snapshot;

/// Main page handler - catches all routes and performs SSR
pub async fn page_handler(
    State(state): State<Arc<AppState>>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let path = uri.path();
    info!(path, "Handling page request");

    let hot = snapshot(&state);

    // Run middleware before anything else
    let mut mw_response_headers: Vec<(String, String)> = Vec::new();
    if should_run_middleware(path, &hot) {
        let header_map: HashMap<String, String> = headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        match execute_middleware(&state, path, "GET", &header_map).await {
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
                        let mut response =
                            page_handler_inner(&state, &hot, &rewrite_path, &uri, &headers).await;
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

    let mut response = page_handler_inner(&state, &hot, path, &uri, &headers).await;

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

/// Render an app/ route using RSC with streaming (head shell + body tail).
pub(super) async fn render_app_route(
    state: &Arc<AppState>,
    hot: &Arc<HotState>,
    route_match: &rex_core::RouteMatch,
    _path: &str,
    uri: &Uri,
) -> Response {
    let route_key = route_match.route.pattern.clone();
    let params = route_match.params.clone();
    let search_params: HashMap<String, String> = uri
        .query()
        .map(|q| {
            url::form_urlencoded::parse(q.as_bytes())
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let props_json =
        serde_json::json!({ "params": params, "searchParams": search_params }).to_string();

    // Look up client chunks for this app route
    let app_assets = hot.manifest.app_routes.get(&route_key);
    let client_chunks: Vec<String> = app_assets
        .map(|a| a.client_chunks.clone())
        .unwrap_or_default();

    // Serialize client reference manifest
    let client_manifest_json = hot
        .manifest
        .client_reference_manifest
        .as_ref()
        .and_then(|m| serde_json::to_string(m).ok())
        .unwrap_or_else(|| "{}".to_string());

    let is_dev = state.is_dev;
    let manifest_json = hot.manifest_json.clone();

    // Flush head shell immediately so browser starts fetching resources
    let shell = assemble_rsc_head_shell(&client_chunks, &client_manifest_json);

    let shell_chunk = stream::once(async { Ok::<_, std::convert::Infallible>(shell) });

    let state_clone = state.clone();
    let route_key_clone = route_key.clone();
    let props_clone = props_json.clone();
    let client_chunks_clone = client_chunks.clone();
    let client_manifest_json_clone = client_manifest_json.clone();

    let tail_chunk = stream::once(async move {
        let rsc_result = state_clone
            .isolate_pool
            .execute(move |iso| iso.render_rsc_to_html(&route_key_clone, &props_clone))
            .await;

        let (body_html, head_html, flight_data) = match rsc_result {
            Ok(Ok(r)) => (r.body, r.head, r.flight),
            Ok(Err(e)) => {
                error!("RSC render error: {e}");
                let msg = e.to_string().replace('<', "&lt;").replace('>', "&gt;");
                if is_dev {
                    (
                        format!("<pre style=\"padding:20px;color:#e63946;font-family:monospace\">RSC Error: {msg}</pre>"),
                        String::new(),
                        String::new(),
                    )
                } else {
                    (
                        "<h1>Internal Server Error</h1>".to_string(),
                        String::new(),
                        String::new(),
                    )
                }
            }
            Err(e) => {
                error!("RSC pool error: {e}");
                (
                    "<h1>Internal Server Error</h1>".to_string(),
                    String::new(),
                    String::new(),
                )
            }
        };

        let tail = assemble_rsc_body_tail(
            &body_html,
            &head_html,
            &flight_data,
            &client_chunks_clone,
            &client_manifest_json_clone,
            is_dev,
            Some(&manifest_json),
        );

        Ok::<_, std::convert::Infallible>(tail)
    });

    let body = Body::from_stream(shell_chunk.chain(tail_chunk));

    Response::builder()
        .header("content-type", "text/html; charset=utf-8")
        .body(body)
        .expect("response build")
}

pub(super) async fn page_handler_inner(
    state: &Arc<AppState>,
    hot: &Arc<HotState>,
    path: &str,
    uri: &Uri,
    headers: &HeaderMap,
) -> Response {
    // Check app/ routes first (RSC)
    if let Some(ref app_trie) = hot.app_route_trie {
        if let Some(app_match) = app_trie.match_path(path) {
            return render_app_route(state, hot, &app_match, path, uri).await;
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
        );

        Ok::<_, std::convert::Infallible>(tail)
    });

    let body = Body::from_stream(shell_chunk.chain(tail_chunk));

    Response::builder()
        .header("content-type", "text/html; charset=utf-8")
        .body(body)
        .expect("response build")
}

#[cfg(test)]
#[path = "page_tests.rs"]
mod tests;
