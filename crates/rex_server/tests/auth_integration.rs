//! Integration tests for auth routes mounted through the RexServer router.
//!
//! These tests verify that auth routes are reachable when mounted via
//! the same merge pattern used in `RexServer::build_router_with_extra`.
//! This catches state-type mismatches and routing priority issues
//! that unit tests on AuthServer alone would miss.

use axum::routing::{any, get};
use axum::Router;
use rex_auth::config::{
    AuthConfig, ClientsConfig, McpAuthConfig, PagesConfig, SessionConfig,
};
use rex_auth::AuthServer;
use serde_json::json;
use std::sync::Arc;
use tokio::net::TcpListener;

fn temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static CTR: AtomicU64 = AtomicU64::new(0);
    let n = CTR.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("rex_integ_{label}_{n}_{}", std::process::id()))
}

fn test_auth_server(project_root: &std::path::Path, base_url: &str) -> Arc<AuthServer> {
    let config = AuthConfig {
        secret: Some("integration-test-secret".to_string()),
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
            refresh_token_ttl: 86400,
            clients: ClientsConfig {
                allow_dynamic: true,
                static_clients: vec![],
            },
        },
    };

    Arc::new(AuthServer::new(config, project_root, base_url, true).unwrap())
}

/// Build a router that mirrors the pattern in `RexServer::build_router_with_extra`.
///
/// This is the critical test: auth routes must be reachable when merged
/// into a Router<()> alongside other routes with different handler signatures.
fn build_test_router(auth: &Arc<AuthServer>) -> Router {
    // Dummy handlers mimicking the non-auth routes in build_router_with_extra
    async fn dummy_handler() -> &'static str {
        "ok"
    }

    let mut router = Router::new()
        .route("/_rex/data/{build_id}/{*path}", get(dummy_handler))
        .route("/_rex/image", get(dummy_handler))
        .route("/_rex/router.js", get(dummy_handler));

    // This is the exact pattern from server.rs line 130-133
    let auth_routes = auth.routes().with_state(auth.clone());
    router = router.merge(auth_routes);

    router
        .route("/api/{*path}", any(dummy_handler))
        .fallback(dummy_handler)
}

/// Start a server and return (base_url, client).
async fn start_server() -> (String, reqwest::Client) {
    let dir = temp_dir("server");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let base_url = format!("http://127.0.0.1:{port}");

    let auth = test_auth_server(&dir, &base_url);
    let router = build_test_router(&auth);

    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    (base_url, client)
}

// ═════════════════════════════════════════════════════════════════════
// Auth Route Reachability (NOT 404)
// ═════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_signin_route_reachable() {
    let (url, client) = start_server().await;
    let resp = client
        .get(format!("{url}/_rex/auth/signin?provider=github"))
        .send()
        .await
        .unwrap();
    // Should be 400 (unknown provider in test mode), NOT 404
    assert_ne!(
        resp.status().as_u16(),
        404,
        "/_rex/auth/signin must not return 404"
    );
    assert_eq!(resp.status(), 400); // unknown provider is 400
}

#[tokio::test]
async fn test_callback_route_reachable() {
    let (url, client) = start_server().await;
    let resp = client
        .get(format!(
            "{url}/_rex/auth/callback/github?code=test&state=test"
        ))
        .header("Cookie", "__rex_auth_state=test")
        .send()
        .await
        .unwrap();
    // 400 (unknown provider), NOT 404
    assert_ne!(
        resp.status().as_u16(),
        404,
        "/_rex/auth/callback/github must not return 404"
    );
}

