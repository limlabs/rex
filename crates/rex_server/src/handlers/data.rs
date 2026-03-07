use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use rex_core::{DataStrategy, ServerSidePropsContext};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::error;

use super::AppState;
use crate::state::snapshot;

/// Data endpoint: GET /_rex/data/{buildId}/{path}.json
/// Returns GSSP result as JSON for client-side navigation
pub async fn data_handler(
    State(state): State<Arc<AppState>>,
    Path((build_id, page_path)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let hot = snapshot(&state);

    // Build ID mismatch = stale client
    if build_id != hot.build_id {
        return StatusCode::NOT_FOUND.into_response();
    }

    let path = format!("/{}", page_path.trim_end_matches(".json"));

    let route_match = match hot.route_trie.match_path(&path) {
        Some(m) => m,
        None => return StatusCode::NOT_FOUND.into_response(),
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
            let header_map: HashMap<String, String> = headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                .collect();
            let context = ServerSidePropsContext {
                params,
                query: HashMap::new(),
                resolved_url: path,
                headers: header_map,
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
        Ok(Ok(json)) => Response::builder()
            .header("Content-Type", "application/json")
            .body(Body::from(json))
            .expect("response build"),
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
