use super::{encode_scopes, url_encode};
use crate::config::ProviderConfig;
use crate::provider::{OAuthProvider, TokenSet};
use crate::session::UserProfile;
use crate::AuthError;
use std::future::Future;
use std::pin::Pin;

pub struct AppleProvider {
    client_id: String,
    client_secret: String,
    scopes: Vec<String>,
}

impl AppleProvider {
    pub fn from_config(config: &ProviderConfig) -> Result<Self, AuthError> {
        Ok(Self {
            client_id: config
                .client_id
                .clone()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| AuthError::Config("Apple provider requires clientId".into()))?,
            client_secret: config
                .client_secret
                .clone()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| AuthError::Config("Apple provider requires clientSecret".into()))?,
            scopes: config
                .scopes
                .clone()
                .unwrap_or_else(|| vec!["name".to_string(), "email".to_string()]),
        })
    }
}

/// Decode the payload portion of a JWT without verifying the signature.
/// Returns the parsed JSON value from the base64url-encoded middle segment.
fn decode_jwt_payload(jwt: &str) -> Result<serde_json::Value, AuthError> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

    let parts: Vec<&str> = jwt.split('.').collect();
    if parts.len() != 3 {
        return Err(AuthError::OAuth("Invalid id_token format".into()));
    }

    let payload_bytes = URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|e| AuthError::OAuth(format!("Failed to decode id_token payload: {e}")))?;

    serde_json::from_slice(&payload_bytes)
        .map_err(|e| AuthError::OAuth(format!("Failed to parse id_token payload: {e}")))
}

impl OAuthProvider for AppleProvider {
    fn id(&self) -> &str {
        "apple"
    }

    fn name(&self) -> &str {
        "Apple"
    }

    fn authorization_url(&self, state: &str, callback_url: &str) -> String {
        let scopes = encode_scopes(&self.scopes);
        format!(
            "https://appleid.apple.com/auth/authorize\
             ?client_id={}\
             &redirect_uri={}\
             &response_type=code\
             &scope={scopes}\
             &state={state}\
             &response_mode=form_post",
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
                .post("https://appleid.apple.com/auth/token")
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
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    AuthError::OAuth(format!("Apple token exchange failed: {error}"))
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
                scope: None,
            })
        })
    }

    fn fetch_user_profile<'a>(
        &'a self,
        tokens: &'a TokenSet,
        _client: &'a reqwest::Client,
    ) -> Pin<Box<dyn Future<Output = Result<UserProfile, AuthError>> + Send + 'a>> {
        Box::pin(async move {
            // Apple returns user info only in the id_token JWT.
            // Decode the payload to extract sub and email.
            let id_token = tokens
                .id_token
                .as_deref()
                .ok_or_else(|| AuthError::OAuth("Apple did not return an id_token".into()))?;

            let claims = decode_jwt_payload(id_token)?;

            let sub = claims
                .get("sub")
                .and_then(|v| v.as_str())
                .unwrap_or_default();

            Ok(UserProfile {
                id: format!("apple|{sub}"),
                name: claims
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                email: claims
                    .get("email")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                image: None,
            })
        })
    }
}
