#![allow(clippy::unwrap_used)]
//! Tests for configuration parsing, environment variable resolution,
//! and provider construction.
//!
//! These tests ensure that misconfigured providers (missing or empty
//! credentials) are caught at startup rather than producing broken
//! authorization URLs at runtime.

use rex_auth::config::{
    AuthConfig, ClientsConfig, McpAuthConfig, PagesConfig, ProviderConfig, SessionConfig,
};
use rex_auth::providers;
use serde_json::json;

// ═════════════════════════════════════════════════════════════════════
// resolve_env_vars
// ═════════════════════════════════════════════════════════════════════

#[test]
fn resolve_env_vars_literal_string() {
    let result = rex_auth::config::resolve_env_vars("my-client-id");
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "my-client-id");
}

#[test]
fn resolve_env_vars_existing_var() {
    std::env::set_var("REX_TEST_RESOLVE_EXISTING", "test-value-123");
    let result = rex_auth::config::resolve_env_vars("$REX_TEST_RESOLVE_EXISTING");
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "test-value-123");
    std::env::remove_var("REX_TEST_RESOLVE_EXISTING");
}

#[test]
fn resolve_env_vars_missing_var_returns_error() {
    std::env::remove_var("REX_TEST_DEFINITELY_NOT_SET_12345");
    let result = rex_auth::config::resolve_env_vars("$REX_TEST_DEFINITELY_NOT_SET_12345");
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("REX_TEST_DEFINITELY_NOT_SET_12345"),
        "Error should name the missing var: {err}"
    );
}

#[test]
fn resolve_env_vars_empty_var_returns_error() {
    std::env::set_var("REX_TEST_RESOLVE_EMPTY", "");
    let result = rex_auth::config::resolve_env_vars("$REX_TEST_RESOLVE_EMPTY");
    assert!(result.is_err(), "Empty env var should be treated as unset");
    std::env::remove_var("REX_TEST_RESOLVE_EMPTY");
}

#[test]
fn resolve_env_vars_dollar_only() {
    // "$" with no var name — should look up env var "" which won't exist
    let result = rex_auth::config::resolve_env_vars("$");
    assert!(result.is_err());
}

#[test]
fn resolve_env_vars_no_dollar_prefix_passes_through() {
    let result = rex_auth::config::resolve_env_vars("GITHUB_CLIENT_ID");
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "GITHUB_CLIENT_ID");
}

// ═════════════════════════════════════════════════════════════════════
// parse_auth_config — env var resolution in providers
// ═════════════════════════════════════════════════════════════════════

#[test]
fn parse_auth_config_rejects_missing_provider_env_vars() {
    std::env::remove_var("REX_TEST_MISSING_ID");
    std::env::remove_var("REX_TEST_MISSING_SECRET");

    let config_json = json!({
        "providers": [{
            "type": "github",
            "clientId": "$REX_TEST_MISSING_ID",
            "clientSecret": "$REX_TEST_MISSING_SECRET"
        }]
    });

    let result = rex_auth::config::parse_auth_config(&config_json);
    assert!(
        result.is_err(),
        "Should fail when provider env vars are missing"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("REX_TEST_MISSING_ID"),
        "Error should mention which env var is missing: {err}"
    );
}

#[test]
fn parse_auth_config_rejects_empty_provider_env_vars() {
    std::env::set_var("REX_TEST_EMPTY_ID", "");
    std::env::set_var("REX_TEST_EMPTY_SECRET", "");

    let config_json = json!({
        "providers": [{
            "type": "github",
            "clientId": "$REX_TEST_EMPTY_ID",
            "clientSecret": "$REX_TEST_EMPTY_SECRET"
        }]
    });

    let result = rex_auth::config::parse_auth_config(&config_json);
    assert!(
        result.is_err(),
        "Should fail when provider env vars are empty"
    );

    std::env::remove_var("REX_TEST_EMPTY_ID");
    std::env::remove_var("REX_TEST_EMPTY_SECRET");
}

