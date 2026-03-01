use crate::AuthServer;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::Json;
use std::sync::Arc;

#[derive(serde::Deserialize)]
pub struct RegistrationRequest {
    #[serde(default)]
    pub client_name: Option<String>,
    #[serde(default)]
    pub redirect_uris: Vec<String>,
    #[serde(default)]
    pub grant_types: Option<Vec<String>>,
    #[serde(default)]
    pub response_types: Option<Vec<String>>,
    #[serde(default)]
    pub token_endpoint_auth_method: Option<String>,
}

/// POST /_rex/auth/register — Dynamic Client Registration (RFC 7591).
pub async fn register_handler(
    State(auth): State<Arc<AuthServer>>,
    Json(req): Json<RegistrationRequest>,
) -> Response {
    // Check if dynamic registration is allowed
    if !auth.config.mcp.clients.allow_dynamic {
        return error_response(403, "registration_not_allowed", "Dynamic client registration is disabled");
    }

    // Validate redirect URIs
    if req.redirect_uris.is_empty() {
        return error_response(400, "invalid_redirect_uri", "At least one redirect_uri is required");
    }

    for uri in &req.redirect_uris {
        if !is_valid_redirect_uri(uri) {
            return error_response(
                400,
                "invalid_redirect_uri",
                &format!("Invalid redirect_uri: {uri}"),
            );
        }
    }

    let store = match &auth.store {
        Some(s) => s,
        None => return error_response(500, "server_error", "Auth store not initialized"),
    };

    let client_name = req
        .client_name
        .unwrap_or_else(|| "Unknown Client".to_string());

    match store.register_client(client_name.clone(), req.redirect_uris.clone()) {
        Ok(client) => {
            let body = serde_json::json!({
                "client_id": client.client_id,
                "client_name": client.client_name,
                "redirect_uris": client.redirect_uris,
                "grant_types": ["authorization_code", "refresh_token"],
                "response_types": ["code"],
                "token_endpoint_auth_method": "none",
            });

            Response::builder()
                .status(201)
                .header("Content-Type", "application/json")
                .header("Cache-Control", "no-store")
                .body(axum::body::Body::from(
                    serde_json::to_string(&body).unwrap_or_else(|_| "{}".to_string()),
                ))
                .unwrap_or_else(|_| error_response(500, "server_error", "Response build failed"))
        }
        Err(e) => {
            tracing::error!("Client registration error: {e}");
            error_response(500, "server_error", "Failed to register client")
        }
    }
}

/// Validate a redirect URI for registration.
fn is_valid_redirect_uri(uri: &str) -> bool {
    // Reject dangerous schemes
    let lower = uri.to_lowercase();
    if lower.starts_with("javascript:") || lower.starts_with("data:") {
        return false;
    }

    // Must be a valid URL
    match url::Url::parse(uri) {
        Ok(parsed) => {
            // Must have http or https scheme (or custom scheme for native apps)
            let scheme = parsed.scheme();
            matches!(scheme, "http" | "https") || scheme.len() > 4 // custom scheme like "myapp"
        }
        Err(_) => false,
    }
}

fn error_response(status: u16, error: &str, description: &str) -> Response {
    let body = serde_json::json!({
        "error": error,
        "error_description": description,
    });

    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_redirect_uris() {
        assert!(is_valid_redirect_uri("http://localhost:8080/callback"));
        assert!(is_valid_redirect_uri("https://myapp.com/auth/callback"));
        assert!(is_valid_redirect_uri("myapp://callback")); // custom scheme
    }

    #[test]
    fn test_invalid_redirect_uris() {
        assert!(!is_valid_redirect_uri("javascript:alert(1)"));
        assert!(!is_valid_redirect_uri("data:text/html,<script>"));
        assert!(!is_valid_redirect_uri("not a url at all"));
    }
}
