use crate::config::ProviderConfig;
use crate::provider::{OAuthProvider, TokenSet};
use crate::session::UserProfile;
use crate::AuthError;
use std::future::Future;
use std::pin::Pin;

fn url_encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

pub struct GitHubProvider {
    client_id: String,
    client_secret: String,
    scopes: Vec<String>,
}

impl GitHubProvider {
    pub fn from_config(config: &ProviderConfig) -> Result<Self, AuthError> {
        let client_id = config
            .client_id
            .clone()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| AuthError::Config("GitHub provider requires clientId".into()))?;
        let client_secret = config
            .client_secret
            .clone()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| AuthError::Config("GitHub provider requires clientSecret".into()))?;
        Ok(Self {
            client_id,
            client_secret,
            scopes: config
                .scopes
                .clone()
                .unwrap_or_else(|| vec!["read:user".to_string(), "user:email".to_string()]),
        })
    }
}

impl OAuthProvider for GitHubProvider {
    fn id(&self) -> &str {
        "github"
    }

    fn name(&self) -> &str {
        "GitHub"
    }

    fn authorization_url(&self, state: &str, callback_url: &str) -> String {
        let scopes = self.scopes.join("%20");
        format!(
            "https://github.com/login/oauth/authorize\
             ?client_id={}\
             &redirect_uri={}\
             &scope={scopes}\
             &state={state}",
            self.client_id,
            url_encode(callback_url),
        )
    }

    fn exchange_code<'a>(
        &'a self,
        code: &'a str,
        _callback_url: &'a str,
        client: &'a reqwest::Client,
    ) -> Pin<Box<dyn Future<Output = Result<TokenSet, AuthError>> + Send + 'a>> {
        Box::pin(async move {
            let resp: serde_json::Value = client
                .post("https://github.com/login/oauth/access_token")
                .header("Accept", "application/json")
                .json(&serde_json::json!({
                    "client_id": self.client_id,
                    "client_secret": self.client_secret,
                    "code": code,
                }))
                .send()
                .await?
                .json()
                .await?;

            let access_token = resp
                .get("access_token")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    let error = resp
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    AuthError::OAuth(format!("GitHub token exchange failed: {error}"))
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
            let user: serde_json::Value = client
                .get("https://api.github.com/user")
                .header("Authorization", format!("Bearer {}", tokens.access_token))
                .header("User-Agent", "Rex")
                .send()
                .await?
                .json()
                .await?;

            let id = user
                .get("id")
                .and_then(|v| v.as_u64())
                .map(|id| format!("github|{id}"))
                .unwrap_or_default();

            // Fetch primary email if not in profile
            let email = if let Some(email) = user.get("email").and_then(|v| v.as_str()) {
                Some(email.to_string())
            } else {
                // Try the emails endpoint
                let emails: Vec<serde_json::Value> = match client
                    .get("https://api.github.com/user/emails")
                    .header("Authorization", format!("Bearer {}", tokens.access_token))
                    .header("User-Agent", "Rex")
                    .send()
                    .await
                {
                    Ok(resp) => resp.json().await.unwrap_or_default(),
                    Err(_) => Vec::new(),
                };

                emails
                    .iter()
                    .find(|e| e.get("primary").and_then(|p| p.as_bool()).unwrap_or(false))
                    .and_then(|e| e.get("email").and_then(|v| v.as_str()))
                    .map(String::from)
            };

            Ok(UserProfile {
                id,
                name: user.get("name").and_then(|v| v.as_str()).map(String::from),
                email,
                image: user
                    .get("avatar_url")
                    .and_then(|v| v.as_str())
                    .map(String::from),
            })
        })
    }
}