#[test]
fn parse_auth_config_resolves_valid_provider_env_vars() {
    std::env::set_var("REX_TEST_VALID_ID", "gh-id-abc123");
    std::env::set_var("REX_TEST_VALID_SECRET", "gh-secret-xyz789");

    let config_json = json!({
        "providers": [{
            "type": "github",
            "clientId": "$REX_TEST_VALID_ID",
            "clientSecret": "$REX_TEST_VALID_SECRET"
        }]
    });

    let result = rex_auth::config::parse_auth_config(&config_json);
    assert!(result.is_ok(), "Should succeed with valid env vars");
    let config = result.unwrap();
    assert_eq!(
        config.providers[0].client_id.as_deref(),
        Some("gh-id-abc123")
    );
    assert_eq!(
        config.providers[0].client_secret.as_deref(),
        Some("gh-secret-xyz789")
    );

    std::env::remove_var("REX_TEST_VALID_ID");
    std::env::remove_var("REX_TEST_VALID_SECRET");
}

#[test]
fn parse_auth_config_secret_falls_back_gracefully() {
    std::env::remove_var("REX_TEST_SECRET_MISSING");

    let config_json = json!({
        "secret": "$REX_TEST_SECRET_MISSING",
        "providers": []
    });

    let result = rex_auth::config::parse_auth_config(&config_json);
    assert!(
        result.is_ok(),
        "Secret should fall back gracefully, not error"
    );
    let config = result.unwrap();
    assert!(
        config.secret.is_none(),
        "Unresolvable secret should become None"
    );
}

#[test]
fn parse_auth_config_literal_credentials_pass_through() {
    let config_json = json!({
        "providers": [{
            "type": "github",
            "clientId": "Iv1.abc123def456",
            "clientSecret": "deadbeef1234567890"
        }]
    });

    let result = rex_auth::config::parse_auth_config(&config_json);
    assert!(result.is_ok());
    let config = result.unwrap();
    assert_eq!(
        config.providers[0].client_id.as_deref(),
        Some("Iv1.abc123def456")
    );
}

#[test]
fn parse_auth_config_issuer_env_var_required_when_present() {
    std::env::remove_var("REX_TEST_ISSUER_MISSING");

    let config_json = json!({
        "issuer": "$REX_TEST_ISSUER_MISSING",
        "providers": []
    });

    let result = rex_auth::config::parse_auth_config(&config_json);
    assert!(result.is_err(), "Missing issuer env var should fail");
}

// ═════════════════════════════════════════════════════════════════════
// Provider from_config — empty credential rejection
// ═════════════════════════════════════════════════════════════════════

fn provider_config(provider_type: &str) -> ProviderConfig {
    ProviderConfig {
        provider_type: provider_type.to_string(),
        id: None,
        name: None,
        client_id: Some("valid-id".to_string()),
        client_secret: Some("valid-secret".to_string()),
        issuer: None,
        authorization_url: None,
        token_url: None,
        userinfo_url: None,
        scopes: None,
    }
}

fn provider_config_empty_id(provider_type: &str) -> ProviderConfig {
    ProviderConfig {
        client_id: Some(String::new()),
        ..provider_config(provider_type)
    }
}

fn provider_config_empty_secret(provider_type: &str) -> ProviderConfig {
    ProviderConfig {
        client_secret: Some(String::new()),
        ..provider_config(provider_type)
    }
}

fn provider_config_none_id(provider_type: &str) -> ProviderConfig {
    ProviderConfig {
        client_id: None,
        ..provider_config(provider_type)
    }
}

fn provider_config_none_secret(provider_type: &str) -> ProviderConfig {
    ProviderConfig {
        client_secret: None,
        ..provider_config(provider_type)
    }
}

// --- GitHub ---

#[test]
fn github_rejects_empty_client_id() {
    let config = provider_config_empty_id("github");
    let result = providers::github::GitHubProvider::from_config(&config);
    assert!(result.is_err(), "GitHub must reject empty clientId");
    let err = result.err().unwrap().to_string();
    assert!(
        err.contains("clientId"),
        "Error should mention clientId: {err}"
    );
}

#[test]
fn github_rejects_none_client_id() {
    let config = provider_config_none_id("github");
    let result = providers::github::GitHubProvider::from_config(&config);
    assert!(result.is_err(), "GitHub must reject None clientId");
}

#[test]
fn github_rejects_empty_client_secret() {
    let config = provider_config_empty_secret("github");
    let result = providers::github::GitHubProvider::from_config(&config);
    assert!(result.is_err(), "GitHub must reject empty clientSecret");
    let err = result.err().unwrap().to_string();
    assert!(
        err.contains("clientSecret"),
        "Error should mention clientSecret: {err}"
    );
}

