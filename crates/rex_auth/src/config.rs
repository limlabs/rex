use serde::{Deserialize, Serialize};

/// Top-level auth configuration from rex.config.json `auth` section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    /// Secret key for session encryption. Supports `$ENV_VAR` syntax.
    #[serde(default)]
    pub secret: Option<String>,

    /// Issuer URL (e.g., "https://myapp.example.com"). Used for JWT `iss` claim.
    #[serde(default)]
    pub issuer: Option<String>,

    /// OAuth provider configurations.
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,

    /// Session configuration.
    #[serde(default)]
    pub session: SessionConfig,

    /// Custom page paths.
    #[serde(default)]
    pub pages: PagesConfig,

    /// MCP Authorization Server configuration.
    #[serde(default)]
    pub mcp: McpAuthConfig,
}

/// Configuration for an OAuth provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Provider type: "github", "google", "discord", "apple", "microsoft", "twitter", "oidc", "oauth"
    #[serde(rename = "type")]
    pub provider_type: String,

    /// Custom ID for the provider (defaults to type). Required for generic OIDC/OAuth.
    #[serde(default)]
    pub id: Option<String>,

    /// Display name for the provider.
    #[serde(default)]
    pub name: Option<String>,

    /// OAuth client ID. Supports `$ENV_VAR` syntax.
    #[serde(rename = "clientId", default)]
    pub client_id: Option<String>,

    /// OAuth client secret. Supports `$ENV_VAR` syntax.
    #[serde(rename = "clientSecret", default)]
    pub client_secret: Option<String>,

    /// OIDC issuer URL (for generic OIDC provider). Supports `$ENV_VAR` syntax.
    #[serde(default)]
    pub issuer: Option<String>,

    /// Authorization endpoint (for generic OAuth provider).
    #[serde(rename = "authorizationUrl", default)]
    pub authorization_url: Option<String>,

    /// Token endpoint (for generic OAuth provider).
    #[serde(rename = "tokenUrl", default)]
    pub token_url: Option<String>,

    /// User info endpoint (for generic OAuth provider).
    #[serde(rename = "userinfoUrl", default)]
    pub userinfo_url: Option<String>,

    /// OAuth scopes to request.
    #[serde(default)]
    pub scopes: Option<Vec<String>>,
}

impl ProviderConfig {
    /// Get the effective provider ID (custom id or provider type).
    pub fn effective_id(&self) -> &str {
        self.id.as_deref().unwrap_or(&self.provider_type)
    }
}

/// Session configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    /// Session max age in seconds (default: 30 days).
    #[serde(rename = "maxAge", default = "default_max_age")]
    pub max_age: u64,

    /// Session cookie name.
    #[serde(rename = "cookieName", default = "default_cookie_name")]
    pub cookie_name: String,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            max_age: default_max_age(),
            cookie_name: default_cookie_name(),
        }
    }
}

fn default_max_age() -> u64 {
    2_592_000 // 30 days
}

fn default_cookie_name() -> String {
    "__rex_session".to_string()
}

/// Custom page paths.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PagesConfig {
    /// Sign-in page path.
    #[serde(rename = "signIn", default)]
    pub sign_in: Option<String>,

    /// Error page path.
    #[serde(default)]
    pub error: Option<String>,
}

/// MCP Authorization Server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpAuthConfig {
    /// Whether the MCP authorization server is enabled.
    #[serde(default)]
    pub enabled: bool,

    /// Scopes that can be requested by MCP clients.
    #[serde(default = "default_mcp_scopes")]
    pub scopes: Vec<String>,

    /// Access token TTL in seconds.
    #[serde(rename = "accessTokenTtl", default = "default_access_token_ttl")]
    pub access_token_ttl: u64,

    /// Refresh token TTL in seconds.
    #[serde(rename = "refreshTokenTtl", default = "default_refresh_token_ttl")]
    pub refresh_token_ttl: u64,

    /// Client configuration.
    #[serde(default)]
    pub clients: ClientsConfig,
}

impl Default for McpAuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            scopes: default_mcp_scopes(),
            access_token_ttl: default_access_token_ttl(),
            refresh_token_ttl: default_refresh_token_ttl(),
            clients: ClientsConfig::default(),
        }
    }
}

fn default_mcp_scopes() -> Vec<String> {
    vec!["tools:read".to_string(), "tools:execute".to_string()]
}

fn default_access_token_ttl() -> u64 {
    3600 // 1 hour
}

fn default_refresh_token_ttl() -> u64 {
    2_592_000 // 30 days
}

