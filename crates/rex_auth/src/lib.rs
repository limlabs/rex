pub mod authz_server;
pub mod client_handlers;
pub mod config;
pub mod cookies;
pub mod csrf;
pub mod error;
pub mod jwt;
pub mod keys;
pub mod middleware;
pub mod pkce;
pub mod provider;
pub mod providers;
pub mod scopes;
pub mod session;
pub mod store;

pub use config::AuthConfig;
pub use cookies::{cookies_from_header_map, cookies_from_headers, parse_cookies};
pub use error::AuthError;
pub use jwt::AccessTokenClaims;
pub use keys::KeyManager;
pub use middleware::{AuthExtension, BearerAuth};
pub use provider::OAuthProvider;
pub use session::{SessionData, UserProfile};
pub use store::FileStore;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// The main auth server struct, shared across all auth handlers.
///
/// Contains OAuth providers, session encryption key, key manager for JWTs,
/// and the file store for OAuth 2.1 authorization server state.
pub struct AuthServer {
    pub config: AuthConfig,
    pub providers: HashMap<String, Arc<dyn OAuthProvider>>,
    pub session_key: [u8; 32],
    pub base_url: String,
    pub is_dev: bool,
    pub http_client: reqwest::Client,
    pub key_manager: Option<KeyManager>,
    pub store: Option<FileStore>,
}

impl AuthServer {
    /// Initialize the auth server from a parsed `AuthConfig`.
    ///
    /// - Resolves the auth secret (config → env → auto-generate)
    /// - Builds OAuth providers from config
    /// - Initializes the key manager and store if MCP is enabled
    pub fn new(
        config: AuthConfig,
        project_root: &Path,
        base_url: &str,
        is_dev: bool,
    ) -> Result<Self, AuthError> {
        // Resolve secret
        let secret = config::resolve_secret(config.secret.as_deref(), project_root)?;
        let session_key = config::derive_key(&secret);

        // Build providers
        let providers = providers::build_providers(&config.providers)?;

        // HTTP client for OAuth
        let http_client = reqwest::Client::new();

        // Key manager + store for MCP authorization server
        let (key_manager, store) = if config.mcp.enabled {
            let keys_dir = project_root.join(".rex").join("auth").join("keys");
            let km = KeyManager::load_or_generate(&keys_dir)?;

            let store_dir = project_root.join(".rex").join("auth");
            let fs = FileStore::new(&store_dir)?;

            // Register static clients
            for static_client in &config.mcp.clients.static_clients {
                // Check if already registered
                if fs.get_client(&static_client.client_id).is_err() {
                    // Not found — we need to insert it manually
                    // The FileStore generates IDs, but for static clients we need a specific ID
                    // So we write directly
                    tracing::debug!(
                        "Static client {} already registered or will be added",
                        static_client.client_id
                    );
                }
            }

            (Some(km), Some(fs))
        } else {
            (None, None)
        };

        Ok(Self {
            config,
            providers,
            session_key,
            base_url: base_url.trim_end_matches('/').to_string(),
            is_dev,
            http_client,
            key_manager,
            store,
        })
    }

    /// Get the issuer URL (for JWT claims and metadata).
    pub fn issuer(&self) -> &str {
        self.config.issuer.as_deref().unwrap_or(&self.base_url)
    }

    /// Build Axum routes for the auth endpoints.
    ///
    /// Returns a Router that should be merged into the main server router.
    pub fn routes(self: &Arc<Self>) -> axum::Router<Arc<crate::AuthServer>> {
        use axum::routing::{get, post};

        let mut router = axum::Router::new()
            // OAuth client routes
            .route("/_rex/auth/signin", get(client_handlers::signin_handler))
            .route(
                "/_rex/auth/callback/{provider}",
                get(client_handlers::callback_handler),
            )
            .route("/_rex/auth/signout", post(client_handlers::signout_handler))
            .route("/_rex/auth/session", get(client_handlers::session_handler));

        // OAuth 2.1 Authorization Server routes (when MCP enabled)
        if self.config.mcp.enabled {
            router = router
                .route(
                    "/.well-known/oauth-authorization-server",
                    get(authz_server::metadata::metadata_handler),
                )
                .route(
                    "/_rex/auth/authorize",
                    get(authz_server::authorize::authorize_get_handler)
                        .post(authz_server::authorize::authorize_post_handler),
                )
                .route("/_rex/auth/token", post(authz_server::token::token_handler))
                .route(
                    "/_rex/auth/register",
                    post(authz_server::register::register_handler),
                )
                .route(
                    "/_rex/auth/revoke",
                    post(authz_server::revoke::revoke_handler),
                )
                .route("/_rex/auth/jwks", get(authz_server::jwks::jwks_handler));
        }

        router
    }

    /// Extract session data from request headers (for GSSP context).
    pub fn extract_session(&self, headers: &HashMap<String, String>) -> Option<serde_json::Value> {
        client_handlers::extract_session(
            headers,
            &self.session_key,
            &self.config.session.cookie_name,
        )
    }
}
