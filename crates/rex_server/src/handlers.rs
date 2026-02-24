use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Response};
use rex_core::{ServerSidePropsContext, ServerSidePropsResult};
use rex_router::RouteTrie;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, error, info};

use crate::document::assemble_document;

/// Shared application state
pub struct AppState {
    pub route_trie: RouteTrie,
    pub isolate_pool: rex_v8::IsolatePool,
    pub manifest: rex_build::AssetManifest,
    pub build_id: String,
    pub is_dev: bool,
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
            return (StatusCode::INTERNAL_SERVER_ERROR, Html(format!("Server error: {e}"))).into_response();
        }
        Err(e) => {
            error!("Isolate pool error: {e}");
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

    let ssr_html = match ssr_result {
        Ok(Ok(html)) => html,
        Ok(Err(e)) => {
            error!("SSR render error: {e}");
            if state.is_dev {
                format!("<div style=\"color:red;font-family:monospace;padding:20px;\"><h2>SSR Error</h2><pre>{e}</pre></div>")
            } else {
                return (StatusCode::INTERNAL_SERVER_ERROR, Html("Internal server error".to_string())).into_response();
            }
        }
        Err(e) => {
            error!("Isolate pool error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, Html("Internal server error".to_string())).into_response();
        }
    };

    // Look up client scripts for this route
    let client_scripts: Vec<String> = state
        .manifest
        .pages
        .get(&route_match.route.pattern)
        .map(|assets| vec![assets.js.clone()])
        .unwrap_or_default();

    let document = assemble_document(
        &ssr_html,
        &render_props,
        &state.manifest.vendor_scripts,
        &client_scripts,
        &state.build_id,
        state.is_dev,
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
