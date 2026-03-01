use crate::config::ProviderConfig;
use crate::provider::{OAuthProvider, TokenSet};
use crate::session::UserProfile;
use crate::AuthError;
use std::future::Future;
use std::pin::Pin;

fn url_encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

pub struct DiscordProvider {
    client_id: String,
    client_secret: String,
    scopes: Vec<String>,
}

impl DiscordProvider {
    pub fn from_config(config: &ProviderConfig) -> Result<Self, AuthError> {
        Ok(Self {
            client_id: config
                .client_id
                .clone()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| AuthError::Config("Discord provider requires clientId".into()))?,
            client_secret: config
                .client_secret
                .clone()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    AuthError::Config("Discord provider requires clientSecret".into())
                })?,
            scopes: config
                .scopes
                .clone()
                .unwrap_or_else(|| vec!["identify".to_string(), "email".to_string()]),
        })
    }
}

impl OAuthProvider for DiscordProvider {
    fn id(&self) -> &str {
        "discord"
    }

    fn name(&self) -> &str {
        "Discord"
    }

    fn authorization_url(&self, state: &str, callback_url: &str) -> String {
        let scopes = self.scopes.join("%20");
        format!(
            "https://discord.com/api/oauth2/authorize\
             ?client_id={}\
             &redirect_uri={}\
             &response_type=code\
             &scope={scopes}\
             &state={state}",
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
                .post("https://discord.com/api/oauth2/token")
                .form(&[
                    ("client_id", self.client_id.as_str()),
                    ("client_secret", self.client_secret.as_str()),
                    ("code", code),
                    ("grant_type", "authorization_code"),
                    ("redirect_uri", callback_url),
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
                    AuthError::OAuth(format!("Discord token exchange failed: {error}"))
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
                .get("https://discord.com/api/users/@me")
                .header("Authorization", format!("Bearer {}", tokens.access_token))
                .send()
                .await?
                .json()
                .await?;

            let id_str = user.get("id").and_then(|v| v.as_str()).unwrap_or_default();

            let avatar = user
                .get("avatar")
                .and_then(|v| v.as_str())
                .map(|avatar| format!("https://cdn.discordapp.com/avatars/{id_str}/{avatar}.png"));

            Ok(UserProfile {
                id: format!("discord|{id_str}"),
                name: user
                    .get("username")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                email: user.get("email").and_then(|v| v.as_str()).map(String::from),
                image: avatar,
            })
        })
    }
}
