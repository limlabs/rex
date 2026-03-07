use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use std::sync::Arc;
use tracing::error;

use super::AppState;
use crate::state::snapshot;

/// Server action handler: POST /_rex/action/{build_id}/{action_id}
///
/// Dispatches a server function call from the client. The request body
/// is a JSON array of arguments. Returns `{ result: ... }` or `{ error: ... }`.
pub async fn server_action_handler(
    State(state): State<Arc<AppState>>,
    Path((build_id, action_id)): Path<(String, String)>,
    body: axum::body::Bytes,
) -> Response {
    let hot = snapshot(&state);

    // Build ID mismatch = stale client
    if build_id != hot.build_id {
        return StatusCode::NOT_FOUND.into_response();
    }

    let args_json = match std::str::from_utf8(&body) {
        Ok(s) => s.to_string(),
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "Invalid UTF-8 body").into_response();
        }
    };

    let action_id_owned = action_id.clone();

    let result = state
        .isolate_pool
        .execute(move |iso| iso.call_server_action(&action_id_owned, &args_json))
        .await;

    match result {
        Ok(Ok(json_result)) => Response::builder()
            .header("Content-Type", "application/json")
            .body(Body::from(json_result))
            .expect("response build"),
        Ok(Err(e)) => {
            error!("Server action error: {e}");
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "error": e.to_string() }).to_string(),
                ))
                .expect("response build")
        }
        Err(e) => {
            error!("Server action pool error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
