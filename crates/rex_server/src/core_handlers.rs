use rex_core::{DataStrategy, MiddlewareAction, ServerSidePropsContext};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, error, info};

use crate::document::{assemble_document, DocumentParams};
use crate::state::{AppState, HotState};

use super::core::{
    check_redirects, check_rewrites, collect_custom_headers, execute_middleware, RexBody,
    RexRequest, RexResponse,
};

/// Check if middleware should run for the given path.
pub fn should_run_middleware(path: &str, hot: &HotState) -> bool {
    super::core::should_run_middleware(path, hot)
}

/// Render a custom error page (404 or _error) via SSR, returning a full HTML RexResponse.
async fn render_error_page(
    state: &Arc<AppState>,
    hot: &Arc<HotState>,
    page_key: &str,
    status: u16,
    props: &str,
) -> RexResponse {
    let key = page_key.to_string();
    let props_clone = props.to_string();
    let ssr_result = state
        .isolate_pool
        .execute(move |iso| iso.render_page(&key, &props_clone))
        .await;

    let render = match ssr_result {
        Ok(Ok(r)) => r,
        _ => return RexResponse::text(status, format!("{status} Error")),
    };

    let document = assemble_document(&DocumentParams {
        ssr_html: &render.body,
        head_html: &render.head,
        props_json: props,
        client_scripts: &[],
        css_files: &hot.manifest.global_css,
        css_contents: &hot.manifest.css_contents,
        app_script: hot.manifest.app_script.as_deref(),
        is_dev: state.is_dev,
        doc_descriptor: hot.document_descriptor.as_ref(),
        manifest_json: Some(&hot.manifest_json),
        font_preloads: &hot.manifest.font_preloads,
    });

    RexResponse::html(status, document)
}

/// Generate a full-page error overlay for dev mode.
fn dev_error_overlay(title: &str, message: &str, _file: Option<&str>) -> String {
    let escaped_message = message
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        r#"<!DOCTYPE html><html><head><meta charset="utf-8"/><title>{title}</title></head><body><h1>{title}</h1><pre>{escaped_message}</pre></body></html>"#,
    )
}

