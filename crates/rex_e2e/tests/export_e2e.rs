//! E2E tests for `rex export` — static site generation.
//!
//! These tests run `rex export` against the app-router fixture and verify
//! that the output directory contains correct HTML files and static assets.
//!
//! Run with: cargo test -p rex_e2e --test export_e2e -- --ignored
//!
//! Prerequisites:
//!   - `cargo build` (debug or release)
//!   - `cd fixtures/app-router && npm install`

#[allow(clippy::unwrap_used)]
mod export {
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use std::sync::OnceLock;

    static EXPORT_OUTPUT: OnceLock<PathBuf> = OnceLock::new();

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

    /// Run `rex export` once and return the output directory path.
    fn ensure_export() -> &'static PathBuf {
        EXPORT_OUTPUT.get_or_init(|| {
            let bin = rex_binary();
            let root = fixture_root();
            let output_dir = root.join(".rex/export");

            eprintln!("[export-e2e] Running rex export...");
            eprintln!("[export-e2e] Binary: {}", bin.display());
            eprintln!("[export-e2e] Root: {}", root.display());

            let output = Command::new(&bin)
                .arg("export")
                .arg("--root")
                .arg(&root)
                .arg("--force")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .unwrap_or_else(|e| panic!("Failed to run rex export: {e}"));

            let stderr = String::from_utf8_lossy(&output.stderr);
            eprintln!("[export-e2e] stderr:\n{stderr}");

            assert!(
                output.status.success(),
                "[export-e2e] rex export failed with status: {}\nstderr: {stderr}",
                output.status,
            );

            output_dir
        })
    }

    // -------------------------------------------------------
    // Export output structure tests
    // -------------------------------------------------------

    #[tokio::test]
    #[ignore]
    async fn export_creates_index_html() {
        let output = ensure_export();
        let index = output.join("index.html");
        assert!(index.exists(), "index.html should exist in export output");

        let html = std::fs::read_to_string(&index).unwrap();
        assert!(
            html.contains("<!DOCTYPE html>") || html.contains("<!doctype html>"),
            "index.html should have DOCTYPE"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn export_creates_about_html() {
        let output = ensure_export();
        let about = output.join("about.html");
        assert!(about.exists(), "about.html should exist in export output");

        let html = std::fs::read_to_string(&about).unwrap();
        assert!(
            html.contains("About"),
            "about.html should contain page content"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn export_creates_nested_route_html() {
        let output = ensure_export();
        let dashboard = output.join("dashboard.html");
        assert!(
            dashboard.exists(),
            "dashboard.html should exist in export output"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn export_copies_client_assets() {
        let output = ensure_export();
        let static_dir = output.join("_rex/static");
        assert!(
            static_dir.exists(),
            "_rex/static/ should exist in export output"
        );

        // Should contain at least one subdirectory or file (client chunks)
        let has_content = std::fs::read_dir(&static_dir)
            .unwrap()
            .flatten()
            .next()
            .is_some();
        assert!(has_content, "_rex/static/ should contain client assets");
    }

    #[tokio::test]
    #[ignore]
    async fn export_html_contains_ssr_content() {
        let output = ensure_export();
        let index = output.join("index.html");
        let html = std::fs::read_to_string(&index).unwrap();

        // App router renders content directly into the body
        assert!(
            html.contains("Rex!"),
            "Exported HTML should contain SSR-rendered page content"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn export_html_has_script_tags() {
        let output = ensure_export();
        let index = output.join("index.html");
        let html = std::fs::read_to_string(&index).unwrap();

        assert!(
            html.contains("<script"),
            "Exported HTML should include script tags for hydration"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn export_skips_dynamic_routes_with_force() {
        let output = ensure_export();
        // /blog/[slug] is dynamic — should not be exported
        let blog_slug = output.join("blog/[slug].html");
        assert!(
            !blog_slug.exists(),
            "Dynamic route blog/[slug] should not be exported"
        );
    }
}