#[test]
fn github_rejects_none_client_secret() {
    let config = provider_config_none_secret("github");
    let result = providers::github::GitHubProvider::from_config(&config);
    assert!(result.is_err());
}

#[test]
fn github_accepts_valid_credentials() {
    let config = provider_config("github");
    let result = providers::github::GitHubProvider::from_config(&config);
    assert!(result.is_ok());
}

// --- Google ---

#[test]
fn google_rejects_empty_client_id() {
    let config = provider_config_empty_id("google");
    let result = providers::google::GoogleProvider::from_config(&config);
    assert!(result.is_err(), "Google must reject empty clientId");
}

#[test]
fn google_rejects_empty_client_secret() {
    let config = provider_config_empty_secret("google");
    let result = providers::google::GoogleProvider::from_config(&config);
    assert!(result.is_err(), "Google must reject empty clientSecret");
}

#[test]
fn google_accepts_valid_credentials() {
    let config = provider_config("google");
    let result = providers::google::GoogleProvider::from_config(&config);
    assert!(result.is_ok());
}

// --- Discord ---

#[test]
fn discord_rejects_empty_client_id() {
    let config = provider_config_empty_id("discord");
    let result = providers::discord::DiscordProvider::from_config(&config);
    assert!(result.is_err(), "Discord must reject empty clientId");
}

#[test]
fn discord_rejects_empty_client_secret() {
    let config = provider_config_empty_secret("discord");
    let result = providers::discord::DiscordProvider::from_config(&config);
    assert!(result.is_err(), "Discord must reject empty clientSecret");
}

#[test]
fn discord_accepts_valid_credentials() {
    let config = provider_config("discord");
    let result = providers::discord::DiscordProvider::from_config(&config);
    assert!(result.is_ok());
}

// --- Apple ---

#[test]
fn apple_rejects_empty_client_id() {
    let config = provider_config_empty_id("apple");
    let result = providers::apple::AppleProvider::from_config(&config);
    assert!(result.is_err(), "Apple must reject empty clientId");
}

#[test]
fn apple_rejects_empty_client_secret() {
    let config = provider_config_empty_secret("apple");
    let result = providers::apple::AppleProvider::from_config(&config);
    assert!(result.is_err(), "Apple must reject empty clientSecret");
}

#[test]
fn apple_accepts_valid_credentials() {
    let config = provider_config("apple");
    let result = providers::apple::AppleProvider::from_config(&config);
    assert!(result.is_ok());
}

// --- Microsoft ---

#[test]
fn microsoft_rejects_empty_client_id() {
    let config = provider_config_empty_id("microsoft");
    let result = providers::microsoft::MicrosoftProvider::from_config(&config);
    assert!(result.is_err(), "Microsoft must reject empty clientId");
}

#[test]
fn microsoft_rejects_empty_client_secret() {
    let config = provider_config_empty_secret("microsoft");
    let result = providers::microsoft::MicrosoftProvider::from_config(&config);
    assert!(result.is_err(), "Microsoft must reject empty clientSecret");
}

#[test]
fn microsoft_accepts_valid_credentials() {
    let config = provider_config("microsoft");
    let result = providers::microsoft::MicrosoftProvider::from_config(&config);
    assert!(result.is_ok());
}

// --- Twitter ---

#[test]
fn twitter_rejects_empty_client_id() {
    let config = provider_config_empty_id("twitter");
    let result = providers::twitter::TwitterProvider::from_config(&config);
    assert!(result.is_err(), "Twitter must reject empty clientId");
}

#[test]
fn twitter_rejects_empty_client_secret() {
    let config = provider_config_empty_secret("twitter");
    let result = providers::twitter::TwitterProvider::from_config(&config);
    assert!(result.is_err(), "Twitter must reject empty clientSecret");
}

#[test]
fn twitter_accepts_valid_credentials() {
    let config = provider_config("twitter");
    let result = providers::twitter::TwitterProvider::from_config(&config);
    assert!(result.is_ok());
}

// --- Generic OIDC ---

fn oidc_config() -> ProviderConfig {
    ProviderConfig {
        provider_type: "oidc".to_string(),
        id: Some("my-oidc".to_string()),
        name: Some("My OIDC".to_string()),
        client_id: Some("valid-id".to_string()),
        client_secret: Some("valid-secret".to_string()),
        issuer: Some("https://auth.example.com".to_string()),
        authorization_url: None,
        token_url: None,
        userinfo_url: None,
        scopes: None,
    }
}

