use super::{encode_scopes, url_encode};
use crate::config::ProviderConfig;
use crate::provider::{OAuthProvider, TokenSet};
use crate::session::UserProfile;
use crate::AuthError;
use std::future::Future;
use std::pin::Pin;

pub struct GoogleProvider {
    client_id: String,
    client_secret: String,
    scopes: Vec<String>,
}

impl GoogleProvider {
    pub fn from_config(config: &ProviderConfig) -> Result<Self, AuthError> {
        Ok(Self {
            client_id: config
                .client_id
                .clone()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| AuthError::Config("Google provider requires clientId".into()))?,
            client_secret: config
                .client_secret
                .clone()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| AuthError::Config("Google provider requires clientSecret".into()))?,
            scopes: config.scopes.clone().unwrap_or_else(|| {
                vec![
                    "openid".to_string(),
                    "email".to_string(),
                    "profile".to_string(),
                ]
            }),
        })
    }
}

impl OAuthProvider for GoogleProvider {
    fn id(&self) -> &str {
        "google"
    }

    fn name(&self) -> &str {
        "Google"
    }

    fn authorization_url(&self, state: &str, callback_url: &str) -> String {
        let scopes = encode_scopes(&self.scopes);
        format!(
            "https://accounts.google.com/o/oauth2/v2/auth\
             ?client_id={}\
             &redirect_uri={}\
             &response_type=code\
             &scope={scopes}\
             &state={state}\
             &access_type=offline\
             &prompt=consent",
            self.client_id,
            url_encode(callback_url),
        )
    }

    fn exchange_code<'a>(
        &'a self,
        code: &'a str,
        callback_url: &'a str,
        client: &'a reqwest::Client,
        _code_verifier: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<TokenSet, AuthError>> + Send + 'a>> {
        Box::pin(async move {
            let resp: serde_json::Value = client
                .post("https://oauth2.googleapis.com/token")
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
                    AuthError::OAuth(format!("Google token exchange failed: {error}"))
                })?
                .to_string();

            Ok(TokenSet {
                access_token,
                refresh_token: resp
                    .get("refresh_token")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                id_token: resp
                    .get("id_token")
                    .and_then(|v| v.as_str())
                    .map(String::from),
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
                .get("https://www.googleapis.com/oauth2/v2/userinfo")
                .header("Authorization", format!("Bearer {}", tokens.access_token))
                .send()
                .await?
                .json()
                .await?;

            let id = user
                .get("id")
                .and_then(|v| v.as_str())
                .map(|id| format!("google|{id}"))
                .unwrap_or_default();

            Ok(UserProfile {
                id,
                name: user.get("name").and_then(|v| v.as_str()).map(String::from),
                email: user.get("email").and_then(|v| v.as_str()).map(String::from),
                image: user
                    .get("picture")
                    .and_then(|v| v.as_str())
                    .map(String::from),
            })
        })
    }
}
