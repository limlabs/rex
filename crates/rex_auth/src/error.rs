use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("Missing auth secret — set REX_AUTH_SECRET or auth.secret in rex.config.json")]
    MissingSecret,

    #[error("Invalid auth configuration: {0}")]
    Config(String),

    #[error("Unknown provider: {0}")]
    UnknownProvider(String),

    #[error("OAuth error: {0}")]
    OAuth(String),

    #[error("Session error: {0}")]
    Session(String),

    #[error("CSRF validation failed")]
    CsrfMismatch,

    #[error("PKCE verification failed")]
    PkceFailure,

    #[error("Invalid authorization code")]
    InvalidCode,

    #[error("Invalid redirect URI: {0}")]
    InvalidRedirectUri(String),

    #[error("Client not found: {0}")]
    ClientNotFound(String),

    #[error("Invalid grant: {0}")]
    InvalidGrant(String),

    #[error("Invalid token")]
    InvalidToken,

    #[error("Insufficient scope: required {required}, have {have}")]
    InsufficientScope { required: String, have: String },

    #[error("Token expired")]
    TokenExpired,

    #[error("Registration not allowed")]
    RegistrationNotAllowed,

    #[error("Unsupported grant type: {0}")]
    UnsupportedGrantType(String),

    #[error("Rate limited")]
    RateLimited,

    #[error("Store error: {0}")]
    Store(String),

    #[error("Key error: {0}")]
    Key(String),

    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

impl AuthError {
    /// RFC 6749 error code for token endpoint responses.
    pub fn oauth_error_code(&self) -> &'static str {
        match self {
            Self::InvalidCode | Self::InvalidGrant(_) => "invalid_grant",
            Self::PkceFailure => "invalid_grant",
            Self::ClientNotFound(_) => "invalid_client",
            Self::InvalidRedirectUri(_) => "invalid_request",
            Self::UnsupportedGrantType(_) => "unsupported_grant_type",
            Self::InsufficientScope { .. } => "insufficient_scope",
            Self::InvalidToken | Self::TokenExpired => "invalid_token",
            Self::RateLimited => "slow_down",
            _ => "server_error",
        }
    }
}
