use crate::AuthServer;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use std::collections::HashMap;
use std::sync::Arc;

/// POST /_rex/auth/token — Token endpoint.
///
/// Supports `grant_type=authorization_code` and `grant_type=refresh_token`.
pub async fn token_handler(
    State(auth): State<Arc<AuthServer>>,
    axum::Form(form): axum::Form<HashMap<String, String>>,
) -> Response {
    let grant_type = match form.get("grant_type") {
        Some(gt) => gt.as_str(),
        None => return token_error(400, "invalid_request", "Missing grant_type"),
    };

    match grant_type {
        "authorization_code" => handle_authorization_code(&auth, &form).await,
        "refresh_token" => handle_refresh_token(&auth, &form).await,
        other => token_error(
            400,
            "unsupported_grant_type",
            &format!("Unsupported grant_type: {other}"),
        ),
    }
}

async fn handle_authorization_code(auth: &AuthServer, form: &HashMap<String, String>) -> Response {
    let code = match form.get("code") {
        Some(c) => c,
        None => return token_error(400, "invalid_request", "Missing code"),
    };

    let client_id = match form.get("client_id") {
        Some(id) => id,
        None => return token_error(400, "invalid_request", "Missing client_id"),
    };

    let redirect_uri = match form.get("redirect_uri") {
        Some(uri) => uri,
        None => return token_error(400, "invalid_request", "Missing redirect_uri"),
    };

    let code_verifier = match form.get("code_verifier") {
        Some(v) => v,
        None => {
            return token_error(
                400,
                "invalid_request",
                "Missing code_verifier (PKCE required)",
            )
        }
    };

    let store = match &auth.store {
        Some(s) => s,
        None => return token_error(500, "server_error", "Auth store not initialized"),
    };

    // Consume the auth code (one-time use)
    let auth_code = match store.consume_auth_code(code) {
        Ok(Some(c)) => c,
        Ok(None) => {
            return token_error(
                400,
                "invalid_grant",
                "Invalid or expired authorization code",
            )
        }
        Err(e) => {
            tracing::error!("Store error consuming auth code: {e}");
            return token_error(500, "server_error", "Internal error");
        }
    };

    // Validate client_id matches
    if auth_code.client_id != *client_id {
        return token_error(400, "invalid_grant", "client_id mismatch");
    }

    // Validate redirect_uri matches
    if auth_code.redirect_uri != *redirect_uri {
        return token_error(400, "invalid_grant", "redirect_uri mismatch");
    }

    // Verify PKCE
    if !crate::pkce::verify_pkce_s256(code_verifier, &auth_code.code_challenge) {
        return token_error(400, "invalid_grant", "PKCE verification failed");
    }

    // Check code expiry (10 min)
    let now = now_secs();
    if now - auth_code.created_at > 600 {
        return token_error(400, "invalid_grant", "Authorization code expired");
    }

    // Issue tokens
    issue_tokens(
        auth,
        store,
        &auth_code.subject,
        client_id,
        redirect_uri,
        &auth_code.scope,
    )
    .await
}

async fn handle_refresh_token(auth: &AuthServer, form: &HashMap<String, String>) -> Response {
    let refresh_token = match form.get("refresh_token") {
        Some(t) => t,
        None => return token_error(400, "invalid_request", "Missing refresh_token"),
    };

    let client_id = match form.get("client_id") {
        Some(id) => id,
        None => return token_error(400, "invalid_request", "Missing client_id"),
    };

    let store = match &auth.store {
        Some(s) => s,
        None => return token_error(500, "server_error", "Auth store not initialized"),
    };

    // Look up and validate refresh token
    let stored = match store.get_refresh_token(refresh_token) {
        Ok(Some(t)) => t,
        Ok(None) => return token_error(400, "invalid_grant", "Invalid refresh token"),
        Err(e) => {
            tracing::error!("Store error: {e}");
            return token_error(500, "server_error", "Internal error");
        }
    };

    // Validate client_id matches
    if stored.client_id != *client_id {
        return token_error(400, "invalid_grant", "client_id mismatch");
    }

    // Validate redirect_uri matches the original grant (if provided)
    if let Some(redirect_uri) = form.get("redirect_uri") {
        if stored.redirect_uri != *redirect_uri {
            return token_error(400, "invalid_grant", "redirect_uri mismatch");
        }
    }

    // Check expiry
    let now = now_secs();
    if stored.expires_at <= now {
        // Revoke expired token
        let _ = store.revoke_refresh_token(refresh_token);
        return token_error(400, "invalid_grant", "Refresh token expired");
    }

    // Revoke old refresh token (rotation)
    let _ = store.revoke_refresh_token(refresh_token);

    // Issue new tokens with the same redirect_uri binding
    issue_tokens(
        auth,
        store,
        &stored.subject,
        client_id,
        &stored.redirect_uri,
        &stored.scope,
    )
    .await
}

async fn issue_tokens(
    auth: &AuthServer,
    store: &crate::store::FileStore,
    subject: &str,
    client_id: &str,
    redirect_uri: &str,
    scope: &str,
) -> Response {
    let now = now_secs();

    let key_manager = match &auth.key_manager {
        Some(km) => km,
        None => return token_error(500, "server_error", "Key manager not initialized"),
    };

    // Create access token (JWT)
    let claims = crate::jwt::AccessTokenClaims {
        iss: auth.issuer().to_string(),
        sub: subject.to_string(),
        aud: client_id.to_string(),
        exp: now + auth.config.mcp.access_token_ttl,
        iat: now,
        jti: uuid::Uuid::new_v4().to_string(),
        scope: scope.to_string(),
        client_id: client_id.to_string(),
    };

    let encoding_key = match key_manager.encoding_key() {
        Ok(k) => k,
        Err(e) => {
            tracing::error!("Encoding key error: {e}");
            return token_error(500, "server_error", "Failed to load signing key");
        }
    };

    let access_token =
        match crate::jwt::sign_access_token(&claims, &encoding_key, key_manager.active_kid()) {
            Ok(t) => t,
            Err(e) => {
                tracing::error!("JWT signing error: {e}");
                return token_error(500, "server_error", "Failed to sign access token");
            }
        };

    // Create refresh token
    let refresh_token = match store.store_refresh_token(
        client_id.to_string(),
        redirect_uri.to_string(),
        subject.to_string(),
        scope.to_string(),
        auth.config.mcp.refresh_token_ttl,
    ) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Refresh token creation error: {e}");
            return token_error(500, "server_error", "Failed to create refresh token");
        }
    };

    let body = serde_json::json!({
        "access_token": access_token,
        "token_type": "Bearer",
        "expires_in": auth.config.mcp.access_token_ttl,
        "refresh_token": refresh_token,
        "scope": scope,
    });

    Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .header("Cache-Control", "no-store")
        .header("Pragma", "no-cache")
        .body(axum::body::Body::from(
            serde_json::to_string(&body).unwrap_or_else(|_| "{}".to_string()),
        ))
        .unwrap_or_else(|_| token_error(500, "server_error", "Response build failed"))
}

fn token_error(status: u16, error: &str, description: &str) -> Response {
    let body = serde_json::json!({
        "error": error,
        "error_description": description,
    });

    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .header("Cache-Control", "no-store")
        .body(axum::body::Body::from(
            serde_json::to_string(&body).unwrap_or_else(|_| "{}".to_string()),
        ))
        .unwrap_or_else(|_| {
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Internal error",
            )
                .into_response()
        })
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
