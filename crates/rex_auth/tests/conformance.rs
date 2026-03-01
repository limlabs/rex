#![allow(clippy::unwrap_used)]
//! RFC conformance tests for the rex_auth OAuth 2.1 Authorization Server.
//!
//! Validates compliance with:
//! - RFC 8414 (Authorization Server Metadata)
//! - RFC 7591 (Dynamic Client Registration)
//! - RFC 7009 (Token Revocation)
//! - OAuth 2.1 draft requirements

use axum::Router;
use base64::Engine;
use rex_auth::config::{AuthConfig, ClientsConfig, McpAuthConfig, PagesConfig, SessionConfig};
use rex_auth::AuthServer;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::net::TcpListener;

/// Unique temp dir per test invocation.
fn temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static CTR: AtomicU64 = AtomicU64::new(0);
    let n = CTR.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("rex_conf_{label}_{n}_{}", std::process::id()))
}

/// Build a test AuthServer with MCP enabled & no OAuth providers (owner mode).
fn test_auth_server(project_root: &std::path::Path, base_url: &str) -> Arc<AuthServer> {
    let config = AuthConfig {
        secret: Some("conformance-test-secret-value".to_string()),
        issuer: Some(base_url.to_string()),
        providers: vec![],
        session: SessionConfig {
            max_age: 86400,
            cookie_name: "__rex_session".to_string(),
        },
        pages: PagesConfig::default(),
        mcp: McpAuthConfig {
            enabled: true,
            scopes: vec!["tools:read".to_string(), "tools:execute".to_string()],
            access_token_ttl: 3600,
            refresh_token_ttl: 2_592_000,
            clients: ClientsConfig {
                allow_dynamic: true,
                static_clients: vec![],
            },
        },
    };

    Arc::new(AuthServer::new(config, project_root, base_url, true).expect("AuthServer::new"))
}

/// Start a real HTTP server on a random port, returning (base_url, client).
async fn start_auth_server() -> (String, reqwest::Client) {
    let dir = temp_dir("server");
    // Bind before creating AuthServer so we know the port.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let base_url = format!("http://127.0.0.1:{port}");

    let auth = test_auth_server(&dir, &base_url);
    let router: Router = auth.routes().with_state(auth);

    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    // Give the server a moment to start.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    (base_url, client)
}

// ── Helpers ──────────────────────────────────────────────────────────

fn generate_random_verifier() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn compute_s256_challenge(verifier: &str) -> String {
    let hash = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash)
}

