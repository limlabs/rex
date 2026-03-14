//! E2E tests for RSC Context API support via the `fixtures/context` fixture.
//!
//! Run with: cargo test -p rex_e2e --test rsc_context_e2e -- --ignored
//!
//! Prerequisites:
//!   - `cargo build` (debug or release)
//!   - No `npm install` needed — this is a zero-config fixture

#[allow(clippy::unwrap_used)]
mod context {
    use std::net::TcpStream;
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    struct TestServer {
        port: u16,
        _child: Child,
    }

    static CONTEXT_SERVER: OnceLock<TestServer> = OnceLock::new();

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
            .join("fixtures/context")
    }

    fn find_free_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    }

    fn ensure_server() -> &'static TestServer {
        CONTEXT_SERVER.get_or_init(|| {
            let bin = rex_binary();
            let root = fixture_root();
            let port = find_free_port();

            eprintln!("[context-e2e] Starting rex dev server on port {port}");
            eprintln!("[context-e2e] Binary: {}", bin.display());
            eprintln!("[context-e2e] Root: {}", root.display());

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
                    panic!("[context-e2e] Server failed to start within 30s on port {port}");
                }
                if TcpStream::connect_timeout(&addr.parse().unwrap(), Duration::from_millis(100))
                    .is_ok()
                {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }

            eprintln!("[context-e2e] Server ready on port {port}");

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

    /// Helper: extract build_id from page HTML
    fn extract_build_id(body: &str) -> Option<String> {
        let start = body.find("\"build_id\":\"")?;
        let rest = &body[start + 12..];
        let end = rest.find('"')?;
        Some(rest[..end].to_string())
    }

    // -------------------------------------------------------
    // Context provider rendering tests
    // -------------------------------------------------------

    #[tokio::test]
    #[ignore]
    async fn context_index_returns_200() {
        let url = format!("{}/", base_url());
        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), 200, "Context index page should return 200");

        let body = resp.text().await.unwrap();
        assert!(
            body.contains("Context Test"),
            "Missing 'Context Test' heading in index page"
        );
        assert!(
            body.contains("Current theme:"),
            "Missing theme display — ThemeProvider should pass value to ThemeDisplay via context"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn context_nested_providers_render() {
        let url = format!("{}/nested", base_url());
        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), 200, "Nested context page should return 200");

        let body = resp.text().await.unwrap();
        assert!(
            body.contains("Nested Context Test"),
            "Missing 'Nested Context Test' heading"
        );
        assert!(
            body.contains("Current theme:"),
            "Missing theme display in nested context page"
        );
        assert!(
            body.contains("User:"),
            "Missing auth display in nested context page"
        );
    }

    // -------------------------------------------------------
    // Flight data tests
    // -------------------------------------------------------

    #[tokio::test]
    #[ignore]
    async fn context_flight_has_client_refs() {
        let url = format!("{}/", base_url());
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

        let build_id = extract_build_id(&body).expect("Could not extract build_id from page HTML");

        let rsc_url = format!("{}/_rex/rsc/{}/", base_url(), build_id);
        let resp = reqwest::get(&rsc_url).await.unwrap();
        assert_eq!(resp.status(), 200);

        let ct = resp
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            ct.contains("text/x-component"),
            "Expected text/x-component content-type, got: {ct}"
        );

        let flight = resp.text().await.unwrap();
        // Flight data should contain client reference rows for the provider/consumer
        assert!(
            flight.contains(":I[") || flight.contains("[\"$\""),
            "Flight data should contain client reference rows for context components"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn context_page_has_module_map() {
        let url = format!("{}/", base_url());
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

        assert!(
            body.contains("__REX_RSC_MODULE_MAP__"),
            "Context page should have client reference module map"
        );
    }

    // -------------------------------------------------------
    // HTML structure tests
    // -------------------------------------------------------

    #[tokio::test]
    #[ignore]
    async fn context_html_has_complete_structure() {
        let url = format!("{}/", base_url());
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

        assert!(
            body.contains("<!DOCTYPE html>") || body.contains("<!doctype html>"),
            "Missing DOCTYPE"
        );
        assert!(body.contains("<html"), "Missing <html> tag");
        assert!(
            body.contains("<div id=\"__rex\">"),
            "Missing __rex root div"
        );
        assert!(
            body.contains("__REX_RSC_DATA__"),
            "Missing inline flight data"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn context_client_chunks_serve_200() {
        let url = format!("{}/", base_url());
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

        // Extract script srcs
        let mut search = body.as_str();
        let mut chunk_urls = vec![];
        while let Some(start) = search.find("src=\"") {
            let rest = &search[start + 5..];
            if let Some(end) = rest.find('"') {
                let src = &rest[..end];
                if src.contains("/_rex/static/") && src.ends_with(".js") {
                    chunk_urls.push(src.to_string());
                }
            }
            search = &search[start + 5..];
        }

        assert!(
            !chunk_urls.is_empty(),
            "Page should have at least one client chunk script tag"
        );

        for src in &chunk_urls {
            let chunk_url = format!("{}{}", base_url(), src);
            let resp = reqwest::get(&chunk_url).await.unwrap();
            assert_eq!(
                resp.status(),
                200,
                "Client chunk returned {} for URL: {}",
                resp.status(),
                chunk_url
            );
        }
    }
}
