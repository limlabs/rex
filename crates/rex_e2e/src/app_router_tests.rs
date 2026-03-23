use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

struct TestServer {
    port: u16,
    _child: Child,
}

static APP_SERVER: OnceLock<TestServer> = OnceLock::new();

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
    // Prefer debug (matches `cargo build` default and pre-push hook)
    let debug = workspace_root.join("target/debug/rex");
    if debug.exists() {
        return debug;
    }
    let release = workspace_root.join("target/release/rex");
    if release.exists() {
        return release;
    }
    panic!("Rex binary not found. Run `cargo build` first.");
}

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("fixtures/app-router")
}

fn find_free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

fn ensure_server() -> &'static TestServer {
    APP_SERVER.get_or_init(|| {
        let bin = rex_binary();
        let root = fixture_root();
        let port = find_free_port();

        eprintln!("[e2e-app] Starting rex dev server on port {port}");
        eprintln!("[e2e-app] Root: {}", root.display());

        let child = Command::new(&bin)
            .arg("dev")
            .arg("--root")
            .arg(&root)
            .arg("--port")
            .arg(port.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("Failed to start rex: {e}"));

        // Poll with HTTP GET until the server returns a valid response.
        // The first successful connection may block while lazy init runs
        // (build + ESM loading), so use a generous per-request read timeout.
        let deadline = Instant::now() + Duration::from_secs(60);
        let addr = format!("127.0.0.1:{port}");
        loop {
            if Instant::now() > deadline {
                panic!("[e2e-app] Server failed to start within 60s on port {port}");
            }
            if let Ok(mut stream) =
                TcpStream::connect_timeout(&addr.parse().unwrap(), Duration::from_millis(500))
            {
                // Lazy init can take 10-30s in CI — give the first request enough time
                stream.set_read_timeout(Some(Duration::from_secs(45))).ok();
                let req = format!("GET / HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n");
                if stream.write_all(req.as_bytes()).is_ok() {
                    let mut buf = [0u8; 32];
                    if let Ok(n) = stream.read(&mut buf) {
                        if n > 0 && String::from_utf8_lossy(&buf[..n]).contains("HTTP/") {
                            break;
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(200));
        }

        eprintln!("[e2e-app] Server ready on port {port}");
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

#[tokio::test]
#[ignore]
async fn e2e_app_route_handler_get() {
    let url = format!("{}/api/hello", base_url());
    let resp = reqwest::get(&url).await.unwrap();
    assert_eq!(resp.status(), 200);

    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        ct.contains("application/json"),
        "Expected JSON content-type, got: {ct}"
    );

    let json: serde_json::Value = resp.json().await.unwrap();
    assert!(
        json.get("message").is_some(),
        "Missing 'message' in response: {json}"
    );
}

#[tokio::test]
#[ignore]
async fn e2e_app_route_handler_get_with_query() {
    let url = format!("{}/api/hello?name=Rex", base_url());
    let resp = reqwest::get(&url).await.unwrap();
    assert_eq!(resp.status(), 200);

    let json: serde_json::Value = resp.json().await.unwrap();
    let message = json["message"].as_str().unwrap();
    assert!(
        message.contains("Rex"),
        "Expected name in message, got: {message}"
    );
}

#[tokio::test]
#[ignore]
async fn e2e_app_route_handler_post() {
    let url = format!("{}/api/hello", base_url());
    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .json(&serde_json::json!({"key": "value"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);

    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["received"], true, "POST should return received:true");
}

#[tokio::test]
#[ignore]
async fn e2e_app_route_handler_method_not_allowed() {
    let url = format!("{}/api/hello", base_url());
    let client = reqwest::Client::new();
    let resp = client.delete(&url).send().await.unwrap();
    assert_eq!(
        resp.status(),
        405,
        "DELETE should return 405 Method Not Allowed"
    );
}
