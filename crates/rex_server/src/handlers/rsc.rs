use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::error;

use super::AppState;
use crate::state::snapshot;

/// RSC flight data endpoint: GET /_rex/rsc/{buildId}/{path}
/// Returns flight data as text/x-component for client-side RSC navigation.
pub async fn rsc_handler(
    State(state): State<Arc<AppState>>,
    Path((build_id, page_path)): Path<(String, String)>,
    uri: Uri,
) -> Response {
    let hot = snapshot(&state);

    // Build ID mismatch = stale client
    if build_id != hot.build_id {
        return StatusCode::NOT_FOUND.into_response();
    }

    let app_route_trie = match &hot.app_route_trie {
        Some(trie) => trie,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    let path = format!("/{page_path}");
    let route_match = match app_route_trie.match_path(&path) {
        Some(m) => m,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    let route_key = &route_match.route.pattern;
    let params = route_match.params.clone();

    // Pass both route params and query string to the RSC render
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
    let route_key_owned = route_key.to_string();

    let result = state
        .isolate_pool
        .execute(move |iso| iso.render_rsc_flight(&route_key_owned, &props_json))
        .await;

    match result {
        Ok(Ok(flight_data)) => Response::builder()
            .header("Content-Type", "text/x-component")
            .header("Cache-Control", "no-cache")
            .body(Body::from(flight_data))
            .expect("response build"),
        Ok(Err(e)) => {
            error!("RSC flight render error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
        Err(e) => {
            error!("RSC pool error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
