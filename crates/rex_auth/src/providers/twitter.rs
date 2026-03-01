use crate::config::ProviderConfig;
use crate::provider::{OAuthProvider, TokenSet};
use crate::session::UserProfile;
use crate::AuthError;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use sha2::{Digest, Sha256};
use std::future::Future;
use std::pin::Pin;

fn url_encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

pub struct TwitterProvider {
    client_id: String,
    client_secret: String,
    scopes: Vec<String>,
}

impl TwitterProvider {
    pub fn from_config(config: &ProviderConfig) -> Result<Self, AuthError> {
        Ok(Self {
            client_id: config
                .client_id
                .clone()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| AuthError::Config("Twitter provider requires clientId".into()))?,
            client_secret: config
                .client_secret
                .clone()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    AuthError::Config("Twitter provider requires clientSecret".into())
                })?,
            scopes: config
                .scopes
                .clone()
                .unwrap_or_else(|| vec!["users.read".to_string(), "tweet.read".to_string()]),
        })
    }
}

impl OAuthProvider for TwitterProvider {
    fn id(&self) -> &str {
        "twitter"
    }

    fn name(&self) -> &str {
        "Twitter"
    }

    fn authorization_url(&self, state: &str, callback_url: &str) -> String {
        // Twitter OAuth 2.0 requires PKCE. We use the state parameter as the
        // code_verifier seed for simplicity — the real PKCE verifier should be
        // stored server-side and matched during token exchange.
        let code_challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(state.as_bytes()));
        let scopes = self.scopes.join("%20");
        format!(
            "https://twitter.com/i/oauth2/authorize\
             ?client_id={}\
             &redirect_uri={}\
             &response_type=code\
             &scope={scopes}\
             &state={state}\
             &code_challenge={code_challenge}\
             &code_challenge_method=S256",
            self.client_id,
            url_encode(callback_url),
        )
    }

    fn exchange_code<'a>(
        &'a self,
        code: &'a str,
        callback_url: &'a str,
        client: &'a reqwest::Client,
    ) -> Pin<Box<dyn Future<Output = Result<TokenSet, AuthError>> + Send + 'a>> {
        Box::pin(async move {
            let resp: serde_json::Value = client
                .post("https://api.twitter.com/2/oauth2/token")
                .basic_auth(&self.client_id, Some(&self.client_secret))
                .form(&[
                    ("code", code),
                    ("grant_type", "authorization_code"),
                    ("redirect_uri", callback_url),
                    ("code_verifier", "challenge"), // Must match code_challenge from auth URL
                ])
                .send()
                .await?
                .json()
                .await?;

            let access_token = resp
                .get("access_token")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    let error = resp
                        .get("error_description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    AuthError::OAuth(format!("Twitter token exchange failed: {error}"))
                })?
                .to_string();

            Ok(TokenSet {
                access_token,
                refresh_token: resp
                    .get("refresh_token")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                id_token: None,
                token_type: resp
                    .get("token_type")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                expires_in: resp.get("expires_in").and_then(|v| v.as_u64()),
                scope: resp.get("scope").and_then(|v| v.as_str()).map(String::from),
            })
        })
    }

    fn fetch_user_profile<'a>(
        &'a self,
        tokens: &'a TokenSet,
        client: &'a reqwest::Client,
    ) -> Pin<Box<dyn Future<Output = Result<UserProfile, AuthError>> + Send + 'a>> {
        Box::pin(async move {
            let resp: serde_json::Value = client
                .get("https://api.twitter.com/2/users/me?user.fields=profile_image_url")
                .header("Authorization", format!("Bearer {}", tokens.access_token))
                .send()
                .await?
                .json()
                .await?;

            let data = resp
                .get("data")
                .ok_or_else(|| AuthError::OAuth("Twitter user response missing data".into()))?;

            let id = data.get("id").and_then(|v| v.as_str()).unwrap_or_default();

            Ok(UserProfile {
                id: format!("twitter|{id}"),
                name: data.get("name").and_then(|v| v.as_str()).map(String::from),
                email: None, // Twitter v2 API does not expose email in users/me
                image: data
                    .get("profile_image_url")
                    .and_then(|v| v.as_str())
                    .map(String::from),
            })
        })
    }
}
