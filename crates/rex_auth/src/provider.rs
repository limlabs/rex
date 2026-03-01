use crate::session::UserProfile;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;

/// Token set returned from an OAuth code exchange.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenSet {
    pub access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

/// Trait for OAuth/OIDC providers.
///
/// Each provider knows how to build an authorization URL, exchange a code for tokens,
/// and fetch the user's profile. Methods return boxed futures for dyn compatibility.
pub trait OAuthProvider: Send + Sync {
    /// Unique identifier for this provider (e.g., "github", "google").
    fn id(&self) -> &str;

    /// Display name for this provider (e.g., "GitHub", "Google").
    fn name(&self) -> &str;

    /// Build the authorization URL to redirect the user to.
    fn authorization_url(&self, state: &str, callback_url: &str) -> String;

    /// Exchange an authorization code for tokens.
    fn exchange_code<'a>(
        &'a self,
        code: &'a str,
        callback_url: &'a str,
        client: &'a reqwest::Client,
    ) -> Pin<Box<dyn Future<Output = Result<TokenSet, crate::AuthError>> + Send + 'a>>;

    /// Fetch the user's profile using the obtained tokens.
    fn fetch_user_profile<'a>(
        &'a self,
        tokens: &'a TokenSet,
        client: &'a reqwest::Client,
    ) -> Pin<Box<dyn Future<Output = Result<UserProfile, crate::AuthError>> + Send + 'a>>;
}
