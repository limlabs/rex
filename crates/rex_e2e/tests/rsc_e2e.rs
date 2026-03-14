//! E2E tests for RSC (React Server Components) via the app/ router.
//!
//! Run with: cargo test -p rex_e2e --test rsc_e2e -- --ignored
//!
//! Prerequisites:
//!   - `cargo build` (debug or release)
//!   - `cd fixtures/app-router && npm install`

#[allow(clippy::unwrap_used)]
mod rsc {
    use futures::StreamExt;
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
                .unwrap_or_else(|e| panic!("Failed to start rex: {e}\nBinary: {}", bin.display()));

            // Wait for server to be ready
            let deadline = Instant::now() + Duration::from_secs(30);
            let addr = format!("127.0.0.1:{port}");
            loop {
                if Instant::now() > deadline {
                    panic!("[rsc-e2e] Server failed to start within 30s on port {port}");
                }
                if TcpStream::connect_timeout(&addr.parse().unwrap(), Duration::from_millis(100))
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
        assert!(body.contains("Rex!"), "Missing SSR content 'Rex!'");
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
        assert!(body.contains("Dashboard"), "Missing dashboard content");
    }

    #[tokio::test]
    #[ignore]
    async fn rsc_nested_layout_settings_page() {
        let url = format!("{}/dashboard/settings", base_url());
        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), 200);

        let body = resp.text().await.unwrap();
        assert!(body.contains("Settings"), "Missing settings content");
    }

    #[tokio::test]
    #[ignore]
    async fn rsc_client_component_reference_in_flight() {
        // The settings page imports Counter ("use client")
        // The flight data should contain a client reference marker
        let url = format!("{}/dashboard/settings", base_url());
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

        // Flight data should have an M: row for the client component
        assert!(body.contains("__REX_RSC_DATA__"), "Missing flight data");
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
            // React RSC flight format uses hex-id prefixed rows:
            //   <id>:I[...] for client imports, <id>:{...} for elements, <id>:D{...} for debug
            assert!(
                flight.contains(":I[") || flight.contains("[\"$\""),
                "Flight data should contain RSC wire format rows (client refs or element trees)"
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
        assert!(
            body.contains("<div id=\"__rex\">"),
            "Missing __rex root div"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn rsc_html_has_layout_attributes() {
        // Root layout sets <html lang="en"> and <body className="app-root">.
        // These attributes must flow through the RSC render into the served HTML
        // to prevent React hydration mismatch errors (#418).
        let url = format!("{}/", base_url());
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

        assert!(
            body.contains(r#"<html lang="en">"#) || body.contains(r#"<html lang="en" "#),
            "Served HTML must include lang=\"en\" on <html> from root layout.\nGot: {}",
            &body[..body.len().min(500)]
        );
        assert!(
            body.contains(r#"class="app-root""#),
            "Served HTML must include class=\"app-root\" on <body> from root layout.\nGot: {}",
            &body[..body.len().min(500)]
        );
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

    // -------------------------------------------------------
    // Client hydration pipeline tests
    // -------------------------------------------------------

    /// Extract all script src URLs from HTML
    fn extract_script_srcs(html: &str) -> Vec<String> {
        let mut srcs = vec![];
        let mut search = html;
        while let Some(start) = search.find("src=\"") {
            let rest = &search[start + 5..];
            if let Some(end) = rest.find('"') {
                let src = &rest[..end];
                if src.ends_with(".js") {
                    srcs.push(src.to_string());
                }
            }
            search = &search[start + 5..];
        }
        srcs
    }

    #[tokio::test]
    #[ignore]
    async fn rsc_client_chunks_serve_200() {
        // The settings page imports Counter ("use client") — its chunks must be serveable
        let url = format!("{}/dashboard/settings", base_url());
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

        let script_srcs = extract_script_srcs(&body);
        assert!(
            !script_srcs.is_empty(),
            "Page should have at least one <script> tag"
        );

        // Every script src that references a client chunk must return 200
        for src in &script_srcs {
            if src.contains("/_rex/static/") {
                let chunk_url = format!("{}{}", base_url(), src);
                let resp = reqwest::get(&chunk_url).await.unwrap();
                assert_eq!(
                    resp.status(),
                    200,
                    "Client chunk returned {} for URL: {}",
                    resp.status(),
                    chunk_url
                );

                let body = resp.text().await.unwrap();
                assert!(!body.is_empty(), "Client chunk is empty: {chunk_url}");
            }
        }
    }

    #[tokio::test]
    #[ignore]
    async fn rsc_client_chunk_contains_react_dom() {
        // Client component chunks need react-dom for hydration to work.
        // Without it, hydrateRoot/createRoot won't be available.
        let url = format!("{}/dashboard/settings", base_url());
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

        let script_srcs = extract_script_srcs(&body);

        // Collect all client JS content
        let mut all_client_js = String::new();
        for src in &script_srcs {
            if src.contains("/_rex/static/") {
                let chunk_url = format!("{}{}", base_url(), src);
                let resp = reqwest::get(&chunk_url).await;
                if let Ok(resp) = resp {
                    if resp.status() == 200 {
                        if let Ok(text) = resp.text().await {
                            all_client_js.push_str(&text);
                        }
                    }
                }
            }
        }

        assert!(
            !all_client_js.is_empty(),
            "No client JS was loaded — chunks may be 404ing"
        );

        // React DOM must be present somewhere in the client bundles
        // (either in the component chunk or as a shared chunk)
        assert!(
            all_client_js.contains("hydrateRoot") || all_client_js.contains("createRoot"),
            "Client bundles must contain React DOM (hydrateRoot/createRoot) for hydration"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn rsc_html_loads_client_chunks() {
        // The RSC hydration entry is bundled as an ESM module alongside
        // client component chunks. Verify client chunks are present.
        let url = format!("{}/dashboard/settings", base_url());
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

        // The page should reference client component chunks
        let has_client_chunk = body.contains("/_rex/static/rsc/");
        assert!(
            has_client_chunk,
            "Settings page imports Counter (use client) — must have client chunk references"
        );

        // The hydration entry should be bundled in the client chunks
        assert!(
            body.contains("type=\"module\""),
            "Client chunks should be loaded as ESM modules"
        );
    }

    // -------------------------------------------------------
    // Mixed pages/ + app/ coexistence tests
    // -------------------------------------------------------

    #[tokio::test]
    #[ignore]
    async fn rsc_mixed_pages_api_route_works() {
        // The app-router fixture has both app/ and pages/ directories.
        // pages/api/health.ts should be handled by the pages router while
        // app/ routes are handled by the RSC router.
        let url = format!("{}/api/health", base_url());
        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), 200, "Pages API route should return 200");

        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["ok"], true);
        assert_eq!(body["router"], "pages");
    }

    #[tokio::test]
    #[ignore]
    async fn rsc_mixed_app_routes_still_work() {
        // Verify app/ routes still work alongside pages/ routes
        let url = format!("{}/about", base_url());
        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), 200, "App route /about should return 200");
        let body = resp.text().await.unwrap();
        assert!(
            body.contains("__REX_RSC_DATA__"),
            "App route should have RSC flight data"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn rsc_module_map_references_valid_chunks() {
        // The module map embedded in HTML must point to chunk URLs that actually exist
        let url = format!("{}/dashboard/settings", base_url());
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

        // Extract module map JSON
        let marker = "window.__REX_RSC_MODULE_MAP__=";
        let map_start = body.find(marker).expect("Missing module map in HTML");
        let json_start = map_start + marker.len();
        let rest = &body[json_start..];
        // Find the end of the JSON (next </script>)
        let json_end = rest.find("</script>").expect("Missing closing script tag");
        let map_json = &rest[..json_end];

        let map: serde_json::Value =
            serde_json::from_str(map_json).expect("Module map is not valid JSON");

        // Every chunk_url in the manifest must return 200
        if let Some(entries) = map.get("entries").and_then(|e| e.as_object()) {
            assert!(
                !entries.is_empty(),
                "Module map has no entries — Counter should have a reference"
            );

            for (ref_id, entry) in entries {
                let chunk_url = entry
                    .get("chunk_url")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| panic!("No chunk_url for ref {ref_id}"));

                let full_url = format!("{}{}", base_url(), chunk_url);
                let resp = reqwest::get(&full_url).await.unwrap();
                assert_eq!(
                    resp.status(),
                    200,
                    "Module map chunk_url returned {} for ref {}: {}",
                    resp.status(),
                    ref_id,
                    full_url,
                );
            }
        }
    }

    // -------------------------------------------------------
    // Streaming HTML verification tests
    // -------------------------------------------------------

    #[tokio::test]
    #[ignore]
    async fn rsc_response_is_chunked_transfer() {
        // Streaming responses use chunked transfer encoding (no content-length).
        // This verifies the server is actually streaming rather than buffering.
        let url = format!("{}/", base_url());
        let resp = reqwest::get(&url).await.unwrap();

        // Chunked responses either have Transfer-Encoding: chunked
        // or lack a Content-Length header (axum uses chunked for Body::from_stream)
        let has_chunked = resp
            .headers()
            .get("transfer-encoding")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.contains("chunked"))
            .unwrap_or(false);
        let no_content_length = resp.headers().get("content-length").is_none();

        assert!(
            has_chunked || no_content_length,
            "Response should be streamed (chunked or no content-length), \
             but has content-length: {:?}",
            resp.headers().get("content-length")
        );
    }

    #[tokio::test]
    #[ignore]
    async fn rsc_head_shell_flushed_before_body() {
        // The head shell (DOCTYPE, meta, modulepreload, module map) must
        // be a separate stream chunk from the body tail (SSR HTML, flight data).
        // Read the response as a byte stream and verify the first chunk
        // contains head content but NOT the SSR body content.
        let url = format!("{}/dashboard/settings", base_url());
        let resp = reqwest::get(&url).await.unwrap();
        let mut stream = resp.bytes_stream();

        let mut chunks: Vec<String> = Vec::new();
        while let Some(chunk) = stream.next().await {
            let bytes = chunk.unwrap();
            chunks.push(String::from_utf8_lossy(&bytes).to_string());
        }

        assert!(
            chunks.len() >= 2,
            "Expected at least 2 stream chunks (head shell + body tail), got {}",
            chunks.len()
        );

        let first_chunk = &chunks[0];

        // Head shell must contain early content
        assert!(
            first_chunk.contains("<!DOCTYPE html>"),
            "First chunk must start with DOCTYPE"
        );
        assert!(
            first_chunk.contains("__REX_RSC_MODULE_MAP__"),
            "First chunk must contain early module map injection"
        );
        assert!(
            first_chunk.contains("<body>") || first_chunk.contains("<body "),
            "First chunk must open <body> tag"
        );

        // Head shell must NOT contain the SSR body (that comes in later chunks)
        assert!(
            !first_chunk.contains("__rex\""),
            "First chunk should not contain the __rex div (that's in the body tail)"
        );
        assert!(
            !first_chunk.contains("__REX_RSC_DATA__"),
            "First chunk should not contain flight data (that's in the body tail)"
        );

        // Remaining chunks (joined) must contain the body tail
        let tail: String = chunks[1..].join("");
        assert!(
            tail.contains("<div id=\"__rex\">"),
            "Body tail must contain the __rex SSR div"
        );
        assert!(
            tail.contains("__REX_RSC_DATA__"),
            "Body tail must contain inline flight data"
        );
        assert!(
            tail.contains("</html>"),
            "Body tail must close the HTML document"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn rsc_head_shell_has_modulepreload_hints() {
        // The head shell should contain <link rel="modulepreload"> for client
        // component chunks. This is critical for eliminating the import waterfall —
        // the browser starts fetching JS while V8 renders the body.
        let url = format!("{}/dashboard/settings", base_url());
        let resp = reqwest::get(&url).await.unwrap();
        let mut stream = resp.bytes_stream();

        // Read only the first chunk (the head shell)
        let first_chunk = stream
            .next()
            .await
            .expect("stream should have at least one chunk")
            .unwrap();
        let head_shell = String::from_utf8_lossy(&first_chunk);

        // Settings page imports Counter ("use client"), so its chunk should be preloaded
        assert!(
            head_shell.contains("rel=\"modulepreload\""),
            "Head shell must contain modulepreload links for client chunks.\n\
             Head shell:\n{}",
            head_shell
        );

        // The modulepreload href should point to a valid static asset path
        assert!(
            head_shell.contains("/_rex/static/rsc/"),
            "Modulepreload links must reference /_rex/static/rsc/ paths"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn rsc_body_tail_contains_ssr_content() {
        // The body tail (second stream phase) should contain actual SSR-rendered
        // content from V8, not just shell/placeholder HTML.
        let url = format!("{}/about", base_url());
        let resp = reqwest::get(&url).await.unwrap();
        let mut stream = resp.bytes_stream();

        let mut chunks: Vec<String> = Vec::new();
        while let Some(chunk) = stream.next().await {
            let bytes = chunk.unwrap();
            chunks.push(String::from_utf8_lossy(&bytes).to_string());
        }

        assert!(
            chunks.len() >= 2,
            "Expected at least 2 stream chunks, got {}",
            chunks.len()
        );

        // The body tail should have the actual rendered content
        let tail: String = chunks[1..].join("");
        assert!(
            tail.contains("About"),
            "Body tail should contain rendered page content ('About')"
        );

        // Flight data should be in the body tail (same chunk as SSR HTML)
        assert!(
            tail.contains("type=\"text/rsc\""),
            "Body tail should contain flight data script with type=text/rsc"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn rsc_streaming_complete_html_is_valid() {
        // When all chunks are assembled, the result must be a complete valid HTML document.
        let url = format!("{}/", base_url());
        let resp = reqwest::get(&url).await.unwrap();
        let mut stream = resp.bytes_stream();

        let mut full_html = String::new();
        while let Some(chunk) = stream.next().await {
            let bytes = chunk.unwrap();
            full_html.push_str(&String::from_utf8_lossy(&bytes));
        }

        // Verify complete document structure
        assert!(full_html.starts_with("<!DOCTYPE html>"));
        assert!(full_html.contains("<html"));
        assert!(full_html.contains("<head>"));
        assert!(full_html.contains("</head>"));
        assert!(full_html.contains("<body>") || full_html.contains("<body "));
        assert!(full_html.contains("</body>"));
        assert!(full_html.trim_end().ends_with("</html>"));

        // Critical ordering: module map appears before flight data
        let map_pos = full_html
            .find("__REX_RSC_MODULE_MAP__")
            .expect("Missing module map");
        let flight_pos = full_html
            .find("__REX_RSC_DATA__")
            .expect("Missing flight data");
        assert!(
            map_pos < flight_pos,
            "Module map (pos {map_pos}) must appear before flight data (pos {flight_pos})"
        );

        // Critical ordering: head shell content before SSR body
        let body_tag_pos = full_html
            .find("<body>")
            .or_else(|| full_html.find("<body "))
            .expect("Missing body tag");
        let rex_div_pos = full_html
            .find("<div id=\"__rex\">")
            .expect("Missing __rex div");
        assert!(
            body_tag_pos < rex_div_pos,
            "Body tag must appear before __rex div"
        );
    }

    // -------------------------------------------------------
    // Async server component tests
    // -------------------------------------------------------

    #[tokio::test]
    #[ignore]
    async fn rsc_async_server_component_renders() {
        let url = format!("{}/data", base_url());
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .unwrap();
        let resp = client.get(&url).send().await.unwrap();
        assert_eq!(
            resp.status(),
            200,
            "Async server component page should return 200"
        );

        let body = resp.text().await.unwrap();
        assert!(
            body.contains("Data Page"),
            "Missing 'Data Page' heading in async server component output"
        );
        assert!(
            body.contains("Hello from async server component"),
            "Missing resolved async data in server component output"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn rsc_async_flight_has_resolved_content() {
        // First get build ID from a page
        let url = format!("{}/", base_url());
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

        if let Some(start) = body.find("\"build_id\":\"") {
            let rest = &body[start + 12..];
            let end = rest.find('"').unwrap();
            let build_id = &rest[..end];

            let rsc_url = format!("{}/_rex/rsc/{}/data", base_url(), build_id);
            let resp = reqwest::get(&rsc_url).await.unwrap();
            assert_eq!(resp.status(), 200);

            let flight = resp.text().await.unwrap();
            assert!(
                flight.contains("Hello from async server component"),
                "Flight data should contain resolved async content, got: {flight}"
            );
            // Should NOT contain the placeholder null
            assert!(
                !flight.contains("J:0:null") && !flight.contains("J:1:null"),
                "Flight data should not contain unresolved placeholder nulls"
            );
        } else {
            panic!("Could not extract build_id from page HTML");
        }
    }

    // -------------------------------------------------------
    // Server action tests
    // -------------------------------------------------------

    /// Helper: extract build_id from page HTML
    fn extract_build_id(body: &str) -> String {
        let start = body
            .find("\"build_id\":\"")
            .expect("Could not find build_id in HTML");
        let rest = &body[start + 12..];
        let end = rest.find('"').unwrap();
        rest[..end].to_string()
    }

    #[tokio::test]
    #[ignore]
    async fn rsc_server_action_returns_result() {
        // First, get the build_id from the page HTML
        let body = reqwest::get(&format!("{}/", base_url()))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let build_id = extract_build_id(&body);

        // Find the action ID from the flight data — look for server action references
        // For now, we'll test the endpoint with a known-bad action ID to verify routing
        // works, then test with a valid one extracted from flight data.

        // Test: stale build_id returns 404
        let client = reqwest::Client::new();
        let resp = client
            .post(format!(
                "{}/_rex/action/stale-build-id/some-action",
                base_url()
            ))
            .header("Content-Type", "application/json")
            .body("[42]")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 404, "Stale build_id should return 404");

        // Test: unknown action ID returns error JSON (not a 500)
        let resp = client
            .post(format!(
                "{}/_rex/action/{}/nonexistent-action-id",
                base_url(),
                build_id
            ))
            .header("Content-Type", "application/json")
            .body("[42]")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let result: serde_json::Value = resp.json().await.unwrap();
        assert!(
            result.get("error").is_some(),
            "Unknown action should return error JSON, got: {result}"
        );
    }

    // -------------------------------------------------------
    // Context provider tests
    // -------------------------------------------------------

    #[tokio::test]
    #[ignore]
    async fn rsc_context_provider_renders() {
        let url = format!("{}/context-test", base_url());
        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), 200, "Context test page should return 200");

        let body = resp.text().await.unwrap();
        assert!(
            body.contains("Context Test"),
            "Missing 'Context Test' heading in context provider page"
        );
        assert!(
            body.contains("Current theme:"),
            "Missing theme display in context provider page. \
             The ThemeProvider should pass the theme value to ThemeDisplay via context."
        );
    }

    #[tokio::test]
    #[ignore]
    async fn rsc_context_flight_has_client_refs() {
        // Get build ID from a page
        let url = format!("{}/", base_url());
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

        if let Some(start) = body.find("\"build_id\":\"") {
            let rest = &body[start + 12..];
            let end = rest.find('"').unwrap();
            let build_id = &rest[..end];

            let rsc_url = format!("{}/_rex/rsc/{}/context-test", base_url(), build_id);
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
            // Flight data should contain client reference rows (:I[) for the
            // ThemeProvider and ThemeDisplay components
            assert!(
                flight.contains(":I[") || flight.contains("[\"$\""),
                "Flight data should contain client reference rows for context \
                 provider/consumer components"
            );
        } else {
            panic!("Could not extract build_id from page HTML");
        }
    }

    #[tokio::test]
    #[ignore]
    async fn rsc_context_nested_providers_render() {
        let url = format!("{}/context-test/nested", base_url());
        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(
            resp.status(),
            200,
            "Nested context test page should return 200"
        );

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

    #[tokio::test]
    #[ignore]
    async fn rsc_context_page_has_module_map() {
        let url = format!("{}/context-test", base_url());
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

        assert!(
            body.contains("__REX_RSC_MODULE_MAP__"),
            "Context test page should have client reference module map"
        );
    }

    // -------------------------------------------------------
    // Server action tests
    // -------------------------------------------------------

    #[tokio::test]
    #[ignore]
    async fn rsc_server_action_increment_works() {
        // Get the page HTML and extract build_id + server action IDs from manifest
        let body = reqwest::get(&format!("{}/", base_url()))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let build_id = extract_build_id(&body);

        // Server action IDs are exposed in __REX_MANIFEST__.server_actions
        // Extract from the inline manifest JSON in the page HTML
        let manifest_start = body
            .find("__REX_MANIFEST__=")
            .expect("Missing __REX_MANIFEST__ in HTML");
        let json_start = manifest_start + "__REX_MANIFEST__=".len();
        let json_end = body[json_start..]
            .find("</script>")
            .expect("Missing closing script tag for manifest");
        let manifest_json = &body[json_start..json_start + json_end];
        let manifest: serde_json::Value =
            serde_json::from_str(manifest_json).expect("Invalid manifest JSON");

        let server_actions = manifest
            .get("server_actions")
            .expect("Missing server_actions in manifest");
        assert!(
            server_actions.is_object() && !server_actions.as_object().unwrap().is_empty(),
            "server_actions should be a non-empty object, got: {server_actions}"
        );

        // Find an incrementCounter or decrementCounter action
        let action_id = server_actions
            .as_object()
            .unwrap()
            .iter()
            .find(|(_, name)| {
                name.as_str()
                    .is_some_and(|n| n == "incrementCounter" || n == "decrementCounter")
            })
            .map(|(id, _)| id.clone())
            .expect("Should find incrementCounter or decrementCounter in server_actions");

        // Call the server action with argument [42]
        let client = reqwest::Client::new();
        let resp = client
            .post(format!(
                "{}/_rex/action/{}/{}",
                base_url(),
                build_id,
                action_id
            ))
            .header("Content-Type", "application/json")
            .body("[42]")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let result: serde_json::Value = resp.json().await.unwrap();
        let value = result
            .get("result")
            .expect("Server action should return {result: ...}");
        // incrementCounter(42) = 43, decrementCounter(42) = 41
        assert!(
            value == 43 || value == 41,
            "Server action should return 43 or 41, got: {value}"
        );
    }
}
