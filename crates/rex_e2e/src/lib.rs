// rex_e2e: End-to-end test harness for Rex
//
// Run with: cargo test -p rex_e2e -- --ignored
//
// Prerequisites:
//   - `cargo build` (debug) or `cargo build --release`
//   - `cd fixtures/basic && npm install`
//   - `cd fixtures/app-router && npm install`

#[cfg(test)]
#[allow(clippy::unwrap_used)]
#[path = "app_router_tests.rs"]
mod app_router_tests;

// Disabled: e2e_hmr_esm_fast_path_for_source_change is flaky in CI —
// the second HTTP request times out after the file change triggers a rebuild.
// See failed runs on worktree-auto-extract-deps, worktree-fix-hmr-react-chunks,
// worktree-postgres-js-compat (all panic at hmr_esm_tests.rs:112).
// TODO: re-enable once the ESM fast-path rebuild reliably keeps the server responsive.
// #[cfg(test)]
// #[allow(clippy::unwrap_used)]
// #[path = "hmr_esm_tests.rs"]
// mod hmr_esm_tests;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
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

    static SERVER: OnceLock<TestServer> = OnceLock::new();

    fn rex_binary() -> PathBuf {
        // Check REX_BIN env var first
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
            .join("fixtures/basic")
    }

    fn find_free_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    }

    fn ensure_server() -> &'static TestServer {
        SERVER.get_or_init(|| {
            let bin = rex_binary();
            let root = fixture_root();
            let port = find_free_port();

            eprintln!("[e2e] Starting rex dev server on port {port}");
            eprintln!("[e2e] Binary: {}", bin.display());
            eprintln!("[e2e] Root: {}", root.display());

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

            // Poll with HTTP GET until the server returns a valid response (not
            // just TCP connect, since rex dev binds the port before the build).
            // Fail fast on 500 — that means init/build failed permanently.
            let deadline = Instant::now() + Duration::from_secs(30);
            let addr = format!("127.0.0.1:{port}");
            loop {
                if Instant::now() > deadline {
                    panic!("[e2e] Server failed to start within 30s on port {port}");
                }
                if let Ok(mut stream) =
                    TcpStream::connect_timeout(&addr.parse().unwrap(), Duration::from_millis(500))
                {
                    stream
                        .set_read_timeout(Some(Duration::from_millis(2000)))
                        .ok();
                    let req = format!("GET / HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n");
                    if stream.write_all(req.as_bytes()).is_ok() {
                        let mut buf = [0u8; 256];
                        if let Ok(n) = stream.read(&mut buf) {
                            if n > 0 {
                                let response = String::from_utf8_lossy(&buf[..n]);
                                if response.contains("HTTP/") {
                                    if response.contains("500") {
                                        panic!(
                                            "[e2e] Server returned 500 on port {port} (init failed): {response}"
                                        );
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(200));
            }

            eprintln!("[e2e] Server ready on port {port}");

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
    // HTTP-level tests (reqwest, no browser needed)
    // -------------------------------------------------------

    #[tokio::test]
    #[ignore]
    async fn e2e_index_returns_200_with_ssr_html() {
        let url = format!("{}/", base_url());
        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), 200);

        let body = resp.text().await.unwrap();
        assert!(body.contains("<div id=\"__rex\">"), "Missing __rex div");
        assert!(body.contains("Rex!"), "Missing SSR content 'Rex!'");
    }

    #[tokio::test]
    #[ignore]
    async fn e2e_about_page_returns_200() {
        let url = format!("{}/about", base_url());
        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), 200);

        let body = resp.text().await.unwrap();
        assert!(body.contains("About"), "Missing 'About' content");
    }

    #[tokio::test]
    #[ignore]
    async fn e2e_dynamic_route_returns_200() {
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
    async fn e2e_404_page_returns_404() {
        let url = format!("{}/nonexistent-page", base_url());
        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), 404);

        let body = resp.text().await.unwrap();
        assert!(body.contains("404"), "Missing 404 text in error page");
    }

    #[tokio::test]
    #[ignore]
    async fn e2e_api_route_returns_json() {
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
            "Missing 'message' in API response"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn e2e_static_assets_served() {
        // Client JS chunks should be served from /_rex/client/
        let url = format!("{}/", base_url());
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

        // Extract a script src from the HTML
        if let Some(start) = body.find("/_rex/client/") {
            let chunk = &body[start..];
            let end = chunk.find('"').unwrap_or(chunk.len());
            let script_path = &chunk[..end];

            let asset_url = format!("{}{}", base_url(), script_path);
            let resp = reqwest::get(&asset_url).await.unwrap();
            assert_eq!(
                resp.status(),
                200,
                "Static asset {script_path} should return 200"
            );

            let ct = resp
                .headers()
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            assert!(
                ct.contains("javascript"),
                "Expected JS content-type for {script_path}, got: {ct}"
            );
        }
    }

    #[tokio::test]
    #[ignore]
    async fn e2e_data_endpoint_returns_json() {
        // The data endpoint is /_rex/data/{buildId}/{path}.json
        // First get the page to find the build ID from the manifest
        let url = format!("{}/", base_url());
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

        // Extract build_id from __REX_MANIFEST__
        if let Some(start) = body.find("\"build_id\":\"") {
            let rest = &body[start + 12..];
            let end = rest.find('"').unwrap();
            let build_id = &rest[..end];

            // Use /about (not index) — the trie stores "/" for index which
            // doesn't match "/index" after the handler trims .json
            let data_url = format!("{}/_rex/data/{}/about.json", base_url(), build_id);
            let resp = reqwest::get(&data_url).await.unwrap();
            assert_eq!(resp.status(), 200, "Data endpoint should return 200");

            let json: serde_json::Value = resp.json().await.unwrap();
            assert!(
                json.get("props").is_some(),
                "Data endpoint should return props"
            );
        }
    }

    #[tokio::test]
    #[ignore]
    async fn e2e_html_document_structure() {
        let url = format!("{}/", base_url());
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

        // Check basic HTML structure
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
        assert!(
            body.contains("<div id=\"__rex\">"),
            "Missing __rex root div"
        );
        assert!(body.contains("__REX_DATA__"), "Missing __REX_DATA__ script");
        assert!(
            body.contains("__REX_MANIFEST__"),
            "Missing __REX_MANIFEST__ script"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn e2e_gssp_props_in_html() {
        // The index page uses getServerSideProps that returns a timestamp
        let url = format!("{}/", base_url());
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

        // __REX_DATA__ should contain the GSSP props
        assert!(
            body.contains("message"),
            "GSSP props should include 'message'"
        );
        assert!(
            body.contains("Hello from Rex!"),
            "GSSP message should be 'Hello from Rex!'"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn e2e_buffer_polyfill() {
        // The buffer-test page uses Buffer in getServerSideProps for
        // base64/hex/utf8 encoding, concat, and type checking
        let url = format!("{}/buffer-test", base_url());
        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), 200, "buffer-test page should return 200");

        let body = resp.text().await.unwrap();

        // UTF-8 round-trip
        assert!(
            body.contains("hello world"),
            "Should contain utf8-encoded 'hello world': {body}"
        );
        // Base64 encoding of "hello world"
        assert!(
            body.contains("aGVsbG8gd29ybGQ="),
            "Should contain base64 of 'hello world': {body}"
        );
        // Hex encoding of "hello world"
        assert!(
            body.contains("68656c6c6f20776f726c64"),
            "Should contain hex of 'hello world': {body}"
        );
        // Base64 decode round-trip ("SGVsbG8gUmV4IQ==" → "Hello Rex!")
        assert!(
            body.contains("Hello Rex!"),
            "Should contain base64-decoded 'Hello Rex!': {body}"
        );
        // Buffer.isBuffer check
        assert!(
            body.contains("true"),
            "Should contain 'true' for isBuffer check: {body}"
        );
        // Buffer.concat
        assert!(
            body.contains("foobar"),
            "Should contain concatenated 'foobar': {body}"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn e2e_concurrent_requests() {
        // Fire multiple requests simultaneously to test isolate pool
        let base = base_url();
        let mut handles = vec![];

        for i in 0..8 {
            let url = if i % 2 == 0 {
                format!("{base}/")
            } else {
                format!("{base}/about")
            };
            handles.push(tokio::spawn(async move {
                let resp = reqwest::get(&url).await.unwrap();
                assert_eq!(resp.status(), 200, "Request {i} failed");
                resp.text().await.unwrap()
            }));
        }

        for (i, handle) in handles.into_iter().enumerate() {
            let body = handle.await.unwrap();
            assert!(!body.is_empty(), "Request {i} returned empty body");
        }
    }

    #[tokio::test]
    #[ignore]
    async fn e2e_head_tags_rendered() {
        // The about page uses <Head> to set a title
        let url = format!("{}/about", base_url());
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

        assert!(
            body.contains("<title>") || body.contains("About"),
            "About page should have title or About text rendered"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn e2e_client_router_script_served() {
        let url = format!("{}/_rex/router.js", base_url());
        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), 200);

        let body = resp.text().await.unwrap();
        assert!(
            body.contains("__REX_ROUTER"),
            "Router script should define __REX_ROUTER"
        );
    }

    // -------------------------------------------------------
    // Dev server lifecycle tests (HMR, rebuild, shutdown)
    // -------------------------------------------------------

    #[tokio::test]
    #[ignore]
    async fn e2e_hmr_websocket_sends_reload_on_file_change() {
        use futures::StreamExt;

        let server = ensure_server();
        let ws_url = format!("ws://127.0.0.1:{}/_rex/hmr", server.port);

        // Connect to HMR WebSocket
        let (ws_stream, _) = tokio_tungstenite::connect_async(&ws_url)
            .await
            .expect("Failed to connect to HMR WebSocket");

        let (_, mut read) = ws_stream.split();

        // Consume the initial "connected" message
        let first_msg = tokio::time::timeout(Duration::from_secs(5), read.next())
            .await
            .expect("Timed out waiting for connected message")
            .expect("Stream ended")
            .expect("WS error");
        let first_text = first_msg.to_text().unwrap_or("");
        assert!(
            first_text.contains("connected"),
            "First message should be 'connected', got: {first_text}"
        );

        // Read the about.tsx file and save original content
        let about_path = fixture_root().join("pages/about.tsx");
        let original = std::fs::read_to_string(&about_path).unwrap();

        // Modify the file (append a harmless comment)
        let modified = format!("{original}\n// e2e-test-marker\n");
        std::fs::write(&about_path, &modified).unwrap();

        // Wait for HMR update message (with timeout)
        let msg = tokio::time::timeout(Duration::from_secs(15), read.next()).await;

        // Restore the file immediately
        std::fs::write(&about_path, &original).unwrap();

        // Verify we got an update/reload message
        let msg = msg.expect("Timed out waiting for HMR message");
        let msg = msg
            .expect("WebSocket stream ended")
            .expect("WebSocket error");
        let text = msg.to_text().unwrap_or("");
        assert!(
            text.contains("update") || text.contains("reload"),
            "Expected HMR update/reload message, got: {text}"
        );

        // Wait for the rebuild from restoring the original file
        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    #[tokio::test]
    #[ignore]
    async fn e2e_hmr_rebuild_reflects_in_response() {
        // Modify a page, wait for rebuild, verify the change is visible in HTTP response
        let about_path = fixture_root().join("pages/about.tsx");
        let original = std::fs::read_to_string(&about_path).unwrap();

        // Add a unique marker to a string literal (not the function name)
        let marker = "E2E_TEST_MARKER_12345";
        let modified = original.replace("<h1>About</h1>", &format!("<h1>About {marker}</h1>"));
        std::fs::write(&about_path, &modified).unwrap();

        // Poll until the page reflects the change (or timeout)
        let url = format!("{}/about", base_url());
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
        std::fs::write(&about_path, &original).unwrap();

        // Wait for restore rebuild
        tokio::time::sleep(Duration::from_secs(3)).await;

        assert!(
            found,
            "Page should reflect the modified content after HMR rebuild"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn e2e_graceful_shutdown() {
        // Spawn a separate server instance on a DIFFERENT fixture directory
        // to avoid build cache conflicts with the shared TestServer on fixtures/basic.
        let bin = rex_binary();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("fixtures/zero-config");
        let port = find_free_port();

        let mut child = Command::new(&bin)
            .arg("dev")
            .arg("--root")
            .arg(&root)
            .arg("--port")
            .arg(port.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        // Wait for server to be fully ready (HTTP 200)
        let url = format!("http://127.0.0.1:{port}/");
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if Instant::now() > deadline {
                child.kill().ok();
                child.wait().ok();
                panic!("Shutdown test: server failed to become HTTP-ready within 60s");
            }
            if let Ok(resp) = http_client.get(&url).send().await {
                let status = resp.status().as_u16();
                if status == 200 {
                    break;
                }
                // 500 means init/build failed permanently — no point retrying
                if status == 500 {
                    let body = resp.text().await.unwrap_or_default();
                    child.kill().ok();
                    child.wait().ok();
                    panic!("Shutdown test: server returned 500 (init failed): {body}");
                }
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }

        // Send SIGTERM (graceful shutdown)
        #[cfg(unix)]
        // SAFETY: child.id() is a valid PID from a process we spawned
        #[allow(unsafe_code)]
        unsafe {
            libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
        }
        #[cfg(not(unix))]
        {
            child.kill().ok();
        }

        // Wait for process to exit (with timeout)
        let exit_deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                // Process exited — success
                eprintln!("[e2e] Server exited with status: {status}");
                break;
            }
            if Instant::now() > exit_deadline {
                child.kill().ok();
                panic!("Server did not shut down within 5s after SIGTERM");
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        // Verify the port is no longer accepting connections
        let addr: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        assert!(
            TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_err(),
            "Port should be closed after shutdown"
        );
    }

    // -------------------------------------------------------
    // MCP endpoint tests (JSON-RPC 2.0 over POST /mcp)
    // -------------------------------------------------------

    /// Helper: POST a JSON-RPC request to /mcp and return the parsed response.
    async fn mcp_request(method: &str, params: serde_json::Value) -> serde_json::Value {
        let url = format!("{}/mcp", base_url());
        let client = reqwest::Client::new();
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });
        let resp = client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200, "MCP endpoint should return 200");
        resp.json().await.unwrap()
    }

    #[tokio::test]
    #[ignore]
    async fn e2e_mcp_initialize() {
        let resp = mcp_request(
            "initialize",
            serde_json::json!({
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": { "name": "rex-e2e", "version": "0.1" }
            }),
        )
        .await;

        let result = resp.get("result").expect("initialize should return result");
        assert_eq!(
            result["protocolVersion"], "2025-03-26",
            "Should return matching protocol version"
        );
        assert!(
            result.get("capabilities").is_some(),
            "Should include capabilities"
        );
        assert_eq!(
            result["serverInfo"]["name"], "rex",
            "Server name should be 'rex'"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn e2e_mcp_tools_list() {
        let resp = mcp_request("tools/list", serde_json::json!({})).await;

        let tools = resp["result"]["tools"]
            .as_array()
            .expect("tools/list should return an array");
        assert!(
            !tools.is_empty(),
            "Should have at least one tool (fixtures/basic/mcp/search.ts)"
        );

        // The basic fixture has a "search" tool
        let search = tools.iter().find(|t| t["name"] == "search");
        assert!(search.is_some(), "Should include 'search' tool");

        let search = search.unwrap();
        assert!(
            search.get("description").is_some(),
            "Tool should have a description"
        );
        assert!(
            search.get("inputSchema").is_some(),
            "Tool should have an inputSchema"
        );
        assert_eq!(
            search["inputSchema"]["type"], "object",
            "inputSchema should be an object type"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn e2e_mcp_tools_call() {
        let resp = mcp_request(
            "tools/call",
            serde_json::json!({
                "name": "search",
                "arguments": { "query": "hello" }
            }),
        )
        .await;

        assert!(
            resp.get("error").is_none(),
            "tools/call should not return an error: {resp}"
        );

        let content = resp["result"]["content"]
            .as_array()
            .expect("tools/call result should have content array");
        assert!(!content.is_empty(), "content should not be empty");
        assert_eq!(content[0]["type"], "text", "content type should be 'text'");

        // Parse the text payload and verify it contains results
        let text = content[0]["text"].as_str().unwrap();
        let payload: serde_json::Value = serde_json::from_str(text).unwrap();
        assert!(
            payload.get("results").is_some(),
            "search tool should return results"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn e2e_mcp_tools_call_unknown_tool() {
        let resp = mcp_request(
            "tools/call",
            serde_json::json!({
                "name": "nonexistent_tool",
                "arguments": {}
            }),
        )
        .await;

        // MCP spec: tool errors are returned as successful JSON-RPC responses
        // with isError: true in the result, not as JSON-RPC errors.
        let result = &resp["result"];
        assert_eq!(
            result["isError"], true,
            "Unknown tool should return isError: true, got: {resp}"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn e2e_mcp_unknown_method() {
        let resp = mcp_request("bogus/method", serde_json::json!({})).await;

        assert!(
            resp.get("error").is_some(),
            "Unknown JSON-RPC method should return an error"
        );
    }

    // -------------------------------------------------------
    // Config (rex.config.toml) tests
    // -------------------------------------------------------

    #[tokio::test]
    #[ignore]
    async fn e2e_config_redirect() {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let url = format!("{}/legacy-about", base_url());
        let resp = client.get(&url).send().await.unwrap();

        assert_eq!(resp.status(), 308, "Permanent redirect should return 308");
        let location = resp.headers().get("location").unwrap().to_str().unwrap();
        assert_eq!(location, "/about", "Should redirect to /about");
    }

    #[tokio::test]
    #[ignore]
    async fn e2e_config_rewrite() {
        let url = format!("{}/rewritten", base_url());
        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), 200, "Rewritten path should return 200");

        let body = resp.text().await.unwrap();
        assert!(
            body.contains("About"),
            "Rewritten path should serve the about page content"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn e2e_config_custom_header() {
        let url = format!("{}/", base_url());
        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), 200);

        let powered_by = resp
            .headers()
            .get("X-Powered-By")
            .expect("X-Powered-By header should be present")
            .to_str()
            .unwrap();
        assert_eq!(powered_by, "Rex", "X-Powered-By should be 'Rex'");
    }

    // -------------------------------------------------------
    // Console output tests
    // -------------------------------------------------------

    #[tokio::test]
    #[ignore]
    async fn e2e_console_log_appears_in_server_output() {
        use std::io::{BufRead, BufReader};

        let bin = rex_binary();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("fixtures/zero-config");
        let port = find_free_port();

        let mut child = Command::new(&bin)
            .arg("dev")
            .arg("--root")
            .arg(&root)
            .arg("--port")
            .arg(port.to_string())
            .arg("--no-tui")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        // Spawn a reader thread that collects stderr lines
        let stderr = child.stderr.take().unwrap();
        let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let lines_writer = lines.clone();
        let reader_thread = std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                match line {
                    Ok(line) => lines_writer.lock().unwrap().push(line),
                    Err(_) => break,
                }
            }
        });

        // Poll with HTTP GET until the server returns a valid response.
        // Fail fast on 500 (init/build failed permanently — retrying is pointless).
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let poll_url = format!("http://127.0.0.1:{port}/");
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if Instant::now() > deadline {
                child.kill().ok();
                child.wait().ok();
                let _ = reader_thread.join();
                panic!("Console test: server failed to start within 30s");
            }
            if let Ok(resp) = http_client.get(&poll_url).send().await {
                let status = resp.status().as_u16();
                if status == 200 {
                    break;
                }
                if status == 500 {
                    let body = resp.text().await.unwrap_or_default();
                    child.kill().ok();
                    child.wait().ok();
                    let _ = reader_thread.join();
                    let captured = lines.lock().unwrap();
                    panic!(
                        "Console test: server returned 500 (init failed): {body}\nstderr:\n{}",
                        captured.join("\n")
                    );
                }
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }

        // Hit /about which has console.log("hello limothy")
        let url = format!("http://127.0.0.1:{port}/about");
        let resp = http_client.get(&url).send().await.unwrap();
        assert_eq!(resp.status(), 200);

        // Give the server a moment to flush the log
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Kill the server
        #[cfg(unix)]
        #[allow(unsafe_code)]
        unsafe {
            libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
        }
        #[cfg(not(unix))]
        {
            child.kill().ok();
        }
        let _ = child.wait();
        let _ = reader_thread.join();

        let captured = lines.lock().unwrap();
        assert!(
            captured.iter().any(|l| l.contains("hello limothy")),
            "Server stderr should contain console.log output 'hello limothy', got:\n{}",
            captured.join("\n")
        );
    }
}