/// Register a client and return its client_id.
async fn register_client(base: &str, client: &reqwest::Client) -> String {
    let resp = client
        .post(format!("{base}/_rex/auth/register"))
        .json(&json!({
            "client_name": "Conformance Test Client",
            "redirect_uris": ["http://localhost:9999/callback"],
            "token_endpoint_auth_method": "none"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    body["client_id"].as_str().unwrap().to_string()
}

/// Complete the authorization flow (owner-mode auto-approve) and return (code, verifier).
async fn get_auth_code(base: &str, client: &reqwest::Client, client_id: &str) -> (String, String) {
    let verifier = generate_random_verifier();
    let challenge = compute_s256_challenge(&verifier);

    let resp = client
        .get(format!(
            "{base}/_rex/auth/authorize?response_type=code&client_id={client_id}\
             &redirect_uri=http://localhost:9999/callback\
             &code_challenge={challenge}&code_challenge_method=S256\
             &scope=tools:read%20tools:execute&state=test-state"
        ))
        .send()
        .await
        .unwrap();

    // Owner mode: auto-approve → 302 redirect with code
    assert_eq!(
        resp.status(),
        302,
        "Expected redirect; got {}",
        resp.status()
    );
    let location = resp.headers().get("location").unwrap().to_str().unwrap();
    let url = url::Url::parse(&format!(
        "http://localhost:9999{}",
        location
            .strip_prefix("http://localhost:9999")
            .unwrap_or(location)
    ))
    .unwrap();
    let code = url
        .query_pairs()
        .find(|(k, _)| k == "code")
        .unwrap()
        .1
        .to_string();

    (code, verifier)
}

/// Exchange an auth code for tokens, returning the JSON response.
async fn exchange_code(
    base: &str,
    client: &reqwest::Client,
    code: &str,
    client_id: &str,
    verifier: &str,
) -> serde_json::Value {
    let resp = client
        .post(format!("{base}/_rex/auth/token"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("client_id", client_id),
            ("redirect_uri", "http://localhost:9999/callback"),
            ("code_verifier", verifier),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    resp.json().await.unwrap()
}

// ═════════════════════════════════════════════════════════════════════
// RFC 8414 — Authorization Server Metadata
// ═════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_rfc8414_metadata_required_fields() {
    let (url, client) = start_auth_server().await;
    let meta: serde_json::Value = client
        .get(format!("{url}/.well-known/oauth-authorization-server"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // Required by RFC 8414 Section 2
    assert!(meta["issuer"].is_string(), "issuer is required");
    assert!(meta["authorization_endpoint"].is_string());
    assert!(meta["token_endpoint"].is_string());
    assert!(meta["response_types_supported"].is_array());

    // Issuer must match exactly — no trailing slash, no query/fragment
    let issuer = meta["issuer"].as_str().unwrap();
    assert!(!issuer.ends_with('/'), "issuer must not end with /");
    assert!(!issuer.contains('?'), "issuer must not have query");
    assert!(!issuer.contains('#'), "issuer must not have fragment");

    // All endpoint URLs must be absolute
    for field in [
        "authorization_endpoint",
        "token_endpoint",
        "registration_endpoint",
        "revocation_endpoint",
        "jwks_uri",
    ] {
        if let Some(endpoint) = meta[field].as_str() {
            assert!(
                endpoint.starts_with("http"),
                "{field} must be an absolute URL, got {endpoint}"
            );
            assert!(
                url::Url::parse(endpoint).is_ok(),
                "{field} must be a valid URL"
            );
        }
    }
}

#[tokio::test]
async fn test_rfc8414_metadata_content_type() {
    let (url, client) = start_auth_server().await;
    let resp = client
        .get(format!("{url}/.well-known/oauth-authorization-server"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        ct.contains("application/json"),
        "content-type must be application/json, got {ct}"
    );
}

#[tokio::test]
async fn test_rfc8414_oauth21_requirements() {
    let (url, client) = start_auth_server().await;
    let meta: serde_json::Value = client
        .get(format!("{url}/.well-known/oauth-authorization-server"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // OAuth 2.1 requirements
    let grants = meta["grant_types_supported"].as_array().unwrap();
    assert!(
        !grants.contains(&json!("implicit")),
        "OAuth 2.1 prohibits implicit grant"
    );
    assert!(
        !grants.contains(&json!("password")),
        "OAuth 2.1 prohibits password grant"
    );

    let response_types = meta["response_types_supported"].as_array().unwrap();
    assert!(
        response_types.contains(&json!("code")),
        "Must support authorization code"
    );
    assert!(
        !response_types.contains(&json!("token")),
        "Must not support implicit token response"
    );

    // PKCE required — only S256
    let challenge_methods = meta["code_challenge_methods_supported"].as_array().unwrap();
    assert!(
        challenge_methods.contains(&json!("S256")),
        "Must support S256 PKCE"
    );
    assert!(
        !challenge_methods.contains(&json!("plain")),
        "Must not support plain PKCE"
    );

    // Token endpoint auth: public clients (no secret)
    let auth_methods = meta["token_endpoint_auth_methods_supported"]
        .as_array()
        .unwrap();
    assert!(
        auth_methods.contains(&json!("none")),
        "Must support public clients"
    );
}

#[tokio::test]
async fn test_rfc8414_metadata_cache_control() {
    let (url, client) = start_auth_server().await;
    let resp = client
        .get(format!("{url}/.well-known/oauth-authorization-server"))
        .send()
        .await
        .unwrap();
    let cc = resp
        .headers()
        .get("cache-control")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        cc.contains("max-age"),
        "Metadata should be cacheable with max-age"
    );
}

// ═════════════════════════════════════════════════════════════════════
// RFC 7591 — Dynamic Client Registration
// ═════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_rfc7591_registration_response_format() {
    let (url, client) = start_auth_server().await;
    let resp = client
        .post(format!("{url}/_rex/auth/register"))
        .json(&json!({
            "client_name": "Test MCP Client",
            "redirect_uris": ["http://localhost:8080/callback"],
            "grant_types": ["authorization_code"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 201, "Registration must return 201 Created");

    let body: serde_json::Value = resp.json().await.unwrap();

    // Required fields per RFC 7591 Section 3.2.1
    assert!(body["client_id"].is_string(), "Must return client_id");
    assert!(
        !body["client_id"].as_str().unwrap().is_empty(),
        "client_id must be non-empty"
    );

    // Returned fields should echo back the registration request
    assert_eq!(body["client_name"], "Test MCP Client");
    let uris = body["redirect_uris"].as_array().unwrap();
    assert!(uris.contains(&json!("http://localhost:8080/callback")));
}

#[tokio::test]
async fn test_rfc7591_registration_cache_control() {
    let (url, client) = start_auth_server().await;
    let resp = client
        .post(format!("{url}/_rex/auth/register"))
        .json(&json!({
            "client_name": "Cache Test",
            "redirect_uris": ["http://localhost:8080/callback"]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 201);
    let cc = resp
        .headers()
        .get("cache-control")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        cc.contains("no-store"),
        "Registration response must have Cache-Control: no-store"
    );
}

#[tokio::test]
async fn test_rfc7591_registration_invalid_redirect_uri() {
    let (url, client) = start_auth_server().await;

    // javascript: URI must be rejected
    let resp = client
        .post(format!("{url}/_rex/auth/register"))
        .json(&json!({ "redirect_uris": ["javascript:alert(1)"] }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);

    // data: URI must be rejected
    let resp = client
        .post(format!("{url}/_rex/auth/register"))
        .json(&json!({ "redirect_uris": ["data:text/html,<script>alert(1)</script>"] }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);

    // Empty redirect_uris must be rejected
    let resp = client
        .post(format!("{url}/_rex/auth/register"))
        .json(&json!({ "redirect_uris": [] }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn test_rfc7591_registration_disabled() {
    // Create a server with dynamic registration disabled
    let dir = temp_dir("reg_disabled");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let base_url = format!("http://127.0.0.1:{port}");

    let config = AuthConfig {
        secret: Some("test-secret".to_string()),
        issuer: Some(base_url.clone()),
        providers: vec![],
        session: SessionConfig::default(),
        pages: PagesConfig::default(),
        mcp: McpAuthConfig {
            enabled: true,
            scopes: vec!["tools:read".to_string()],
            access_token_ttl: 3600,
            refresh_token_ttl: 86400,
            clients: ClientsConfig {
                allow_dynamic: false,
                static_clients: vec![],
            },
        },
    };

    let auth = Arc::new(AuthServer::new(config, &dir, &base_url, true).unwrap());
    let router: Router = auth.routes().with_state(auth);

    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base_url}/_rex/auth/register"))
        .json(&json!({
            "client_name": "Should Fail",
            "redirect_uris": ["http://localhost:8080/callback"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        403,
        "Dynamic registration should be rejected when disabled"
    );
}

// ═════════════════════════════════════════════════════════════════════
// RFC 7009 — Token Revocation
// ═════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_rfc7009_revocation_always_200() {
    let (url, client) = start_auth_server().await;

    // Per RFC 7009, revocation always returns 200, even for invalid tokens
    let resp = client
        .post(format!("{url}/_rex/auth/revoke"))
        .form(&[("token", "nonexistent-token")])
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        200,
        "Revocation must return 200 per RFC 7009"
    );
}

#[tokio::test]
async fn test_rfc7009_revocation_no_token() {
    let (url, client) = start_auth_server().await;

    // Missing token should still return 200
    let resp = client
        .post(format!("{url}/_rex/auth/revoke"))
        .form(&[("dummy", "value")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn test_rfc7009_revocation_invalidates_refresh_token() {
    let (url, client) = start_auth_server().await;

    // Get a real refresh token
    let client_id = register_client(&url, &client).await;
    let (code, verifier) = get_auth_code(&url, &client, &client_id).await;
    let tokens = exchange_code(&url, &client, &code, &client_id, &verifier).await;
    let refresh_token = tokens["refresh_token"].as_str().unwrap();

    // Revoke it
    let resp = client
        .post(format!("{url}/_rex/auth/revoke"))
        .form(&[("token", refresh_token)])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Using revoked refresh token should fail
    let resp = client
        .post(format!("{url}/_rex/auth/token"))
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", &client_id),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "invalid_grant");
}

#[tokio::test]
async fn test_rfc7009_revocation_cache_headers() {
    let (url, client) = start_auth_server().await;
    let resp = client
        .post(format!("{url}/_rex/auth/revoke"))
        .form(&[("token", "x")])
        .send()
        .await
        .unwrap();
    let cc = resp
        .headers()
        .get("cache-control")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(cc.contains("no-store"), "Revocation must have no-store");
}

// ═════════════════════════════════════════════════════════════════════
// Authorization Endpoint
// ═════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_authorize_requires_response_type_code() {
    let (url, client) = start_auth_server().await;
    let client_id = register_client(&url, &client).await;

    // Missing response_type
    let resp = client
        .get(format!(
            "{url}/_rex/auth/authorize?client_id={client_id}\
             &redirect_uri=http://localhost:9999/callback\
             &code_challenge=abc&code_challenge_method=S256"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);

    // Wrong response_type (implicit)
    let resp = client
        .get(format!(
            "{url}/_rex/auth/authorize?response_type=token&client_id={client_id}\
             &redirect_uri=http://localhost:9999/callback\
             &code_challenge=abc&code_challenge_method=S256"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn test_authorize_requires_pkce() {
    let (url, client) = start_auth_server().await;
    let client_id = register_client(&url, &client).await;

    // Missing code_challenge
    let resp = client
        .get(format!(
            "{url}/_rex/auth/authorize?response_type=code&client_id={client_id}\
             &redirect_uri=http://localhost:9999/callback"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "invalid_request");
}

#[tokio::test]
async fn test_authorize_rejects_plain_pkce() {
    let (url, client) = start_auth_server().await;
    let client_id = register_client(&url, &client).await;

    // code_challenge_method=plain must be rejected
    let resp = client
        .get(format!(
            "{url}/_rex/auth/authorize?response_type=code&client_id={client_id}\
             &redirect_uri=http://localhost:9999/callback\
             &code_challenge=test&code_challenge_method=plain"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

// ═════════════════════════════════════════════════════════════════════
// Token Endpoint
// ═════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_token_missing_grant_type() {
    let (url, client) = start_auth_server().await;

    let resp = client
        .post(format!("{url}/_rex/auth/token"))
        .form(&[("code", "x")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "invalid_request");
}

#[tokio::test]
async fn test_token_unsupported_grant_type() {
    let (url, client) = start_auth_server().await;

    let resp = client
        .post(format!("{url}/_rex/auth/token"))
        .form(&[("grant_type", "password")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "unsupported_grant_type");
}

#[tokio::test]
async fn test_token_response_headers() {
    let (url, client) = start_auth_server().await;
    let client_id = register_client(&url, &client).await;
    let (code, verifier) = get_auth_code(&url, &client, &client_id).await;

    let resp = client
        .post(format!("{url}/_rex/auth/token"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("client_id", &client_id),
            ("redirect_uri", "http://localhost:9999/callback"),
            ("code_verifier", &verifier),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Verify response headers per spec
    assert_eq!(resp.headers().get("cache-control").unwrap(), "no-store");
    assert_eq!(resp.headers().get("pragma").unwrap(), "no-cache");
}

#[tokio::test]
async fn test_token_response_body_format() {
    let (url, client) = start_auth_server().await;
    let client_id = register_client(&url, &client).await;
    let (code, verifier) = get_auth_code(&url, &client, &client_id).await;
    let tokens = exchange_code(&url, &client, &code, &client_id, &verifier).await;

    assert!(tokens["access_token"].is_string());
    assert_eq!(tokens["token_type"], "Bearer");
    assert!(tokens["expires_in"].is_number());
    assert!(tokens["refresh_token"].is_string());
    assert!(tokens["scope"].is_string());
}

// ═════════════════════════════════════════════════════════════════════
// JWT Format
// ═════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_jwt_has_three_parts() {
    let (url, client) = start_auth_server().await;
    let client_id = register_client(&url, &client).await;
    let (code, verifier) = get_auth_code(&url, &client, &client_id).await;
    let tokens = exchange_code(&url, &client, &code, &client_id, &verifier).await;

    let jwt = tokens["access_token"].as_str().unwrap();
    let parts: Vec<&str> = jwt.split('.').collect();
    assert_eq!(parts.len(), 3, "JWT must have 3 parts");
}

#[tokio::test]
async fn test_jwt_header_claims() {
    let (url, client) = start_auth_server().await;
    let client_id = register_client(&url, &client).await;
    let (code, verifier) = get_auth_code(&url, &client, &client_id).await;
    let tokens = exchange_code(&url, &client, &code, &client_id, &verifier).await;

    let jwt = tokens["access_token"].as_str().unwrap();
    let parts: Vec<&str> = jwt.split('.').collect();

    // Decode header
    let header: serde_json::Value = serde_json::from_slice(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[0])
            .unwrap(),
    )
    .unwrap();
    assert_eq!(header["alg"], "RS256");
    assert!(header["kid"].is_string(), "JWT must have kid header");
}

#[tokio::test]
async fn test_jwt_payload_claims() {
    let (url, client) = start_auth_server().await;
    let client_id = register_client(&url, &client).await;
    let (code, verifier) = get_auth_code(&url, &client, &client_id).await;
    let tokens = exchange_code(&url, &client, &code, &client_id, &verifier).await;

    let jwt = tokens["access_token"].as_str().unwrap();
    let parts: Vec<&str> = jwt.split('.').collect();

    // Decode claims
    let claims: serde_json::Value = serde_json::from_slice(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[1])
            .unwrap(),
    )
    .unwrap();

    assert!(claims["iss"].is_string(), "JWT must have iss");
    assert!(claims["sub"].is_string(), "JWT must have sub");
    assert!(claims["exp"].is_number(), "JWT must have exp");
    assert!(claims["iat"].is_number(), "JWT must have iat");
    assert!(claims["jti"].is_string(), "JWT must have jti");
    assert_eq!(claims["scope"], "tools:read tools:execute");
}

#[tokio::test]
async fn test_jwt_verifiable_against_jwks() {
    let (url, client) = start_auth_server().await;
    let client_id = register_client(&url, &client).await;
    let (code, verifier) = get_auth_code(&url, &client, &client_id).await;
    let tokens = exchange_code(&url, &client, &code, &client_id, &verifier).await;

    let jwt = tokens["access_token"].as_str().unwrap();
    let parts: Vec<&str> = jwt.split('.').collect();
    let header: serde_json::Value = serde_json::from_slice(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[0])
            .unwrap(),
    )
    .unwrap();
    let kid = header["kid"].as_str().unwrap();

    // Fetch JWKS
    let jwks: serde_json::Value = client
        .get(format!("{url}/_rex/auth/jwks"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let keys = jwks["keys"].as_array().unwrap();
    let matching_key = keys.iter().find(|k| k["kid"].as_str() == Some(kid));
    assert!(matching_key.is_some(), "JWKS must contain the signing key");
}

// ═════════════════════════════════════════════════════════════════════
// JWKS Endpoint
// ═════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_jwks_response_headers() {
    let (url, client) = start_auth_server().await;
    let resp = client
        .get(format!("{url}/_rex/auth/jwks"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(ct.contains("application/json"));
    let cc = resp
        .headers()
        .get("cache-control")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(cc.contains("max-age"));
}

#[tokio::test]
async fn test_jwks_contains_valid_keys() {
    let (url, client) = start_auth_server().await;
    let jwks: serde_json::Value = client
        .get(format!("{url}/_rex/auth/jwks"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let keys = jwks["keys"].as_array().unwrap();
    assert!(!keys.is_empty());
    for key in keys {
        assert_eq!(key["kty"], "RSA");
        assert_eq!(key["alg"], "RS256");
        assert_eq!(key["use"], "sig");
        assert!(key["kid"].is_string());
        assert!(key["n"].is_string(), "modulus must be present");
        assert!(key["e"].is_string(), "exponent must be present");
        // Must NOT leak private key components
        assert!(
            key.get("d").is_none(),
            "private exponent must not be in JWK"
        );
        assert!(key.get("p").is_none());
        assert!(key.get("q").is_none());
    }
}

// ═════════════════════════════════════════════════════════════════════
// Full OAuth 2.1 Flow Conformance
// ═════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_oauth21_full_flow_conformance() {
    let (url, client) = start_auth_server().await;

    // 1. Register client
    let reg: serde_json::Value = client
        .post(format!("{url}/_rex/auth/register"))
        .json(&json!({
            "client_name": "Full Flow Test",
            "redirect_uris": ["http://localhost:9999/callback"],
            "token_endpoint_auth_method": "none"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let client_id = reg["client_id"].as_str().unwrap();

    // 2. Generate PKCE
    let verifier = generate_random_verifier();
    let challenge = compute_s256_challenge(&verifier);

    // 3. Authorization request (owner mode → auto-approve)
    let auth_resp = client
        .get(format!(
            "{url}/_rex/auth/authorize?response_type=code&client_id={client_id}\
             &redirect_uri=http://localhost:9999/callback\
             &code_challenge={challenge}&code_challenge_method=S256\
             &scope=tools:read%20tools:execute&state=test-state-123"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(auth_resp.status(), 302);

    let location = auth_resp
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap();
    let redirect_url = url::Url::parse(location).unwrap();
    let code = redirect_url
        .query_pairs()
        .find(|(k, _)| k == "code")
        .unwrap()
        .1
        .to_string();
    let returned_state = redirect_url
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .to_string();
    assert_eq!(
        returned_state, "test-state-123",
        "State must be echoed back"
    );

    // 4. Token exchange
    let token_resp = client
        .post(format!("{url}/_rex/auth/token"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("client_id", client_id),
            ("redirect_uri", "http://localhost:9999/callback"),
            ("code_verifier", verifier.as_str()),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(token_resp.status(), 200);
    assert_eq!(
        token_resp.headers().get("cache-control").unwrap(),
        "no-store"
    );
    assert_eq!(token_resp.headers().get("pragma").unwrap(), "no-cache");

    let tokens: serde_json::Value = token_resp.json().await.unwrap();
    assert!(tokens["access_token"].is_string());
    assert_eq!(tokens["token_type"], "Bearer");
    assert!(tokens["expires_in"].is_number());
    assert!(tokens["refresh_token"].is_string());

    // 5. Refresh token flow
    let refresh = tokens["refresh_token"].as_str().unwrap();
    let refresh_resp = client
        .post(format!("{url}/_rex/auth/token"))
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh),
            ("client_id", client_id),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(refresh_resp.status(), 200);

    let new_tokens: serde_json::Value = refresh_resp.json().await.unwrap();
    assert!(new_tokens["access_token"].is_string());
    assert_ne!(
        new_tokens["access_token"], tokens["access_token"],
        "Must issue new access token on refresh"
    );
}
