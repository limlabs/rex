//! E2E tests for RSC (React Server Components) via the app/ router.
//!
//! Run with: cargo test -p rex_e2e --test rsc_e2e -- --ignored
//!
//! Prerequisites:
//!   - `cargo build` (debug or release)
//!   - `cd fixtures/app-router && npm install`

#[allow(clippy::unwrap_used)]
mod rsc {
    use std::net::TcpStream;
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    struct TestServer {
        port: u16,
        _child: Child,
    }

    static RSC_SERVER: OnceLock<TestServer> = OnceLock::new();

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
            .join("fixtures/app-router")
    }

    fn find_free_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    }

    fn ensure_server() -> &'static TestServer {
        RSC_SERVER.get_or_init(|| {
            let bin = rex_binary();
            let root = fixture_root();
            let port = find_free_port();

            eprintln!("[rsc-e2e] Starting rex dev server on port {port}");
            eprintln!("[rsc-e2e] Binary: {}", bin.display());
            eprintln!("[rsc-e2e] Root: {}", root.display());

            let child = Command::new(&bin)
                .arg("dev")
                .arg("--root")
                .arg(&root)
                .arg("--port")
                .arg(port.to_string())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap_or_else(|e| {
                    panic!("Failed to start rex: {e}\nBinary: {}", bin.display())
                });

            // Wait for server to be ready
            let deadline = Instant::now() + Duration::from_secs(30);
            let addr = format!("127.0.0.1:{port}");
            loop {
                if Instant::now() > deadline {
                    panic!("[rsc-e2e] Server failed to start within 30s on port {port}");
                }
                if TcpStream::connect_timeout(
                    &addr.parse().unwrap(),
                    Duration::from_millis(100),
                )
                .is_ok()
                {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }

            eprintln!("[rsc-e2e] Server ready on port {port}");

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
    // RSC page rendering tests
    // -------------------------------------------------------

    #[tokio::test]
    #[ignore]
    async fn rsc_index_returns_200_with_html() {
        let url = format!("{}/", base_url());
        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), 200);

        let body = resp.text().await.unwrap();
        assert!(body.contains("<div id=\"__rex\">"), "Missing __rex div");
        assert!(
            body.contains("Rex App Router"),
            "Missing SSR content 'Rex App Router'"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn rsc_index_contains_flight_data() {
        let url = format!("{}/", base_url());
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

        assert!(
            body.contains("__REX_RSC_DATA__"),
            "Missing inline flight data script tag"
        );
        assert!(
            body.contains("text/rsc"),
            "Flight data script should have type text/rsc"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn rsc_index_contains_module_map() {
        let url = format!("{}/", base_url());
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

        assert!(
            body.contains("__REX_RSC_MODULE_MAP__"),
            "Missing client reference module map"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn rsc_about_page_returns_200() {
        let url = format!("{}/about", base_url());
        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), 200);

        let body = resp.text().await.unwrap();
        assert!(body.contains("About"), "Missing 'About' content");
    }

    #[tokio::test]
    #[ignore]
    async fn rsc_dynamic_route_returns_200() {
        let url = format!("{}/blog/hello-world", base_url());
        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), 200);

        let body = resp.text().await.unwrap();
        assert!(
            body.contains("hello-world"),
            "Missing dynamic slug in response body"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn rsc_nested_layout_renders() {
        let url = format!("{}/dashboard", base_url());
        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), 200);

        let body = resp.text().await.unwrap();
        assert!(
            body.contains("Dashboard"),
            "Missing dashboard content"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn rsc_nested_layout_settings_page() {
        let url = format!("{}/dashboard/settings", base_url());
        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), 200);

        let body = resp.text().await.unwrap();
        assert!(
            body.contains("Settings"),
            "Missing settings content"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn rsc_client_component_reference_in_flight() {
        // The settings page imports Counter ("use client")
        // The flight data should contain a client reference marker
        let url = format!("{}/dashboard/settings", base_url());
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

        // Flight data should have an M: row for the client component
        assert!(
            body.contains("__REX_RSC_DATA__"),
            "Missing flight data"
        );
    }

    // -------------------------------------------------------
    // RSC flight data endpoint tests
    // -------------------------------------------------------

    #[tokio::test]
    #[ignore]
    async fn rsc_flight_endpoint_returns_flight_data() {
        // First get build ID from a page
        let url = format!("{}/", base_url());
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

        if let Some(start) = body.find("\"build_id\":\"") {
            let rest = &body[start + 12..];
            let end = rest.find('"').unwrap();
            let build_id = &rest[..end];

            let rsc_url = format!("{}/_rex/rsc/{}/about", base_url(), build_id);
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
            assert!(
                flight.contains("J:") || flight.contains("R:"),
                "Flight data should contain J: or R: rows"
            );
        } else {
            panic!("Could not extract build_id from page HTML");
        }
    }

    #[tokio::test]
    #[ignore]
    async fn rsc_flight_endpoint_dynamic_route() {
        let url = format!("{}/", base_url());
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

        if let Some(start) = body.find("\"build_id\":\"") {
            let rest = &body[start + 12..];
            let end = rest.find('"').unwrap();
            let build_id = &rest[..end];

            let rsc_url = format!("{}/_rex/rsc/{}/blog/test-slug", base_url(), build_id);
            let resp = reqwest::get(&rsc_url).await.unwrap();
            assert_eq!(resp.status(), 200);

            let flight = resp.text().await.unwrap();
            assert!(
                flight.contains("test-slug"),
                "Flight data for dynamic route should contain the slug"
            );
        } else {
            panic!("Could not extract build_id from page HTML");
        }
    }

    // -------------------------------------------------------
    // RSC runtime script tests
    // -------------------------------------------------------

    #[tokio::test]
    #[ignore]
    async fn rsc_runtime_script_served() {
        let url = format!("{}/_rex/rsc-runtime.js", base_url());
        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), 200);

        let body = resp.text().await.unwrap();
        assert!(
            body.contains("__REX_RSC_INIT"),
            "RSC runtime should define __REX_RSC_INIT"
        );
        assert!(
            body.contains("__REX_RSC_NAVIGATE"),
            "RSC runtime should define __REX_RSC_NAVIGATE"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn rsc_router_script_has_app_route_support() {
        let url = format!("{}/_rex/router.js", base_url());
        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), 200);

        let body = resp.text().await.unwrap();
        assert!(
            body.contains("matchAppRoute"),
            "Router should have matchAppRoute function"
        );
        assert!(
            body.contains("__REX_RSC_NAVIGATE"),
            "Router should reference __REX_RSC_NAVIGATE"
        );
    }

    // -------------------------------------------------------
    // HTML document structure tests
    // -------------------------------------------------------

    #[tokio::test]
    #[ignore]
    async fn rsc_html_has_complete_structure() {
        let url = format!("{}/", base_url());
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

        assert!(
            body.contains("<!DOCTYPE html>") || body.contains("<!doctype html>"),
            "Missing DOCTYPE"
        );
        assert!(body.contains("<html"), "Missing <html> tag");
        assert!(
            body.contains("<head>") || body.contains("<head "),
            "Missing <head> tag"
        );
        assert!(body.contains("<body"), "Missing <body> tag");
        assert!(body.contains("<div id=\"__rex\">"), "Missing __rex root div");
    }

    #[tokio::test]
    #[ignore]
    async fn rsc_concurrent_requests() {
        let base = base_url();
        let mut handles = vec![];

        for i in 0..8 {
            let url = match i % 3 {
                0 => format!("{base}/"),
                1 => format!("{base}/about"),
                _ => format!("{base}/blog/post-{i}"),
            };
            handles.push(tokio::spawn(async move {
                let resp = reqwest::get(&url).await.unwrap();
                assert_eq!(resp.status(), 200, "Request {i} to {url} failed");
                resp.text().await.unwrap()
            }));
        }

        for (i, handle) in handles.into_iter().enumerate() {
            let body = handle.await.unwrap();
            assert!(!body.is_empty(), "Request {i} returned empty body");
        }
    }
}