/// Client registration configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClientsConfig {
    /// Whether dynamic client registration (RFC 7591) is allowed.
    /// Defaults to `false` for security-by-default; set to `true` to enable.
    #[serde(rename = "allowDynamic", default)]
    pub allow_dynamic: bool,

    /// Pre-registered static clients.
    #[serde(rename = "static", default)]
    pub static_clients: Vec<StaticClient>,
}

/// A pre-registered OAuth client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaticClient {
    #[serde(rename = "clientId")]
    pub client_id: String,
    #[serde(rename = "clientName")]
    pub client_name: String,
    #[serde(rename = "redirectUris")]
    pub redirect_uris: Vec<String>,
}

/// Resolve `$VAR` references in a string to environment variable values.
///
/// Returns `Err` if the env var is referenced but not set or empty.
pub fn resolve_env_vars(input: &str) -> Result<String, crate::AuthError> {
    if let Some(var_name) = input.strip_prefix('$') {
        match std::env::var(var_name) {
            Ok(val) if !val.is_empty() => Ok(val),
            _ => Err(crate::AuthError::Config(format!(
                "Environment variable {var_name} is not set (referenced as ${var_name})"
            ))),
        }
    } else {
        Ok(input.to_string())
    }
}

/// Parse an `AuthConfig` from a `serde_json::Value`, resolving `$ENV_VAR` references.
pub fn parse_auth_config(value: &serde_json::Value) -> Result<AuthConfig, crate::AuthError> {
    let mut config: AuthConfig = serde_json::from_value(value.clone())
        .map_err(|e| crate::AuthError::Config(format!("Invalid auth config: {e}")))?;

    // Resolve env vars in secret (optional — falls back to env/auto-generate)
    if let Some(ref secret) = config.secret {
        match resolve_env_vars(secret) {
            Ok(val) => config.secret = Some(val),
            Err(_) => {
                tracing::debug!("Auth secret env var not set, will use fallback");
                config.secret = None;
            }
        }
    }

    // Resolve env vars in issuer
    if let Some(ref issuer) = config.issuer {
        config.issuer = Some(resolve_env_vars(issuer)?);
    }

    // Resolve env vars in provider configs
    for provider in &mut config.providers {
        if let Some(ref id) = provider.client_id {
            provider.client_id = Some(resolve_env_vars(id)?);
        }
        if let Some(ref secret) = provider.client_secret {
            provider.client_secret = Some(resolve_env_vars(secret)?);
        }
        if let Some(ref issuer) = provider.issuer {
            provider.issuer = Some(resolve_env_vars(issuer)?);
        }
    }

    Ok(config)
}

/// Derive the 32-byte encryption key from the auth secret using HKDF-SHA256.
///
/// Uses domain separation ("rex-auth-session-v1" salt, "session-encryption" info)
/// so the same secret used for different purposes yields different keys.
pub fn derive_key(secret: &str) -> [u8; 32] {
    use hkdf::Hkdf;
    use sha2::Sha256;
    let hk = Hkdf::<Sha256>::new(Some(b"rex-auth-session-v1"), secret.as_bytes());
    let mut key = [0u8; 32];
    hk.expand(b"session-encryption", &mut key)
        .expect("32 bytes is a valid HKDF-SHA256 output length");
    key
}

/// Get or auto-generate an auth secret.
///
/// Priority: config value → REX_AUTH_SECRET env var → auto-generate and store in `.rex/auth/secret`.
pub fn resolve_secret(
    config_secret: Option<&str>,
    project_root: &std::path::Path,
) -> Result<String, crate::AuthError> {
    // 1. Config value
    if let Some(secret) = config_secret {
        if !secret.is_empty() {
            return Ok(secret.to_string());
        }
    }

    // 2. Environment variable
    if let Ok(secret) = std::env::var("REX_AUTH_SECRET") {
        if !secret.is_empty() {
            return Ok(secret);
        }
    }

    // 3. Auto-generate and store
    let secret_path = project_root.join(".rex").join("auth").join("secret");
    if secret_path.exists() {
        return std::fs::read_to_string(&secret_path)
            .map(|s| s.trim().to_string())
            .map_err(|e| crate::AuthError::Config(format!("Failed to read auth secret: {e}")));
    }

    // Generate new secret
    use rand::Rng;
    let secret: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(64)
        .map(char::from)
        .collect();

    if let Some(parent) = secret_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| crate::AuthError::Config(format!("Failed to create auth dir: {e}")))?;
    }
    std::fs::write(&secret_path, &secret)
        .map_err(|e| crate::AuthError::Config(format!("Failed to write auth secret: {e}")))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&secret_path, std::fs::Permissions::from_mode(0o600)).map_err(
            |e| crate::AuthError::Config(format!("Failed to set secret permissions: {e}")),
        )?;
    }

    tracing::info!("Auto-generated auth secret at {}", secret_path.display());
    Ok(secret)
}