#[test]
fn oidc_rejects_empty_client_id() {
    let config = ProviderConfig {
        client_id: Some(String::new()),
        ..oidc_config()
    };
    let result = providers::oidc::GenericOidcProvider::from_config(&config);
    assert!(result.is_err(), "OIDC must reject empty clientId");
}

#[test]
fn oidc_rejects_empty_client_secret() {
    let config = ProviderConfig {
        client_secret: Some(String::new()),
        ..oidc_config()
    };
    let result = providers::oidc::GenericOidcProvider::from_config(&config);
    assert!(result.is_err(), "OIDC must reject empty clientSecret");
}

#[test]
fn oidc_rejects_empty_issuer() {
    let config = ProviderConfig {
        issuer: Some(String::new()),
        ..oidc_config()
    };
    let result = providers::oidc::GenericOidcProvider::from_config(&config);
    assert!(result.is_err(), "OIDC must reject empty issuer");
}

#[test]
fn oidc_rejects_empty_id() {
    let config = ProviderConfig {
        id: Some(String::new()),
        ..oidc_config()
    };
    let result = providers::oidc::GenericOidcProvider::from_config(&config);
    assert!(result.is_err(), "OIDC must reject empty provider id");
}

#[test]
fn oidc_accepts_valid_config() {
    let result = providers::oidc::GenericOidcProvider::from_config(&oidc_config());
    assert!(result.is_ok());
}

// --- Generic OAuth ---

fn oauth_config() -> ProviderConfig {
    ProviderConfig {
        provider_type: "oauth".to_string(),
        id: Some("my-oauth".to_string()),
        name: Some("My OAuth".to_string()),
        client_id: Some("valid-id".to_string()),
        client_secret: Some("valid-secret".to_string()),
        issuer: None,
        authorization_url: Some("https://auth.example.com/authorize".to_string()),
        token_url: Some("https://auth.example.com/token".to_string()),
        userinfo_url: Some("https://auth.example.com/userinfo".to_string()),
        scopes: None,
    }
}

#[test]
fn oauth_rejects_empty_client_id() {
    let config = ProviderConfig {
        client_id: Some(String::new()),
        ..oauth_config()
    };
    let result = providers::oauth::GenericOAuthProvider::from_config(&config);
    assert!(result.is_err(), "OAuth must reject empty clientId");
}

#[test]
fn oauth_rejects_empty_client_secret() {
    let config = ProviderConfig {
        client_secret: Some(String::new()),
        ..oauth_config()
    };
    let result = providers::oauth::GenericOAuthProvider::from_config(&config);
    assert!(result.is_err(), "OAuth must reject empty clientSecret");
}

#[test]
fn oauth_rejects_empty_authorization_url() {
    let config = ProviderConfig {
        authorization_url: Some(String::new()),
        ..oauth_config()
    };
    let result = providers::oauth::GenericOAuthProvider::from_config(&config);
    assert!(result.is_err(), "OAuth must reject empty authorizationUrl");
}

#[test]
fn oauth_rejects_empty_token_url() {
    let config = ProviderConfig {
        token_url: Some(String::new()),
        ..oauth_config()
    };
    let result = providers::oauth::GenericOAuthProvider::from_config(&config);
    assert!(result.is_err(), "OAuth must reject empty tokenUrl");
}

#[test]
fn oauth_rejects_empty_id() {
    let config = ProviderConfig {
        id: Some(String::new()),
        ..oauth_config()
    };
    let result = providers::oauth::GenericOAuthProvider::from_config(&config);
    assert!(result.is_err(), "OAuth must reject empty provider id");
}

#[test]
fn oauth_accepts_valid_config() {
    let result = providers::oauth::GenericOAuthProvider::from_config(&oauth_config());
    assert!(result.is_ok());
}

// ═════════════════════════════════════════════════════════════════════
// build_providers — the factory function
// ═════════════════════════════════════════════════════════════════════

#[test]
fn build_providers_rejects_unknown_type() {
    let configs = vec![ProviderConfig {
        provider_type: "myspace".to_string(),
        id: None,
        name: None,
        client_id: Some("id".to_string()),
        client_secret: Some("secret".to_string()),
        issuer: None,
        authorization_url: None,
        token_url: None,
        userinfo_url: None,
        scopes: None,
    }];

    let result = providers::build_providers(&configs);
    assert!(result.is_err());
    let err = result.err().unwrap().to_string();
    assert!(
        err.contains("myspace"),
        "Error should name the unknown provider: {err}"
    );
}

