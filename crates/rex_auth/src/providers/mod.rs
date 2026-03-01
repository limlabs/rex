pub mod apple;
pub mod discord;
pub mod github;
pub mod google;
pub mod microsoft;
pub mod oauth;
pub mod oidc;
pub mod twitter;

use crate::config::ProviderConfig;
use crate::provider::OAuthProvider;
use std::collections::HashMap;
use std::sync::Arc;

/// URL-encode a string for use in query parameters.
pub(crate) fn url_encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// Encode a list of scopes into a single URL-safe string (space-separated via %20).
/// Each individual scope value is URL-encoded first.
pub(crate) fn encode_scopes(scopes: &[String]) -> String {
    scopes
        .iter()
        .map(|s| url_encode(s))
        .collect::<Vec<_>>()
        .join("%20")
}

/// Build a map of provider ID → provider instance from the auth config.
pub fn build_providers(
    configs: &[ProviderConfig],
) -> Result<HashMap<String, Arc<dyn OAuthProvider>>, crate::AuthError> {
    let mut providers: HashMap<String, Arc<dyn OAuthProvider>> = HashMap::new();

    for config in configs {
        let provider: Arc<dyn OAuthProvider> = match config.provider_type.as_str() {
            "github" => Arc::new(github::GitHubProvider::from_config(config)?),
            "google" => Arc::new(google::GoogleProvider::from_config(config)?),
            "discord" => Arc::new(discord::DiscordProvider::from_config(config)?),
            "apple" => Arc::new(apple::AppleProvider::from_config(config)?),
            "microsoft" => Arc::new(microsoft::MicrosoftProvider::from_config(config)?),
            "twitter" => Arc::new(twitter::TwitterProvider::from_config(config)?),
            "oidc" => Arc::new(oidc::GenericOidcProvider::from_config(config)?),
            "oauth" => Arc::new(oauth::GenericOAuthProvider::from_config(config)?),
            other => return Err(crate::AuthError::UnknownProvider(other.to_string())),
        };

        providers.insert(config.effective_id().to_string(), provider);
    }

    Ok(providers)
}
