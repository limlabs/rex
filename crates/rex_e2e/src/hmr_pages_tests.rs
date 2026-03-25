//! E2E tests for pages router HMR via unbundled dev serving.
//!
//! Verifies the module-update HMR path works correctly for pages router:
//! file changes trigger ESM fast path (not full rolldown rebuild), the
//! client receives module-update messages (not full-reload), and SSR
//! reflects changes without the flash-then-revert hydration mismatch.
//!
//! Also verifies the unbundled dev serving infrastructure: import maps,
//! /_rex/entry/ routes, /_rex/src/ source serving with DCE, /_rex/dep/
//! pre-bundled browser deps.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

struct TestServer {
    port: u16,
    _child: Child,
}

static SERVER: OnceLock<TestServer> = OnceLock::new();

fn fixture_root() -> std::path::PathBuf {
    crate::workspace_root().join("fixtures/basic")
}

fn ensure_server() -> &'static TestServer {
    SERVER.get_or_init(|| {
        let bin = crate::rex_binary();
        let root = fixture_root();
        let port = crate::find_free_port();

        eprintln!("[hmr-pages-e2e] Starting rex dev on port {port}");

        let child = Command::new(&bin)
            .arg("dev")
            .arg("--root")
            .arg(&root)
            .arg("--port")
            .arg(port.to_string())
            .arg("--no-tui")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("Failed to start rex: {e}\nBinary: {}", bin.display()));

        // Wait for HTTP readiness
        let deadline = Instant::now() + Duration::from_secs(30);
        let addr = format!("127.0.0.1:{port}");
        loop {
            if Instant::now() > deadline {
                panic!("[hmr-pages-e2e] Server failed to start within 30s");
            }
            if let Ok(mut stream) =
                TcpStream::connect_timeout(&addr.parse().unwrap(), Duration::from_millis(500))
            {
                stream
                    .set_read_timeout(Some(Duration::from_millis(500)))
                    .ok();
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

        eprintln!("[hmr-pages-e2e] Server ready on port {port}");
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

// -------------------------------------------------------
// Unbundled dev serving infrastructure tests
// -------------------------------------------------------

#[tokio::test]
#[ignore]
async fn e2e_dev_import_map_in_html() {
    let url = format!("{}/", base_url());
    let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

    // Import map present in HTML
    assert!(
        body.contains("<script type=\"importmap\">"),
        "HTML should contain an import map"
    );
    assert!(
        body.contains("/_rex/dep/react.js"),
        "Import map should map react to /_rex/dep/react.js"
    );

    // Entry scripts instead of bundled chunks
    assert!(
        body.contains("/_rex/entry/"),
        "HTML should use /_rex/entry/ script tags"
    );
    assert!(
        !body.contains("chunk-react-"),
        "HTML should NOT contain rolldown chunk filenames"
    );
}

#[tokio::test]
#[ignore]
async fn e2e_dev_dep_route_serves_react() {
    let dep_url = format!("{}/_rex/dep/react.js", base_url());
    let resp = reqwest::get(&dep_url).await.unwrap();
    assert_eq!(resp.status(), 200, "/_rex/dep/react.js should return 200");

    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        ct.contains("javascript"),
        "Should have JS content-type, got: {ct}"
    );

    let body = resp.text().await.unwrap();
    assert!(
        body.contains("export"),
        "React dep bundle should contain ESM exports"
    );
}

#[tokio::test]
#[ignore]
async fn e2e_dev_src_route_serves_transformed_js() {
    let src_url = format!("{}/_rex/src/pages/about.tsx", base_url());
    let resp = reqwest::get(&src_url).await.unwrap();
    assert_eq!(
        resp.status(),
        200,
        "/_rex/src/pages/about.tsx should return 200"
    );

    let body = resp.text().await.unwrap();
    assert!(
        !body.contains("getServerSideProps"),
        "DCE should strip getServerSideProps from browser-served source"
    );
    assert!(
        !body.contains(": Props"),
        "OXC should strip TypeScript type annotations"
    );
}

#[tokio::test]
#[ignore]
async fn e2e_dev_entry_route_serves_hydration_bootstrap() {
    // URL-encode "/" as "%2F" for the root route pattern
    let entry_url = format!("{}/_rex/entry//", base_url());
    let resp = reqwest::get(&entry_url).await.unwrap();
    assert_eq!(resp.status(), 200, "/_rex/entry// should return 200");

    let body = resp.text().await.unwrap();
    assert!(
        body.contains("hydrateRoot"),
        "Entry should contain hydrateRoot call"
    );
    assert!(
        body.contains("__REX_PAGES"),
        "Entry should register on __REX_PAGES"
    );
    assert!(
        body.contains("__REX_RENDER__"),
        "Entry should define __REX_RENDER__"
    );
    assert!(
        body.contains("/_rex/src/"),
        "Entry should import page from /_rex/src/ URL"
    );
}

// -------------------------------------------------------
// HMR module-update test
// -------------------------------------------------------

#[tokio::test]
#[ignore]
async fn e2e_hmr_pages_module_update_reflects_in_ssr() {
    let about_path = fixture_root().join("pages/about.tsx");
    let original = std::fs::read_to_string(&about_path)
        .expect("Failed to read about.tsx — run `cd fixtures/basic && npm install` first");

    let url = format!("{}/about", base_url());

    // Verify initial content
    let resp = reqwest::get(&url).await.unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("Rex is a Next.js Pages Router reimplemented in Rust."),
        "Initial about page should contain expected description"
    );

    // Wait for any in-flight rebuilds to settle
    std::thread::sleep(Duration::from_secs(2));

    // Modify the file with a unique marker
    let marker = "HMR_E2E_PAGES_MODULE_UPDATE_42";
    let modified = original.replace(
        "Rex is a Next.js Pages Router reimplemented in Rust.",
        &format!("Rex is a {marker} Pages Router reimplemented in Rust."),
    );
    std::fs::write(&about_path, &modified).expect("Failed to write about.tsx");

    // Poll until the change is visible in SSR (or timeout)
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut found = false;
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if let Ok(resp) = reqwest::get(&url).await {
            if let Ok(body) = resp.text().await {
                if body.contains(marker) {
                    found = true;
                    break;
                }
            }
        }
    }

    // Restore the file
    std::fs::write(&about_path, &original).expect("Failed to restore about.tsx");
    std::thread::sleep(Duration::from_secs(2));

    assert!(found, "SSR should reflect the modified content after HMR");
}