#[test]
fn build_providers_propagates_empty_credential_errors() {
    let configs = vec![ProviderConfig {
        provider_type: "github".to_string(),
        id: None,
        name: None,
        client_id: Some(String::new()),
        client_secret: Some("secret".to_string()),
        issuer: None,
        authorization_url: None,
        token_url: None,
        userinfo_url: None,
        scopes: None,
    }];

    let result = providers::build_providers(&configs);
    assert!(
        result.is_err(),
        "build_providers must propagate empty credential errors"
    );
}

#[test]
fn build_providers_succeeds_with_valid_configs() {
    let configs = vec![
        provider_config("github"),
        provider_config("google"),
        provider_config("discord"),
    ];

    let result = providers::build_providers(&configs);
    assert!(result.is_ok());
    let providers = result.unwrap();
    assert_eq!(providers.len(), 3);
    assert!(providers.contains_key("github"));
    assert!(providers.contains_key("google"));
    assert!(providers.contains_key("discord"));
}

#[test]
fn build_providers_uses_effective_id_as_key() {
    let configs = vec![ProviderConfig {
        provider_type: "github".to_string(),
        id: Some("my-github".to_string()),
        name: None,
        client_id: Some("valid-id".to_string()),
        client_secret: Some("valid-secret".to_string()),
        issuer: None,
        authorization_url: None,
        token_url: None,
        userinfo_url: None,
        scopes: None,
    }];

    let result = providers::build_providers(&configs);
    assert!(result.is_ok());
    let providers = result.unwrap();
    assert!(
        providers.contains_key("my-github"),
        "Should use custom id as key"
    );
    assert!(
        !providers.contains_key("github"),
        "Should not use type as key when custom id is set"
    );
}

// ═════════════════════════════════════════════════════════════════════
// End-to-end: parse_auth_config → build_providers
// ═════════════════════════════════════════════════════════════════════

#[test]
fn end_to_end_unset_env_vars_fail_before_provider_construction() {
    std::env::remove_var("REX_TEST_E2E_GITHUB_ID");
    std::env::remove_var("REX_TEST_E2E_GITHUB_SECRET");

    let config_json = json!({
        "providers": [{
            "type": "github",
            "clientId": "$REX_TEST_E2E_GITHUB_ID",
            "clientSecret": "$REX_TEST_E2E_GITHUB_SECRET"
        }]
    });

    // This should fail at parse_auth_config, not at build_providers
    let result = rex_auth::config::parse_auth_config(&config_json);
    assert!(
        result.is_err(),
        "Missing env vars must be caught at config parse time, \
         before reaching the provider constructor"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("REX_TEST_E2E_GITHUB_ID"),
        "Error must name the missing env var: {err}"
    );
}

#[test]
fn end_to_end_valid_env_vars_produce_working_provider() {
    std::env::set_var("REX_TEST_E2E_VALID_ID", "Iv1.test123");
    std::env::set_var("REX_TEST_E2E_VALID_SECRET", "secret456");

    let config_json = json!({
        "providers": [{
            "type": "github",
            "clientId": "$REX_TEST_E2E_VALID_ID",
            "clientSecret": "$REX_TEST_E2E_VALID_SECRET"
        }]
    });

    let config = rex_auth::config::parse_auth_config(&config_json).unwrap();
    let providers = providers::build_providers(&config.providers).unwrap();
    let github = providers
        .get("github")
        .expect("GitHub provider should exist");

    // Verify the authorization URL contains the resolved client_id
    let auth_url = github.authorization_url("state123", "http://localhost:3000/callback");
    assert!(
        auth_url.contains("client_id=Iv1.test123"),
        "Authorization URL must contain resolved client_id: {auth_url}"
    );
    assert!(
        !auth_url.contains("client_id=&"),
        "Authorization URL must never have empty client_id: {auth_url}"
    );
    assert!(
        !auth_url.contains("client_id=$"),
        "Authorization URL must never have unresolved $VAR: {auth_url}"
    );

    std::env::remove_var("REX_TEST_E2E_VALID_ID");
    std::env::remove_var("REX_TEST_E2E_VALID_SECRET");
}

