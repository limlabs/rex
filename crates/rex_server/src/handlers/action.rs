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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::server_action_handler;
    use crate::handlers::test_support::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::post;
    use axum::Router;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_server_action_stale_build_id() {
        let app = TestAppBuilder::new()
            .extra_bundle(TEST_ACTION_RUNTIME)
            .custom_router(|state| {
                Router::new()
                    .route(
                        "/_rex/action/{build_id}/{action_id}",
                        post(server_action_handler),
                    )
                    .with_state(state)
            })
            .build();

        let resp = app
            .oneshot(
                Request::post("/_rex/action/wrong-build-id/test_action_id")
                    .header("Content-Type", "application/json")
                    .body(Body::from("[42]"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_server_action_success() {
        let app = TestAppBuilder::new()
            .extra_bundle(TEST_ACTION_RUNTIME)
            .custom_router(|state| {
                Router::new()
                    .route(
                        "/_rex/action/{build_id}/{action_id}",
                        post(server_action_handler),
                    )
                    .with_state(state)
            })
            .build();

        let resp = app
            .oneshot(
                Request::post("/_rex/action/test-build-id/test_action_id")
                    .header("Content-Type", "application/json")
                    .body(Body::from("[42]"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp.into_body()).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["result"], 43);
    }

    #[tokio::test]
    async fn test_server_action_not_found() {
        let app = TestAppBuilder::new()
            .extra_bundle(TEST_ACTION_RUNTIME)
            .custom_router(|state| {
                Router::new()
                    .route(
                        "/_rex/action/{build_id}/{action_id}",
                        post(server_action_handler),
                    )
                    .with_state(state)
            })
            .build();

        let resp = app
            .oneshot(
                Request::post("/_rex/action/test-build-id/nonexistent")
                    .header("Content-Type", "application/json")
                    .body(Body::from("[]"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp.into_body()).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed["error"].as_str().unwrap().contains("not found"));
    }

    #[tokio::test]
    async fn test_server_action_invalid_utf8() {
        let app = TestAppBuilder::new()
            .extra_bundle(TEST_ACTION_RUNTIME)
            .custom_router(|state| {
                Router::new()
                    .route(
                        "/_rex/action/{build_id}/{action_id}",
                        post(server_action_handler),
                    )
                    .with_state(state)
            })
            .build();

        let resp = app
            .oneshot(
                Request::post("/_rex/action/test-build-id/test_action_id")
                    .header("Content-Type", "application/json")
                    .body(Body::from(vec![0xFF, 0xFE]))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
