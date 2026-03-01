use crate::AuthServer;
use axum::extract::State;
use axum::response::IntoResponse;
use std::sync::Arc;

/// GET /.well-known/oauth-authorization-server
///
/// Returns RFC 8414 Authorization Server Metadata.
pub async fn metadata_handler(State(auth): State<Arc<AuthServer>>) -> impl IntoResponse {
    let base = &auth.base_url;

    let metadata = serde_json::json!({
        "issuer": auth.issuer(),
        "authorization_endpoint": format!("{base}/_rex/auth/authorize"),
        "token_endpoint": format!("{base}/_rex/auth/token"),
        "registration_endpoint": format!("{base}/_rex/auth/register"),
        "revocation_endpoint": format!("{base}/_rex/auth/revoke"),
        "jwks_uri": format!("{base}/_rex/auth/jwks"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "token_endpoint_auth_methods_supported": ["none"],
        "code_challenge_methods_supported": ["S256"],
        "scopes_supported": auth.config.mcp.scopes,
    });

    (
        [
            (axum::http::header::CONTENT_TYPE, "application/json"),
            (
                axum::http::header::CACHE_CONTROL,
                "public, max-age=3600",
            ),
        ],
        serde_json::to_string(&metadata).unwrap_or_else(|_| "{}".to_string()),
    )
}
