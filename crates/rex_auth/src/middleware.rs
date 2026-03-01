use crate::jwt::{self, AccessTokenClaims};
use crate::keys::KeyManager;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// Axum extractor that validates a Bearer token and provides the claims.
///
/// Usage:
/// ```ignore
/// async fn handler(auth: BearerAuth) -> impl IntoResponse {
///     auth.0.require_scope("tools:execute")?;
///     // ...
/// }
/// ```
pub struct BearerAuth(pub AccessTokenClaims);

impl<S> FromRequestParts<S> for BearerAuth
where
    S: Send + Sync,
{
    type Rejection = AuthRejection;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Get Authorization header
        let auth_header = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .ok_or(AuthRejection::Missing)?;

        // Must be "Bearer <token>"
        let token = auth_header
            .strip_prefix("Bearer ")
            .ok_or(AuthRejection::InvalidScheme)?;

        // Get the key manager and issuer from extensions
        let auth_state = parts
            .extensions
            .get::<AuthExtension>()
            .ok_or(AuthRejection::NotConfigured)?;

        // Validate the JWT
        let decoding_keys = auth_state
            .key_manager
            .decoding_keys()
            .map_err(|_| AuthRejection::NotConfigured)?;
        let claims = jwt::validate_access_token(token, &decoding_keys, &auth_state.issuer)
            .map_err(|_| AuthRejection::InvalidToken)?;

        Ok(BearerAuth(claims))
    }
}

/// Extension inserted into Axum request extensions for Bearer auth validation.
#[derive(Clone)]
pub struct AuthExtension {
    pub key_manager: std::sync::Arc<KeyManager>,
    pub issuer: String,
}

/// Rejection type for `BearerAuth` extractor.
pub enum AuthRejection {
    Missing,
    InvalidScheme,
    InvalidToken,
    NotConfigured,
}

impl IntoResponse for AuthRejection {
    fn into_response(self) -> Response {
        match self {
            Self::Missing => (
                StatusCode::UNAUTHORIZED,
                [("WWW-Authenticate", "Bearer")],
                "Missing Authorization header",
            )
                .into_response(),
            Self::InvalidScheme => (
                StatusCode::UNAUTHORIZED,
                [("WWW-Authenticate", "Bearer")],
                "Invalid authorization scheme (expected Bearer)",
            )
                .into_response(),
            Self::InvalidToken => (
                StatusCode::UNAUTHORIZED,
                [("WWW-Authenticate", "Bearer error=\"invalid_token\"")],
                "Invalid or expired token",
            )
                .into_response(),
            Self::NotConfigured => {
                (StatusCode::INTERNAL_SERVER_ERROR, "Auth not configured").into_response()
            }
        }
    }
}

/// Validate a Bearer token from a raw headers map (for framework-agnostic use).
pub fn validate_bearer_token(
    headers: &std::collections::HashMap<String, String>,
    key_manager: &KeyManager,
    issuer: &str,
) -> Result<AccessTokenClaims, crate::AuthError> {
    let auth_header = headers
        .get("authorization")
        .or_else(|| headers.get("Authorization"))
        .ok_or(crate::AuthError::InvalidToken)?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or(crate::AuthError::InvalidToken)?;

    let decoding_keys = key_manager.decoding_keys()?;
    jwt::validate_access_token(token, &decoding_keys, issuer)
}
