use crate::AuthServer;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use std::sync::Arc;

/// GET /_rex/auth/jwks — JSON Web Key Set endpoint.
///
/// Returns public keys for JWT verification. Never exposes private key components.
pub async fn jwks_handler(State(auth): State<Arc<AuthServer>>) -> Response {
    let key_manager = match &auth.key_manager {
        Some(km) => km,
        None => {
            return Response::builder()
                .status(200)
                .header("Content-Type", "application/json")
                .header("Cache-Control", "public, max-age=3600")
                .body(axum::body::Body::from(r#"{"keys":[]}"#))
                .unwrap_or_else(|_| {
                    (axum::http::StatusCode::OK, r#"{"keys":[]}"#).into_response()
                });
        }
    };

    let keys = key_manager.all_jwks();

    let jwks = serde_json::json!({ "keys": keys });

    Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .header("Cache-Control", "public, max-age=3600")
        .body(axum::body::Body::from(
            serde_json::to_string(&jwks).unwrap_or_else(|_| r#"{"keys":[]}"#.to_string()),
        ))
        .unwrap_or_else(|_| {
            (axum::http::StatusCode::OK, r#"{"keys":[]}"#).into_response()
        })
}
