//! E2E tests for `rex live` — on-demand compilation with multi-project mounting.
//!
//! Uses `fixtures/live/` which contains two sub-projects (app-a, app-b),
//! each with their own `pages/` directory and `node_modules` symlink.
//!
//! Run with: cargo test -p rex_e2e --test live_e2e -- --ignored
//!
//! Prerequisites:
//!   - `cargo build --release --features build` (or debug)
//!   - `fixtures/live/app-a/node_modules` and `fixtures/live/app-b/node_modules` exist

#[allow(clippy::unwrap_used)]
mod live {
    use std::net::TcpStream;
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    struct LiveServer {
        port: u16,
        _child: Child,
    }

    static SERVER: OnceLock<LiveServer> = OnceLock::new();

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
            .join("fixtures/live")
    }

    fn find_free_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    }

    fn ensure_server() -> &'static LiveServer {
        SERVER.get_or_init(|| {
            let bin = rex_binary();
            let root = fixture_root();
            let port = find_free_port();

            eprintln!("[live-e2e] Starting rex live on port {port}");
            eprintln!("[live-e2e] Binary: {}", bin.display());

            let child = Command::new(&bin)
                .arg("live")
                .arg("-m")
                .arg(format!("/={}", root.join("app-a").display()))
                .arg("-m")
                .arg(format!("/b={}", root.join("app-b").display()))
                .arg("--port")
                .arg(port.to_string())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap_or_else(|e| panic!("Failed to start rex live: {e}"));

            // Wait for server to accept connections
            let deadline = Instant::now() + Duration::from_secs(30);
            let addr = format!("127.0.0.1:{port}");
            loop {
                if Instant::now() > deadline {
                    panic!("[live-e2e] Server failed to start within 30s on port {port}");
                }
                if TcpStream::connect_timeout(&addr.parse().unwrap(), Duration::from_millis(100))
                    .is_ok()
                {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }

            eprintln!("[live-e2e] Server ready on port {port}");

            LiveServer {
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
    // App A (mounted at /)
    // -------------------------------------------------------

    #[tokio::test]
    #[ignore]
    async fn live_app_a_index_returns_200_with_ssr() {
        let url = format!("{}/", base_url());
        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), 200);

        let body = resp.text().await.unwrap();
        assert!(body.contains("Live App A"), "Missing SSR content");
        assert!(body.contains("live-a"), "Missing GSSP app identifier");
        assert!(
            body.contains("<div id=\"__rex\">"),
            "Missing __rex root div"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn live_app_a_about_returns_200() {
        let url = format!("{}/about", base_url());
        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), 200);

        let body = resp.text().await.unwrap();
        assert!(body.contains("About Live A"), "Missing about content");
    }

    #[tokio::test]
    #[ignore]
    async fn live_app_a_404_for_unknown_route() {
        let url = format!("{}/nonexistent", base_url());
        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), 404);
    }

    // -------------------------------------------------------
    // App B (mounted at /b)
    // -------------------------------------------------------

    #[tokio::test]
    #[ignore]
    async fn live_app_b_index_returns_200_with_ssr() {
        let url = format!("{}/b", base_url());
        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), 200);

        let body = resp.text().await.unwrap();
        assert!(body.contains("Live App B"), "Missing SSR content");
        assert!(body.contains("live-b"), "Missing GSSP app identifier");
    }

    // -------------------------------------------------------
    // Cross-project isolation
    // -------------------------------------------------------

    #[tokio::test]
    #[ignore]
    async fn live_projects_are_isolated() {
        // App A should not leak into App B's content and vice versa
        let a_body = reqwest::get(&format!("{}/", base_url()))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let b_body = reqwest::get(&format!("{}/b", base_url()))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();

        assert!(a_body.contains("Live App A"));
        assert!(
            !a_body.contains("Live App B"),
            "App A should not contain App B content"
        );

        assert!(b_body.contains("Live App B"));
        assert!(
            !b_body.contains("Live App A"),
            "App B should not contain App A content"
        );
    }

    // -------------------------------------------------------
    // On-demand compilation (cache behavior)
    // -------------------------------------------------------

    #[tokio::test]
    #[ignore]
    async fn live_second_request_uses_cache() {
        // First request triggers compilation; second should be faster (cached).
        // We verify by checking both return 200 with correct content.
        let url = format!("{}/", base_url());

        let resp1 = reqwest::get(&url).await.unwrap();
        assert_eq!(resp1.status(), 200);
        let body1 = resp1.text().await.unwrap();

        let resp2 = reqwest::get(&url).await.unwrap();
        assert_eq!(resp2.status(), 200);
        let body2 = resp2.text().await.unwrap();

        // Both should have the same structure (SSR content present)
        assert!(body1.contains("Live App A"));
        assert!(body2.contains("Live App A"));
    }

    // -------------------------------------------------------
    // Concurrent requests across projects
    // -------------------------------------------------------

    #[tokio::test]
    #[ignore]
    async fn live_concurrent_requests() {
        let base = base_url();
        let mut handles = vec![];

        for i in 0..6 {
            let url = if i % 2 == 0 {
                format!("{base}/")
            } else {
                format!("{base}/b")
            };
            handles.push(tokio::spawn(async move {
                let resp = reqwest::get(&url).await.unwrap();
                assert_eq!(resp.status(), 200, "Concurrent request {i} failed");
                resp.text().await.unwrap()
            }));
        }

        for (i, handle) in handles.into_iter().enumerate() {
            let body = handle.await.unwrap();
            assert!(!body.is_empty(), "Request {i} returned empty body");
            if i % 2 == 0 {
                assert!(body.contains("Live App A"));
            } else {
                assert!(body.contains("Live App B"));
            }
        }
    }

    // -------------------------------------------------------
    // HTML document structure
    // -------------------------------------------------------

    #[tokio::test]
    #[ignore]
    async fn live_html_document_structure() {
        let url = format!("{}/", base_url());
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

        assert!(
            body.contains("<!DOCTYPE html>") || body.contains("<!doctype html>"),
            "Missing DOCTYPE"
        );
        assert!(body.contains("<html"), "Missing <html>");
        assert!(
            body.contains("<head>") || body.contains("<head "),
            "Missing <head>"
        );
        assert!(body.contains("<body"), "Missing <body>");
        assert!(
            body.contains("<div id=\"__rex\">"),
            "Missing __rex root div"
        );
        assert!(body.contains("__REX_DATA__"), "Missing __REX_DATA__");
    }

    // -------------------------------------------------------
    // Graceful shutdown
    // -------------------------------------------------------

    #[tokio::test]
    #[ignore]
    async fn live_graceful_shutdown() {
        let bin = rex_binary();
        let root = fixture_root();
        let port = find_free_port();

        let mut child = Command::new(&bin)
            .arg("live")
            .arg("-m")
            .arg(format!("/={}", root.join("app-a").display()))
            .arg("--port")
            .arg(port.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        // Wait for HTTP 200
        let url = format!("http://127.0.0.1:{port}/");
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if Instant::now() > deadline {
                child.kill().ok();
                panic!("Live shutdown test: server failed to become ready");
            }
            if let Ok(resp) = reqwest::get(&url).await {
                if resp.status() == 200 {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }

        // SIGTERM
        #[cfg(unix)]
        #[allow(unsafe_code)]
        unsafe {
            libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
        }
        #[cfg(not(unix))]
        {
            child.kill().ok();
        }

        let exit_deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            if Instant::now() > exit_deadline {
                child.kill().ok();
                panic!("Live server did not shut down within 5s");
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        let addr: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        assert!(
            TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_err(),
            "Port should be closed after shutdown"
        );
    }
}
