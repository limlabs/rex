// E2E tests for auth + MCP integration.
//
// Uses fixtures/auth-mcp which has auth enabled with no providers (owner mode),
// so the authorize endpoint auto-approves without needing real OAuth credentials.
//
// Run with: cargo test -p rex_e2e -- --ignored e2e_auth

use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

struct TestServer {
    port: u16,
    _child: Child,
}

static AUTH_SERVER: OnceLock<TestServer> = OnceLock::new();

fn rex_binary() -> PathBuf {
    if let Ok(bin) = std::env::var("REX_BIN") {
        return PathBuf::from(bin);
    }

    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();

    let release = workspace_root.join("target/release/rex");
    if release.exists() {
        return release;
    }

    let debug = workspace_root.join("target/debug/rex");
    if debug.exists() {
        return debug;
    }

    panic!(
        "Rex binary not found. Run `cargo build` or `cargo build --release` first.\n\
         Or set REX_BIN=/path/to/rex"
    );
}

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("fixtures/auth-mcp")
}

fn find_free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

fn ensure_server() -> &'static TestServer {
    AUTH_SERVER.get_or_init(|| {
        let bin = rex_binary();
        let root = fixture_root();
        let port = find_free_port();

        eprintln!("[e2e-auth] Starting rex dev server on port {port}");
        eprintln!("[e2e-auth] Binary: {}", bin.display());
        eprintln!("[e2e-auth] Root: {}", root.display());

        let child = Command::new(&bin)
            .arg("dev")
            .arg("--root")
            .arg(&root)
            .arg("--port")
            .arg(port.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("Failed to start rex: {e}\nBinary: {}", bin.display()));

        // Wait for server to be ready
        let deadline = Instant::now() + Duration::from_secs(30);
        let addr = format!("127.0.0.1:{port}");
        loop {
            if Instant::now() > deadline {
                panic!("[e2e-auth] Server failed to start within 30s on port {port}");
            }
            if TcpStream::connect_timeout(&addr.parse().unwrap(), Duration::from_millis(100))
                .is_ok()
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        eprintln!("[e2e-auth] Server ready on port {port}");

        TestServer {
            port,
            _child: child,
        }
    })
}

fn base_url() -> String {
    let server = ensure_server();
    format!("http://127.0.0.1:{}", server.port)
}

/// Generate PKCE code_verifier and S256 code_challenge.
fn generate_pkce() -> (String, String) {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    use sha2::{Digest, Sha256};

    // code_verifier: 43-128 chars of [A-Z / a-z / 0-9 / - . _ ~]
    let verifier: String = (0..64)
        .map(|i| {
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~"[i % 66] as char
        })
        .collect();

    let hash = Sha256::digest(verifier.as_bytes());
    let challenge = URL_SAFE_NO_PAD.encode(hash);

    (verifier, challenge)
}

// -------------------------------------------------------
// Auth metadata + session tests
// -------------------------------------------------------

#[tokio::test]
#[ignore]
async fn e2e_auth_metadata_endpoint() {
    let url = format!("{}/.well-known/oauth-authorization-server", base_url());
    let resp = reqwest::get(&url).await.unwrap();
    assert_eq!(resp.status(), 200);

    let json: serde_json::Value = resp.json().await.unwrap();
    assert!(json.get("issuer").is_some(), "Missing issuer");
    assert!(
        json.get("token_endpoint").is_some(),
        "Missing token_endpoint"
    );
    assert!(
        json.get("authorization_endpoint").is_some(),
        "Missing authorization_endpoint"
    );
    assert!(json.get("jwks_uri").is_some(), "Missing jwks_uri");
}

#[tokio::test]
#[ignore]
async fn e2e_auth_jwks_endpoint() {
    let url = format!("{}/_rex/auth/jwks", base_url());
    let resp = reqwest::get(&url).await.unwrap();
    assert_eq!(resp.status(), 200);

    let json: serde_json::Value = resp.json().await.unwrap();
    let keys = json["keys"]
        .as_array()
        .expect("JWKS should have keys array");
    assert!(!keys.is_empty(), "JWKS should have at least one key");
    assert_eq!(keys[0]["kty"], "RSA", "Key type should be RSA");
    assert_eq!(keys[0]["alg"], "RS256", "Algorithm should be RS256");
}

#[tokio::test]
#[ignore]
async fn e2e_auth_session_unauthenticated() {
    let url = format!("{}/_rex/auth/session", base_url());
    let resp = reqwest::get(&url).await.unwrap();
    assert_eq!(resp.status(), 200);

    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["status"], "unauthenticated");
    assert!(
        json["user"].is_null(),
        "user should be null when unauthenticated"
    );
}

// -------------------------------------------------------
// MCP auth gate tests
// -------------------------------------------------------

