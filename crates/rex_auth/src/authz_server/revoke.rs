use crate::AuthServer;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use std::collections::HashMap;
use std::sync::Arc;

/// POST /_rex/auth/revoke — Token Revocation (RFC 7009).
///
/// Per RFC 7009, this endpoint ALWAYS returns 200, even for invalid tokens.
pub async fn revoke_handler(
    State(auth): State<Arc<AuthServer>>,
    axum::Form(form): axum::Form<HashMap<String, String>>,
) -> Response {
    let token = match form.get("token") {
        Some(t) => t,
        None => {
            // Even missing token gets 200 per RFC 7009
            return ok_response();
        }
    };

    if let Some(store) = &auth.store {
        // Try to revoke as refresh token
        let _ = store.revoke_refresh_token(token);
        // Access tokens (JWTs) are stateless — can't truly revoke them,
        // but we accept the request gracefully.
    }

    ok_response()
}

fn ok_response() -> Response {
    Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .header("Cache-Control", "no-store")
        .body(axum::body::Body::from("{}"))
        .unwrap_or_else(|_| {
            (axum::http::StatusCode::OK, "{}").into_response()
        })
}