#[tokio::test]
async fn test_session_route_reachable() {
    let (url, client) = start_server().await;
    let resp = client
        .get(format!("{url}/_rex/auth/session"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        200,
        "/_rex/auth/session must return 200"
    );
    let body = resp.text().await.unwrap();
    assert_eq!(body, "{}"); // no session
}

#[tokio::test]
async fn test_signout_route_reachable() {
    let (url, client) = start_server().await;
    let resp = client
        .post(format!("{url}/_rex/auth/signout"))
        .send()
        .await
        .unwrap();
    // Should be 302 redirect, NOT 404
    assert_ne!(
        resp.status().as_u16(),
        404,
        "/_rex/auth/signout must not return 404"
    );
    assert_eq!(resp.status(), 302);
}

#[tokio::test]
async fn test_metadata_route_reachable() {
    let (url, client) = start_server().await;
    let resp = client
        .get(format!(
            "{url}/.well-known/oauth-authorization-server"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        200,
        "/.well-known/oauth-authorization-server must return 200"
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["issuer"].is_string());
}

#[tokio::test]
async fn test_register_route_reachable() {
    let (url, client) = start_server().await;
    let resp = client
        .post(format!("{url}/_rex/auth/register"))
        .json(&json!({
            "client_name": "Test",
            "redirect_uris": ["http://localhost:8080/callback"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        201,
        "/_rex/auth/register must return 201"
    );
}

#[tokio::test]
async fn test_token_route_reachable() {
    let (url, client) = start_server().await;
    let resp = client
        .post(format!("{url}/_rex/auth/token"))
        .form(&[("grant_type", "authorization_code")])
        .send()
        .await
        .unwrap();
    // 400 (missing params), NOT 404
    assert_ne!(
        resp.status().as_u16(),
        404,
        "/_rex/auth/token must not return 404"
    );
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn test_revoke_route_reachable() {
    let (url, client) = start_server().await;
    let resp = client
        .post(format!("{url}/_rex/auth/revoke"))
        .form(&[("token", "test")])
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        200,
        "/_rex/auth/revoke must return 200"
    );
}

#[tokio::test]
async fn test_jwks_route_reachable() {
    let (url, client) = start_server().await;
    let resp = client
        .get(format!("{url}/_rex/auth/jwks"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        200,
        "/_rex/auth/jwks must return 200"
    );
}

#[tokio::test]
async fn test_authorize_route_reachable() {
    let (url, client) = start_server().await;
    // Register a client first so we have a valid client_id
    let reg: serde_json::Value = client
        .post(format!("{url}/_rex/auth/register"))
        .json(&json!({
            "client_name": "Test",
            "redirect_uris": ["http://localhost:8080/callback"]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let client_id = reg["client_id"].as_str().unwrap();

    let resp = client
        .get(format!(
            "{url}/_rex/auth/authorize?response_type=code&client_id={client_id}\
             &redirect_uri=http://localhost:8080/callback\
             &code_challenge=test&code_challenge_method=S256\
             &scope=tools:read&state=test"
        ))
        .send()
        .await
        .unwrap();
    // Should be 302 (owner mode auto-approve), NOT 404
    assert_ne!(
        resp.status().as_u16(),
        404,
        "/_rex/auth/authorize must not return 404"
    );
}

// ═════════════════════════════════════════════════════════════════════
// Non-auth routes still work alongside auth
// ═════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_other_rex_routes_still_work() {
    let (url, client) = start_server().await;

    let resp = client
        .get(format!("{url}/_rex/router.js"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "/_rex/router.js must still work");

    let resp = client
        .get(format!("{url}/_rex/image"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "/_rex/image must still work");
}

// ═════════════════════════════════════════════════════════════════════
// Full OAuth 2.1 flow through merged router
// ═════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_full_oauth_flow_through_server_router() {
    let (url, client) = start_server().await;

    // 1. Register client
    let reg: serde_json::Value = client
        .post(format!("{url}/_rex/auth/register"))
        .json(&json!({
            "client_name": "Flow Test",
            "redirect_uris": ["http://localhost:9999/callback"]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let client_id = reg["client_id"].as_str().unwrap();

    // 2. PKCE
    use sha2::{Digest, Sha256};
    let verifier = "test-verifier-that-is-at-least-43-characters-long-for-rfc";
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));

    use base64::Engine;

    // 3. Authorize (owner mode → auto-approve → 302 with code)
    let resp = client
        .get(format!(
            "{url}/_rex/auth/authorize?response_type=code&client_id={client_id}\
             &redirect_uri=http://localhost:9999/callback\
             &code_challenge={challenge}&code_challenge_method=S256\
             &scope=tools:read%20tools:execute&state=xyz"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 302, "Authorize should redirect");
    let location = resp.headers().get("location").unwrap().to_str().unwrap();
    let redir = url::Url::parse(location).unwrap();
    let code = redir
        .query_pairs()
        .find(|(k, _)| k == "code")
        .expect("redirect must have code param")
        .1
        .to_string();

    // 4. Token exchange
    let token_resp = client
        .post(format!("{url}/_rex/auth/token"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("client_id", client_id),
            ("redirect_uri", "http://localhost:9999/callback"),
            ("code_verifier", verifier),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(token_resp.status(), 200);
    let tokens: serde_json::Value = token_resp.json().await.unwrap();
    assert!(tokens["access_token"].is_string());
    assert!(tokens["refresh_token"].is_string());

    // 5. JWKS
    let jwks_resp = client
        .get(format!("{url}/_rex/auth/jwks"))
        .send()
        .await
        .unwrap();
    assert_eq!(jwks_resp.status(), 200);

    // 6. Revoke
    let refresh = tokens["refresh_token"].as_str().unwrap();
    let revoke_resp = client
        .post(format!("{url}/_rex/auth/revoke"))
        .form(&[("token", refresh)])
        .send()
        .await
        .unwrap();
    assert_eq!(revoke_resp.status(), 200);
}