/// Core page handler — framework-agnostic. Returns full (non-streaming) HTML.
pub async fn handle_page(
    state: &Arc<AppState>,
    hot: &Arc<HotState>,
    req: &RexRequest,
) -> RexResponse {
    let path = &req.path;
    info!(path = path.as_str(), "Handling page request (core)");

    // Run middleware before anything else
    let mut mw_response_headers: Vec<(String, String)> = Vec::new();
    if should_run_middleware(path, hot) {
        match execute_middleware(state, path, &req.method, &req.headers).await {
            Ok(Some(mw)) => match mw.action {
                MiddlewareAction::Redirect => {
                    let url = mw.url.as_deref().unwrap_or("/");
                    return RexResponse::redirect(mw.status, url);
                }
                MiddlewareAction::Rewrite => {
                    if let Some(url) = &mw.url {
                        let rewrite_path = if let Ok(parsed) = url::Url::parse(url) {
                            parsed.path().to_string()
                        } else {
                            url.clone()
                        };
                        mw_response_headers = mw.response_headers.into_iter().collect();
                        let custom_headers =
                            collect_custom_headers(&rewrite_path, &hot.project_config);
                        let mut response = handle_page_inner(state, hot, &rewrite_path, req).await;
                        for (key, value) in custom_headers.into_iter().chain(mw_response_headers) {
                            response.headers.push((key, value));
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
                    return RexResponse::html(500, dev_error_overlay("Middleware Error", &e, None));
                }
                return RexResponse::internal_error();
            }
        }
    }

    // Check redirect rules first
    if let Some(redirect) = check_redirects(path, &hot.project_config) {
        return redirect;
    }

    // Check rewrite rules
    let effective_path;
    let path = if let Some(rewritten) = check_rewrites(path, &hot.project_config) {
        effective_path = rewritten;
        &effective_path
    } else {
        path.as_str()
    };

    // Collect custom headers
    let custom_headers = collect_custom_headers(path, &hot.project_config);

    let mut response = handle_page_inner(state, hot, path, req).await;

    // Apply custom headers + middleware response headers
    for (key, value) in custom_headers.into_iter().chain(mw_response_headers) {
        response.headers.push((key, value));
    }

    response
}

async fn handle_page_inner(
    state: &Arc<AppState>,
    hot: &Arc<HotState>,
    path: &str,
    req: &RexRequest,
) -> RexResponse {
    // Try to match the route
    let route_match = match hot.route_trie.match_path(path) {
        Some(m) => m,
        None => {
            debug!(path, "No route matched");
            if hot.has_custom_404 {
                return render_error_page(state, hot, "404", 404, "{}").await;
            }
            return RexResponse::html(404, "404 - Page Not Found".to_string());
        }
    };

    // Automatic static optimization: serve pre-rendered HTML without V8
    if let Some(page) = hot.prerendered.get(&route_match.route.pattern) {
        debug!(path, "Serving pre-rendered static page (core)");
        let mut resp = RexResponse::html(200, page.html.clone());
        resp.headers
            .push(("x-rex-render-mode".to_string(), "static".to_string()));
        return resp;
    }

    let route_key = route_match.route.module_name();
    let params = route_match.params.clone();

    // Look up data strategy from build manifest
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
        DataStrategy::None => Ok(Ok(r#"{"props":{}}"#.to_string())),
        DataStrategy::GetStaticProps => {
            let ctx_json = serde_json::json!({ "params": params }).to_string();
            state
                .isolate_pool
                .execute(move |iso| iso.get_static_props(&route_key_clone, &ctx_json))
                .await
        }
        DataStrategy::GetServerSideProps => {
            let query = req.query_params();
            let context = ServerSidePropsContext {
                params,
                query,
                resolved_url: path.to_string(),
                headers: req.headers.clone(),
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
                return RexResponse::html(
                    500,
                    dev_error_overlay("Server Props Error", &e.to_string(), None),
                );
            } else if hot.has_custom_error {
                let err_props = serde_json::json!({ "statusCode": 500 }).to_string();
                return render_error_page(state, hot, "_error", 500, &err_props).await;
            }
            return RexResponse::html(500, format!("Server error: {e}"));
        }
        Err(e) => {
            error!("Isolate pool error: {e}");
            if state.is_dev {
                return RexResponse::html(
                    500,
                    dev_error_overlay("Runtime Error", &e.to_string(), None),
                );
            } else if hot.has_custom_error {
                let err_props = serde_json::json!({ "statusCode": 500 }).to_string();
                return render_error_page(state, hot, "_error", 500, &err_props).await;
            }
            return RexResponse::html(500, "Internal server error".to_string());
        }
    };

    // Parse props to check redirect/notFound
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
        return RexResponse::redirect(status, destination);
    }

    // Check for notFound
    if parsed
        .get("notFound")
        .and_then(|n| n.as_bool())
        .unwrap_or(false)
    {
        if hot.has_custom_404 {
            return render_error_page(state, hot, "404", 404, "{}").await;
        }
        return RexResponse::html(404, "404 - Not Found".to_string());
    }

    // Extract props for rendering
    let render_props = match parsed.get("props") {
        Some(props) => serde_json::to_string(props).expect("JSON serialization"),
        None => "{}".to_string(),
    };

    // SSR render
    let route_key_clone = route_key.clone();
    let render_props_clone = render_props.clone();
    let ssr_result = state
        .isolate_pool
        .execute(move |iso| iso.render_page(&route_key_clone, &render_props_clone))
        .await;

    let (body_html, head_html) = match ssr_result {
        Ok(Ok(r)) => (r.body, r.head),
        Ok(Err(e)) => {
            error!("SSR render error: {e}");
            if state.is_dev {
                let msg = e.to_string().replace('<', "&lt;").replace('>', "&gt;");
                (
                    format!("<pre style=\"padding:20px;color:#e63946;font-family:monospace\">SSR Error: {msg}</pre>"),
                    String::new(),
                )
            } else {
                ("<h1>Internal Server Error</h1>".to_string(), String::new())
            }
        }
        Err(e) => {
            error!("Isolate pool error: {e}");
            ("<h1>Internal Server Error</h1>".to_string(), String::new())
        }
    };

    // Look up client assets
    let page_assets = hot.manifest.pages.get(&route_match.route.pattern);
    let client_scripts: Vec<String> = page_assets
        .map(|assets| vec![assets.js.clone()])
        .unwrap_or_default();

    let mut css_files = hot.manifest.global_css.clone();
    if let Some(assets) = page_assets {
        css_files.extend(assets.css.iter().cloned());
    }

    let document = assemble_document(&DocumentParams {
        ssr_html: &body_html,
        head_html: &head_html,
        props_json: &render_props,
        client_scripts: &client_scripts,
        css_files: &css_files,
        css_contents: &hot.manifest.css_contents,
        app_script: hot.manifest.app_script.as_deref(),
        is_dev: state.is_dev,
        doc_descriptor: hot.document_descriptor.as_ref(),
        manifest_json: Some(&hot.manifest_json),
        font_preloads: &hot.manifest.font_preloads,
    });

    RexResponse::html(200, document)
}

/// Core API handler — framework-agnostic.
pub async fn handle_api(
    state: &Arc<AppState>,
    hot: &Arc<HotState>,
    req: &RexRequest,
) -> RexResponse {
    let path = &req.path;
    info!(
        path = path.as_str(),
        method = req.method.as_str(),
        "Handling API request (core)"
    );

    // Run middleware before route matching
    if should_run_middleware(path, hot) {
        match execute_middleware(state, path, &req.method, &req.headers).await {
            Ok(Some(mw)) => match mw.action {
                MiddlewareAction::Redirect => {
                    let url = mw.url.as_deref().unwrap_or("/");
                    return RexResponse::redirect(mw.status, url);
                }
                MiddlewareAction::Rewrite | MiddlewareAction::Next => {
                    // Continue normally
                }
            },
            Ok(None) => {}
            Err(e) => {
                error!("Middleware error: {e}");
                return RexResponse::text(500, format!("Middleware error: {e}"));
            }
        }
    }

    let route_match = match hot.api_route_trie.match_path(path) {
        Some(m) => m,
        None => return RexResponse::not_found(),
    };

    let route_key = route_match.route.module_name();
    let query = req.query_params();

    // Parse body based on content-type
    let content_type = req
        .headers
        .get("content-type")
        .map(|s| s.as_str())
        .unwrap_or("");
    let body_value = if content_type.starts_with("application/json") {
        serde_json::from_slice::<serde_json::Value>(&req.body).unwrap_or(serde_json::Value::Null)
    } else if !req.body.is_empty() {
        serde_json::Value::String(String::from_utf8_lossy(&req.body).into_owned())
    } else {
        serde_json::Value::Null
    };

    let req_data = serde_json::json!({
        "method": req.method,
        "url": path,
        "headers": req.headers,
        "query": query,
        "body": body_value,
        "cookies": {},
    });
    let req_json = serde_json::to_string(&req_data).expect("JSON serialization");

    let result = state
        .isolate_pool
        .execute(move |iso| iso.call_api_handler(&route_key, &req_json))
        .await;

    #[derive(serde::Deserialize)]
    struct ApiResponse {
        #[serde(rename = "statusCode")]
        status_code: u16,
        #[serde(default)]
        headers: HashMap<String, String>,
        #[serde(default)]
        body: String,
    }

    match result {
        Ok(Ok(json)) => {
            let api_res: ApiResponse = match serde_json::from_str(&json) {
                Ok(r) => r,
                Err(e) => {
                    error!("Failed to parse API response: {e}");
                    return RexResponse::internal_error();
                }
            };
            let mut headers: Vec<(String, String)> = api_res.headers.into_iter().collect();
            // Ensure content-type is present
            if !headers
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
            {
                headers.push(("content-type".to_string(), "application/json".to_string()));
            }
            RexResponse {
                status: api_res.status_code,
                headers,
                body: RexBody::Full(api_res.body.into_bytes()),
            }
        }
        Ok(Err(e)) => {
            error!("API handler V8 error: {e}");
            RexResponse::text(500, format!("API error: {e}"))
        }
        Err(e) => {
            error!("API handler pool error: {e}");
            RexResponse::internal_error()
        }
    }
}

/// Core data handler — framework-agnostic.
pub async fn handle_data(
    state: &Arc<AppState>,
    hot: &Arc<HotState>,
    build_id: &str,
    page_path: &str,
    req: &RexRequest,
) -> RexResponse {
    // Build ID mismatch = stale client
    if build_id != hot.build_id {
        return RexResponse::not_found();
    }

    let path = format!("/{}", page_path.trim_end_matches(".json"));

    let route_match = match hot.route_trie.match_path(&path) {
        Some(m) => m,
        None => return RexResponse::not_found(),
    };

    let route_key = route_match.route.module_name();
    let params = route_match.params.clone();

    let strategy = hot
        .manifest
        .pages
        .get(&route_match.route.pattern)
        .map(|p| &p.data_strategy)
        .cloned()
        .unwrap_or_default();

    let result = match strategy {
        DataStrategy::None => Ok(Ok(r#"{"props":{}}"#.to_string())),
        DataStrategy::GetStaticProps => {
            let ctx_json = serde_json::json!({ "params": params }).to_string();
            state
                .isolate_pool
                .execute(move |iso| iso.get_static_props(&route_key, &ctx_json))
                .await
        }
        DataStrategy::GetServerSideProps => {
            let context = ServerSidePropsContext {
                params,
                query: HashMap::new(),
                resolved_url: path,
                headers: req.headers.clone(),
                cookies: HashMap::new(),
            };
            let context_json = serde_json::to_string(&context).expect("JSON serialization");
            state
                .isolate_pool
                .execute(move |iso| iso.get_server_side_props(&route_key, &context_json))
                .await
        }
    };

    match result {
        Ok(Ok(json)) => RexResponse::json(200, json),
        Ok(Err(e)) => {
            error!("Data endpoint GSSP error: {e}");
            RexResponse::text(500, e.to_string())
        }
        Err(e) => {
            error!("Data endpoint pool error: {e}");
            RexResponse::internal_error()
        }
    }
}

/// Core image handler — framework-agnostic.
pub async fn handle_image(state: &Arc<AppState>, req: &RexRequest) -> RexResponse {
    let query = req.query_params();

    let url = match query.get("url") {
        Some(u) => u.clone(),
        None => return RexResponse::text(400, "missing url param".to_string()),
    };
    let w: u32 = match query.get("w").and_then(|v| v.parse().ok()) {
        Some(w) => w,
        None => return RexResponse::text(400, "missing or invalid w param".to_string()),
    };
    let q: u8 = query.get("q").and_then(|v| v.parse().ok()).unwrap_or(75);
    let f = query.get("f").cloned();

    if !(16..=4096).contains(&w) {
        return RexResponse::text(400, "width must be 16-4096".to_string());
    }

    let explicit_format = match f.as_deref() {
        Some("webp") => Some(rex_image::OutputFormat::WebP),
        Some("jpeg") | Some("jpg") => Some(rex_image::OutputFormat::Jpeg),
        Some("png") => Some(rex_image::OutputFormat::Png),
        Some(_) => return RexResponse::text(400, "unsupported format".to_string()),
        None => None,
    };
    let format = explicit_format.unwrap_or_else(|| {
        if url.ends_with(".png") {
            rex_image::OutputFormat::Png
        } else {
            let accept = req.headers.get("accept").map(|s| s.as_str()).unwrap_or("");
            rex_image::negotiate_format(accept)
        }
    });

    let fmt_ext = format.extension();
    if let Some(data) = state.image_cache.get(&url, w, q, fmt_ext) {
        return RexResponse {
            status: 200,
            headers: vec![
                (
                    "content-type".to_string(),
                    format.content_type().to_string(),
                ),
                (
                    "cache-control".to_string(),
                    "public, max-age=31536000, immutable".to_string(),
                ),
            ],
            body: RexBody::Full(data),
        };
    }

    let url_path = url.trim_start_matches('/');

    // Reject paths with traversal components before touching the filesystem
    if url_path.split('/').any(|seg| seg == ".." || seg == ".") {
        return RexResponse::text(400, "invalid path".to_string());
    }

    let file_path = state.project_root.join("public").join(url_path);

    // Prevent path traversal — both canonicalizations must succeed
    // and the resolved path must be inside public/
    let public_dir = state.project_root.join("public");
    let public_canonical = match public_dir.canonicalize() {
        Ok(p) => p,
        Err(_) => return RexResponse::text(404, "image not found".to_string()),
    };
    let canonical = match file_path.canonicalize() {
        Ok(p) => p,
        Err(_) => return RexResponse::text(404, "image not found".to_string()),
    };
    if !canonical.starts_with(&public_canonical) {
        return RexResponse::text(400, "invalid path".to_string());
    }

    let src_bytes = match std::fs::read(&canonical) {
        Ok(data) => data,
        Err(_) => return RexResponse::text(404, "image not found".to_string()),
    };

    let params = rex_image::OptimizeParams {
        width: w,
        quality: q,
        format,
    };

    match rex_image::optimize(&src_bytes, &params) {
        Ok(optimized) => {
            if let Err(e) = state.image_cache.put(&url, w, q, fmt_ext, &optimized) {
                debug!("image cache write failed: {e}");
            }
            RexResponse {
                status: 200,
                headers: vec![
                    (
                        "content-type".to_string(),
                        format.content_type().to_string(),
                    ),
                    (
                        "cache-control".to_string(),
                        "public, max-age=31536000, immutable".to_string(),
                    ),
                ],
                body: RexBody::Full(optimized),
            }
        }
        Err(e) => {
            error!("image optimization failed: {e}");
            RexResponse::text(500, e.to_string())
        }
    }
}