#[test]
fn end_to_end_authorization_url_never_has_empty_client_id() {
    // This is the exact scenario that caused the production bug.
    // Even if somehow an empty client_id slips through config parsing,
    // the provider constructor must reject it.
    let config = ProviderConfig {
        provider_type: "github".to_string(),
        id: None,
        name: None,
        client_id: Some(String::new()), // Empty string — the bug
        client_secret: Some("valid-secret".to_string()),
        issuer: None,
        authorization_url: None,
        token_url: None,
        userinfo_url: None,
        scopes: None,
    };

    let result = providers::github::GitHubProvider::from_config(&config);
    assert!(
        result.is_err(),
        "Provider construction must reject empty client_id. \
         An empty client_id leads to broken authorization URLs like \
         'client_id=&redirect_uri=...' which GitHub returns as 404."
    );
}

// ═════════════════════════════════════════════════════════════════════
// AuthServer::new — full initialization pipeline
// ═════════════════════════════════════════════════════════════════════

#[test]
fn auth_server_new_rejects_missing_provider_credentials() {
    let config = AuthConfig {
        secret: Some("test-secret".to_string()),
        issuer: None,
        providers: vec![ProviderConfig {
            provider_type: "github".to_string(),
            id: None,
            name: None,
            client_id: Some(String::new()),
            client_secret: Some("secret".to_string()),
            issuer: None,
            authorization_url: None,
            token_url: None,
            userinfo_url: None,
            scopes: None,
        }],
        session: SessionConfig {
            max_age: 86400,
            cookie_name: "__rex_session".to_string(),
        },
        pages: PagesConfig::default(),
        mcp: McpAuthConfig {
            enabled: false,
            scopes: vec![],
            access_token_ttl: 3600,
            refresh_token_ttl: 86400,
            clients: ClientsConfig {
                allow_dynamic: true,
                static_clients: vec![],
            },
        },
    };

    let dir = std::env::temp_dir().join("rex_test_auth_server_reject");
    let result = rex_auth::AuthServer::new(config, &dir, "http://localhost:3000", true);
    assert!(
        result.is_err(),
        "AuthServer::new must fail when a provider has empty credentials"
    );
}

// ═════════════════════════════════════════════════════════════════════
// ProviderConfig::effective_id
// ═════════════════════════════════════════════════════════════════════

#[test]
fn effective_id_returns_custom_id_when_set() {
    let config = ProviderConfig {
        provider_type: "github".to_string(),
        id: Some("my-github".to_string()),
        name: None,
        client_id: None,
        client_secret: None,
        issuer: None,
        authorization_url: None,
        token_url: None,
        userinfo_url: None,
        scopes: None,
    };
    assert_eq!(config.effective_id(), "my-github");
}

#[test]
fn effective_id_falls_back_to_type() {
    let config = ProviderConfig {
        provider_type: "github".to_string(),
        id: None,
        name: None,
        client_id: None,
        client_secret: None,
        issuer: None,
        authorization_url: None,
        token_url: None,
        userinfo_url: None,
        scopes: None,
    };
    assert_eq!(config.effective_id(), "github");
}

// ═════════════════════════════════════════════════════════════════════
// derive_key and resolve_secret
// ═════════════════════════════════════════════════════════════════════

#[test]
fn derive_key_is_deterministic() {
    let key1 = rex_auth::config::derive_key("my-secret");
    let key2 = rex_auth::config::derive_key("my-secret");
    assert_eq!(key1, key2);
}

#[test]
fn derive_key_different_secrets_produce_different_keys() {
    let key1 = rex_auth::config::derive_key("secret-a");
    let key2 = rex_auth::config::derive_key("secret-b");
    assert_ne!(key1, key2);
}

#[test]
fn resolve_secret_prefers_config_value() {
    let dir = std::env::temp_dir().join("rex_test_resolve_secret_config");
    let result = rex_auth::config::resolve_secret(Some("my-config-secret"), &dir);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "my-config-secret");
}

#[test]
fn resolve_secret_skips_empty_config() {
    std::env::set_var("REX_AUTH_SECRET", "from-env");
    let dir = std::env::temp_dir().join("rex_test_resolve_secret_empty");
    let result = rex_auth::config::resolve_secret(Some(""), &dir);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "from-env");
    std::env::remove_var("REX_AUTH_SECRET");
}
