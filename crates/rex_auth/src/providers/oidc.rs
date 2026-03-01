use crate::config::ProviderConfig;
use crate::provider::{OAuthProvider, TokenSet};
use crate::session::UserProfile;
use crate::AuthError;
use std::future::Future;
use std::pin::Pin;
use tokio::sync::OnceCell;

fn url_encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// Discovered OIDC endpoints from `.well-known/openid-configuration`.
#[derive(Debug, Clone)]
struct OidcDiscovery {
    _authorization_endpoint: String,
    token_endpoint: String,
    userinfo_endpoint: Option<String>,
}

pub struct GenericOidcProvider {
    provider_id: String,
    display_name: String,
    client_id: String,
    client_secret: String,
    issuer: String,
    scopes: Vec<String>,
    discovery: OnceCell<OidcDiscovery>,
}

impl GenericOidcProvider {
    pub fn from_config(config: &ProviderConfig) -> Result<Self, AuthError> {
        let provider_id = config
            .id
            .clone()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| AuthError::Config("OIDC provider requires id".into()))?;

        let display_name = config
            .name
            .clone()
            .unwrap_or_else(|| provider_id.clone());

        Ok(Self {
            provider_id,
            display_name,
            client_id: config
                .client_id
                .clone()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| AuthError::Config("OIDC provider requires clientId".into()))?,
            client_secret: config
                .client_secret
                .clone()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| AuthError::Config("OIDC provider requires clientSecret".into()))?,
            issuer: config
                .issuer
                .clone()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| AuthError::Config("OIDC provider requires issuer".into()))?,
            scopes: config.scopes.clone().unwrap_or_else(|| {
                vec![
                    "openid".to_string(),
                    "email".to_string(),
                    "profile".to_string(),
                ]
            }),
            discovery: OnceCell::new(),
        })
    }

    /// Fetch and cache the OIDC discovery document.
    async fn discover(&self, client: &reqwest::Client) -> Result<&OidcDiscovery, AuthError> {
        self.discovery
            .get_or_try_init(|| async {
                let url = format!(
                    "{}/.well-known/openid-configuration",
                    self.issuer.trim_end_matches('/')
                );

                let doc: serde_json::Value = client
                    .get(&url)
                    .send()
                    .await?
                    .json()
                    .await?;

                let authorization_endpoint = doc
                    .get("authorization_endpoint")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        AuthError::Config(
                            "OIDC discovery missing authorization_endpoint".into(),
                        )
                    })?
                    .to_string();

                let token_endpoint = doc
                    .get("token_endpoint")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        AuthError::Config("OIDC discovery missing token_endpoint".into())
                    })?
                    .to_string();

                let userinfo_endpoint = doc
                    .get("userinfo_endpoint")
                    .and_then(|v| v.as_str())
                    .map(String::from);

                Ok(OidcDiscovery {
                    _authorization_endpoint: authorization_endpoint,
                    token_endpoint,
                    userinfo_endpoint,
                })
            })
            .await
    }
}

impl OAuthProvider for GenericOidcProvider {
    fn id(&self) -> &str {
        &self.provider_id
    }

    fn name(&self) -> &str {
        &self.display_name
    }

    fn authorization_url(&self, state: &str, callback_url: &str) -> String {
        // We cannot call the async discover() in this sync method, so we build
        // the URL using the issuer + /authorize path. Most OIDC providers follow
        // this convention. The discovered endpoint is used during exchange_code
        // and fetch_user_profile.
        let base = self
            .issuer
            .trim_end_matches('/');
        let scopes = self.scopes.join("%20");
        format!(
            "{base}/authorize\
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
            let discovery = self.discover(client).await?;

            let resp: serde_json::Value = client
                .post(&discovery.token_endpoint)
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
                    AuthError::OAuth(format!("OIDC token exchange failed: {error}"))
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
                scope: resp
                    .get("scope")
                    .and_then(|v| v.as_str())
                    .map(String::from),
            })
        })
    }

    fn fetch_user_profile<'a>(
        &'a self,
        tokens: &'a TokenSet,
        client: &'a reqwest::Client,
    ) -> Pin<Box<dyn Future<Output = Result<UserProfile, AuthError>> + Send + 'a>> {
        Box::pin(async move {
            let discovery = self.discover(client).await?;

            let userinfo_url = discovery
                .userinfo_endpoint
                .as_deref()
                .ok_or_else(|| {
                    AuthError::OAuth("OIDC provider has no userinfo_endpoint".into())
                })?;

            let user: serde_json::Value = client
                .get(userinfo_url)
                .header("Authorization", format!("Bearer {}", tokens.access_token))
                .send()
                .await?
                .json()
                .await?;

            let sub = user
                .get("sub")
                .and_then(|v| v.as_str())
                .unwrap_or_default();

            Ok(UserProfile {
                id: format!("{}|{sub}", self.provider_id),
                name: user
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                email: user
                    .get("email")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                image: user
                    .get("picture")
                    .and_then(|v| v.as_str())
                    .map(String::from),
            })
        })
    }
}
