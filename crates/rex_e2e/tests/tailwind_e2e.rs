//! E2E tests for Tailwind CSS integration.
//!
//! Tests the built-in V8-based Tailwind compiler against a fixture that has
//! NO tailwindcss npm dependency — Rex should compile Tailwind CSS using the
//! embedded compiler without requiring `npm install`.
//!
//! Run with: cargo test -p rex_e2e --test tailwind_e2e -- --ignored
//!
//! Prerequisites:
//!   - `cargo build` (debug or release)

#[allow(clippy::unwrap_used)]
mod tailwind {
    use std::net::TcpStream;
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    struct TestServer {
        port: u16,
        _child: Child,
    }

    static TAILWIND_SERVER: OnceLock<TestServer> = OnceLock::new();

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
            .join("fixtures/tailwind-builtin")
    }

    fn find_free_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    }

    fn ensure_server() -> &'static TestServer {
        TAILWIND_SERVER.get_or_init(|| {
            let bin = rex_binary();
            let root = fixture_root();
            let port = find_free_port();

            eprintln!("[tailwind-e2e] Starting rex dev server on port {port}");
            eprintln!("[tailwind-e2e] Binary: {}", bin.display());
            eprintln!("[tailwind-e2e] Root: {}", root.display());

            // Verify no package.json with tailwindcss — this fixture uses the built-in compiler
            let pkg_json = root.join("package.json");
            if pkg_json.exists() {
                let content = std::fs::read_to_string(&pkg_json).unwrap();
                assert!(
                    !content.contains("tailwindcss"),
                    "tailwind-builtin fixture should NOT have tailwindcss in package.json"
                );
            }

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

            // Wait for server to be ready (TCP connect)
            let deadline = Instant::now() + Duration::from_secs(30);
            let addr = format!("127.0.0.1:{port}");
            loop {
                if Instant::now() > deadline {
                    panic!("[tailwind-e2e] Server failed to start within 30s on port {port}");
                }
                if TcpStream::connect_timeout(&addr.parse().unwrap(), Duration::from_millis(100))
                    .is_ok()
                {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }

            eprintln!("[tailwind-e2e] Server ready on port {port}");

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
    // Page rendering tests
    // -------------------------------------------------------

    #[tokio::test]
    #[ignore]
    async fn e2e_tailwind_index_returns_200() {
        let url = format!("{}/", base_url());
        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), 200);

        let body = resp.text().await.unwrap();
        assert!(
            body.contains("Tailwind Builtin"),
            "Index page should contain 'Tailwind Builtin'"
        );
        assert!(
            body.contains("Hello from Rex with built-in Tailwind!"),
            "Index page should contain GSSP message"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn e2e_tailwind_html_structure() {
        let url = format!("{}/", base_url());
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

        assert!(
            body.contains("<!DOCTYPE html>") || body.contains("<!doctype html>"),
            "Missing DOCTYPE"
        );
        assert!(
            body.contains("<div id=\"__rex\">"),
            "Missing __rex root div"
        );
        assert!(body.contains("__REX_DATA__"), "Missing __REX_DATA__ script");
    }

    // -------------------------------------------------------
    // CSS compilation tests — the core of the Tailwind E2E
    // -------------------------------------------------------

    #[tokio::test]
    #[ignore]
    async fn e2e_tailwind_css_present_in_html() {
        let url = format!("{}/", base_url());
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

        // Tailwind CSS should be present in the HTML — either as an inline <style>
        // tag or as a <link> to an external CSS file
        let has_inline_css = body.contains("<style>") && body.contains("tailwindcss");
        let has_css_link = extract_css_href(&body).is_some();

        assert!(
            has_inline_css || has_css_link,
            "HTML should contain Tailwind CSS (inline <style> or <link>)"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn e2e_tailwind_css_contains_utilities() {
        let css = get_tailwind_css().await;

        // Verify the CSS contains Tailwind utility rules.
        // The fixture uses: font-bold, shadow, grid, text-4xl, bg-white, etc.
        assert!(
            css.contains(".font-bold"),
            "CSS should contain .font-bold utility"
        );
        assert!(
            css.contains(".shadow"),
            "CSS should contain .shadow utility"
        );
        assert!(css.contains(".grid"), "CSS should contain .grid utility");
        assert!(
            css.contains(".bg-white"),
            "CSS should contain .bg-white utility"
        );
        assert!(
            css.contains(".text-4xl"),
            "CSS should contain .text-4xl utility"
        );
        assert!(
            css.contains(".mx-auto"),
            "CSS should contain .mx-auto utility"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn e2e_tailwind_css_contains_preflight() {
        let css = get_tailwind_css().await;

        // Preflight includes a universal box-sizing reset
        assert!(
            css.contains("box-sizing"),
            "CSS should contain preflight box-sizing reset"
        );
        assert!(
            css.contains("border-box"),
            "CSS should contain border-box declaration"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn e2e_tailwind_css_contains_theme() {
        let css = get_tailwind_css().await;

        // Tailwind v4 theme layer includes CSS custom properties
        assert!(
            css.contains("--font-sans") || css.contains("--color-gray"),
            "CSS should contain Tailwind v4 theme custom properties"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn e2e_tailwind_css_is_substantial() {
        let css = get_tailwind_css().await;

        // Tailwind preflight + theme + utilities should produce substantial CSS
        assert!(
            css.len() > 1000,
            "Compiled CSS should be substantial (got {} bytes), \
             suggesting Tailwind compilation ran successfully",
            css.len()
        );
    }

    #[tokio::test]
    #[ignore]
    async fn e2e_tailwind_ssr_renders_class_attributes() {
        // Verify the SSR HTML contains the Tailwind class names in the rendered markup
        let url = format!("{}/", base_url());
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

        // These class names are used in the fixture's JSX
        for class in [
            "max-w-2xl",
            "text-4xl",
            "font-bold",
            "bg-white",
            "grid-cols-2",
            "min-h-screen",
            "bg-gray-50",
        ] {
            assert!(
                body.contains(class),
                "SSR HTML should contain '{class}' class"
            );
        }
    }

    // -------------------------------------------------------
    // Helper functions
    // -------------------------------------------------------

    /// Fetch the page HTML and extract Tailwind CSS content.
    ///
    /// CSS may be inlined via `<style>` or linked via `<link>`. This function
    /// handles both cases and returns the raw CSS text.
    async fn get_tailwind_css() -> String {
        let url = format!("{}/", base_url());
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

        // Try external CSS file first
        if let Some(css_href) = extract_css_href(&body) {
            let css_url = if css_href.starts_with("http") {
                css_href
            } else {
                format!("{}{}", base_url(), css_href)
            };
            let css_resp = reqwest::get(&css_url).await.unwrap();
            assert_eq!(css_resp.status(), 200, "CSS file should return 200");
            return css_resp.text().await.unwrap();
        }

        // Fall back to inline <style> tag
        if let Some(css) = extract_inline_css(&body) {
            return css;
        }

        panic!("No Tailwind CSS found in HTML (neither <link> nor <style>):\n{body}");
    }

    /// Extract CSS href from a `<link rel="stylesheet" href="...">` tag.
    fn extract_css_href(html: &str) -> Option<String> {
        let mut search_from = 0;
        while let Some(link_start) = html[search_from..].find("<link") {
            let abs_start = search_from + link_start;
            let link_end = html[abs_start..].find('>')?;
            let link_tag = &html[abs_start..abs_start + link_end + 1];

            if link_tag.contains("stylesheet") || link_tag.contains(".css") {
                if let Some(href_start) = link_tag.find("href=\"") {
                    let href_val = &link_tag[href_start + 6..];
                    let href_end = href_val.find('"')?;
                    let href = &href_val[..href_end];
                    if href.contains(".css") {
                        return Some(href.to_string());
                    }
                }
            }

            search_from = abs_start + link_end + 1;
        }
        None
    }

    /// Extract CSS content from the first `<style>` tag that contains Tailwind CSS.
    fn extract_inline_css(html: &str) -> Option<String> {
        let mut search_from = 0;
        while let Some(style_start) = html[search_from..].find("<style>") {
            let abs_start = search_from + style_start + 7; // skip "<style>"
            if let Some(style_end) = html[abs_start..].find("</style>") {
                let css = &html[abs_start..abs_start + style_end];
                // Check if this is Tailwind CSS (contains tailwindcss comment or @layer)
                if css.contains("tailwindcss") || css.contains("@layer") {
                    return Some(css.to_string());
                }
            }
            search_from = abs_start;
        }
        None
    }
}