#[tokio::test]
#[ignore]
async fn e2e_mcp_without_token_returns_401() {
    let url = format!("{}/mcp", base_url());
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {}
    });

    let resp = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 401, "MCP without token should return 401");
    assert!(
        resp.headers().get("www-authenticate").is_some(),
        "Should include WWW-Authenticate header"
    );
}

#[tokio::test]
#[ignore]
async fn e2e_mcp_with_invalid_token_returns_401() {
    let url = format!("{}/mcp", base_url());
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {}
    });

    let resp = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Authorization", "Bearer garbage-token")
        .json(&body)
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        401,
        "MCP with invalid token should return 401"
    );
    let www_auth = resp
        .headers()
        .get("www-authenticate")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        www_auth.contains("invalid_token"),
        "Should indicate invalid_token in WWW-Authenticate, got: {www_auth}"
    );
}

// -------------------------------------------------------
// Full OAuth → MCP flow
// -------------------------------------------------------

#[tokio::test]
#[ignore]
async fn e2e_mcp_full_oauth_flow() {
    let base = base_url();
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    // Step 1: Register a dynamic client
    let reg_resp = http
        .post(format!("{base}/_rex/auth/register"))
        .json(&serde_json::json!({
            "client_name": "E2E Test Client",
            "redirect_uris": ["http://localhost:9999/callback"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        reg_resp.status(),
        201,
        "Client registration should return 201"
    );

    let reg_json: serde_json::Value = reg_resp.json().await.unwrap();
    let client_id = reg_json["client_id"]
        .as_str()
        .expect("registration should return client_id");

    // Step 2: Generate PKCE
    let (code_verifier, code_challenge) = generate_pkce();

    // Step 3: Authorize (owner mode → auto-approves → 302 with code)
    let auth_url = format!(
        "{base}/_rex/auth/authorize?response_type=code&client_id={client_id}\
         &redirect_uri=http%3A%2F%2Flocalhost%3A9999%2Fcallback\
         &code_challenge={code_challenge}&code_challenge_method=S256\
         &scope=tools%3Aread+tools%3Aexecute"
    );
    let auth_resp = http.get(&auth_url).send().await.unwrap();
    assert_eq!(
        auth_resp.status(),
        302,
        "Authorize should redirect with 302, got {}",
        auth_resp.status()
    );

    let location = auth_resp
        .headers()
        .get("location")
        .expect("302 should have Location header")
        .to_str()
        .unwrap();

    let redirect_url = url::Url::parse(location).expect("Location should be a valid URL");
    let code = redirect_url
        .query_pairs()
        .find(|(k, _)| k == "code")
        .map(|(_, v)| v.to_string())
        .expect("Redirect should include code parameter");

    // Step 4: Exchange code for tokens
    let token_resp = http
        .post(format!("{base}/_rex/auth/token"))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(format!(
            "grant_type=authorization_code&code={}&client_id={}&redirect_uri={}&code_verifier={}",
            urlencoded(&code),
            urlencoded(client_id),
            urlencoded("http://localhost:9999/callback"),
            urlencoded(&code_verifier),
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(
        token_resp.status(),
        200,
        "Token exchange should return 200, got {}",
        token_resp.status()
    );

    let token_json: serde_json::Value = token_resp.json().await.unwrap();
    let access_token = token_json["access_token"]
        .as_str()
        .expect("Token response should include access_token");
    assert_eq!(token_json["token_type"], "Bearer");
    assert!(token_json.get("refresh_token").is_some());

    // Step 5: MCP initialize with valid token
    let init_resp = http
        .post(format!("{base}/mcp"))
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {access_token}"))
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": { "name": "e2e-test", "version": "0.1" }
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        init_resp.status(),
        200,
        "MCP initialize with valid token should return 200"
    );

    let init_json: serde_json::Value = init_resp.json().await.unwrap();
    assert!(
        init_json.get("result").is_some(),
        "initialize should return result, got: {init_json}"
    );

    // Step 6: MCP tools/list with valid token
    let list_resp = http
        .post(format!("{base}/mcp"))
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {access_token}"))
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(list_resp.status(), 200);

    let list_json: serde_json::Value = list_resp.json().await.unwrap();
    let tools = list_json["result"]["tools"]
        .as_array()
        .expect("tools/list should return tools array");
    assert!(
        tools.iter().any(|t| t["name"] == "echo"),
        "Should include 'echo' tool, got: {tools:?}"
    );

    // Step 7: MCP tools/call echo with valid token
    let call_resp = http
        .post(format!("{base}/mcp"))
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {access_token}"))
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "echo",
                "arguments": { "message": "hello from e2e" }
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(call_resp.status(), 200);

    let call_json: serde_json::Value = call_resp.json().await.unwrap();
    assert!(
        call_json.get("error").is_none(),
        "tools/call should not error: {call_json}"
    );
    let text = call_json["result"]["content"][0]["text"]
        .as_str()
        .expect("should have text content");
    let payload: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(
        payload["echo"], "hello from e2e",
        "Echo tool should return the input message"
    );
}

fn urlencoded(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}
