#![allow(clippy::unwrap_used)]

use super::*;

#[test]
fn route_to_file_path_root() {
    let output = Path::new("/out");
    assert_eq!(
        route_to_file_path(output, "/"),
        PathBuf::from("/out/index.html")
    );
}

#[test]
fn route_to_file_path_simple() {
    let output = Path::new("/out");
    assert_eq!(
        route_to_file_path(output, "/about"),
        PathBuf::from("/out/about.html")
    );
}

#[test]
fn route_to_file_path_nested() {
    let output = Path::new("/out");
    assert_eq!(
        route_to_file_path(output, "/docs/intro"),
        PathBuf::from("/out/docs/intro.html")
    );
}

#[test]
fn validate_exportability_all_static() {
    let mut manifest = AssetManifest::new("test".into());
    manifest.add_page("/", "index.js", DataStrategy::None, false);
    manifest.add_page("/about", "about.js", DataStrategy::GetStaticProps, false);

    let warnings = validate_exportability(&manifest, false).unwrap();
    assert!(warnings.is_empty());
}

#[test]
fn validate_exportability_gssp_fails() {
    let mut manifest = AssetManifest::new("test".into());
    manifest.add_page("/", "index.js", DataStrategy::None, false);
    manifest.add_page("/dash", "dash.js", DataStrategy::GetServerSideProps, false);

    let err = validate_exportability(&manifest, false).unwrap_err();
    assert!(err.to_string().contains("getServerSideProps"));
}

#[test]
fn validate_exportability_gssp_force() {
    let mut manifest = AssetManifest::new("test".into());
    manifest.add_page("/", "index.js", DataStrategy::None, false);
    manifest.add_page("/dash", "dash.js", DataStrategy::GetServerSideProps, false);

    let warnings = validate_exportability(&manifest, true).unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("getServerSideProps"));
}

#[test]
fn validate_exportability_dynamic_fails() {
    let mut manifest = AssetManifest::new("test".into());
    manifest.add_page("/blog/:slug", "slug.js", DataStrategy::None, true);

    let err = validate_exportability(&manifest, false).unwrap_err();
    assert!(err.to_string().contains("dynamic segments"));
}

#[test]
fn write_html_creates_parent_dirs() {
    let tmp = std::env::temp_dir().join("rex_export_test");
    let _ = std::fs::remove_dir_all(&tmp);
    let path = tmp.join("nested").join("page.html");
    write_html_file(&path, "<html></html>").unwrap();
    assert!(path.exists());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "<html></html>");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn rewrite_asset_paths_empty_base() {
    let html = r#"<script src="/_rex/static/app.js"></script>"#;
    assert_eq!(rewrite_asset_paths(html, ""), html);
}

#[test]
fn rewrite_asset_paths_with_base() {
    let html = r#"<link href="/_rex/static/style.css" /><script src="/_rex/router.js"></script>"#;
    let result = rewrite_asset_paths(html, "/rex");
    assert!(result.contains(r#"href="/rex/_rex/static/style.css""#));
    assert!(result.contains(r#"src="/rex/_rex/router.js""#));
}

#[test]
fn rewrite_asset_paths_multiple_occurrences() {
    let html = "/_rex/static/a.js /_rex/static/b.js /_rex/data/c.json";
    let result = rewrite_asset_paths(html, "/docs");
    assert!(result.contains("/docs/_rex/static/a.js"));
    assert!(result.contains("/docs/_rex/static/b.js"));
    assert!(result.contains("/docs/_rex/data/c.json"));
}

#[test]
fn rewrite_asset_paths_rewrites_nav_links() {
    let html = r#"<a href="/about">About</a><a href="/getting-started">Start</a>"#;
    let result = rewrite_asset_paths(html, "/rex");
    assert!(result.contains(r#"href="/rex/about""#));
    assert!(result.contains(r#"href="/rex/getting-started""#));
}

#[test]
fn rewrite_asset_paths_preserves_external_links() {
    let html = r#"<a href="https://github.com">GH</a>"#;
    let result = rewrite_asset_paths(html, "/rex");
    // No <head> tag, so no script injection — external link preserved as-is
    assert!(result.contains(r#"href="https://github.com""#));
}

#[test]
fn rewrite_asset_paths_no_double_prefix() {
    let html = r#"<link href="/_rex/static/s.css" /><a href="/about">A</a>"#;
    let result = rewrite_asset_paths(html, "/rex");
    assert!(result.contains(r#"href="/rex/_rex/static/s.css""#));
    assert!(result.contains(r#"href="/rex/about""#));
    assert!(!result.contains("/rex/rex/"));
}

#[test]
fn rewrite_asset_paths_injects_base_path_global() {
    let html = r#"<html><head><meta charset="utf-8" /></head><body></body></html>"#;
    let result = rewrite_asset_paths(html, "/rex");
    assert!(result.contains(r#"<script>window.__REX_BASE_PATH="/rex"</script>"#));
    // Script is injected right after <head>
    let head_pos = result.find("<head>").unwrap();
    let script_pos = result.find("__REX_BASE_PATH").unwrap();
    assert!(script_pos > head_pos);
}

#[test]
fn rewrite_asset_paths_no_injection_without_head() {
    // JS files (e.g. router.js) don't have <head> — no injection
    let js = r#"var x = "/_rex/data/test.json";"#;
    let result = rewrite_asset_paths(js, "/rex");
    assert!(!result.contains("__REX_BASE_PATH"));
}

#[test]
fn inject_base_path_global_into_rsc_html() {
    let html = "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\" /></head><body></body></html>";
    let result = inject_base_path_global(html, "/docs");
    assert!(result.contains(r#"<head><script>window.__REX_BASE_PATH="/docs"</script><meta"#));
}

#[test]
fn rewrite_nav_links_adds_html_extension() {
    let html = r#"<a href="/about">About</a><a href="/getting-started">Start</a>"#;
    let result = rewrite_nav_links_to_html(html);
    assert!(result.contains(r#"href="/about.html""#));
    assert!(result.contains(r#"href="/getting-started.html""#));
}

#[test]
fn rewrite_nav_links_preserves_root() {
    let html = r#"<a href="/">Home</a>"#;
    let result = rewrite_nav_links_to_html(html);
    assert!(result.contains(r#"href="/""#));
    assert!(!result.contains(r#"href="/.html""#));
}

#[test]
fn rewrite_nav_links_preserves_asset_links() {
    let html = r#"<link href="/_rex/static/style.css" />"#;
    let result = rewrite_nav_links_to_html(html);
    assert!(result.contains(r#"href="/_rex/static/style.css""#));
}

#[test]
fn rewrite_nav_links_preserves_external_links() {
    let html = r#"<a href="https://github.com">GH</a>"#;
    let result = rewrite_nav_links_to_html(html);
    assert_eq!(result, html);
}

#[test]
fn rewrite_nav_links_preserves_links_with_extension() {
    let html = r#"<a href="/file.pdf">PDF</a>"#;
    let result = rewrite_nav_links_to_html(html);
    assert!(result.contains(r#"href="/file.pdf""#));
}

#[test]
fn rewrite_nav_links_nested_paths() {
    let html = r#"<a href="/features/routing">Routing</a>"#;
    let result = rewrite_nav_links_to_html(html);
    assert!(result.contains(r#"href="/features/routing.html""#));
}

#[test]
fn rewrite_nav_links_with_hash_fragment() {
    let html = r#"<a href="/about#team">Team</a>"#;
    let result = rewrite_nav_links_to_html(html);
    assert!(result.contains(r#"href="/about.html#team""#));
}

#[test]
fn rewrite_nav_links_in_rsc_flight_data() {
    // RSC flight data uses JSON: "href":"/path"
    let flight = r#"["$","a",null,{"href":"/getting-started","children":"Quickstart"}]"#;
    let result = rewrite_nav_links_to_html(flight);
    assert!(
        result.contains(r#""href":"/getting-started.html""#),
        "Flight data hrefs should get .html: {result}"
    );
}

#[test]
fn rewrite_nav_links_mixed_html_and_flight() {
    let html = r#"<a href="/about">About</a><script>"href":"/docs"</script>"#;
    let result = rewrite_nav_links_to_html(html);
    assert!(result.contains(r#"href="/about.html""#));
    assert!(result.contains(r#""href":"/docs.html""#));
}

#[test]
fn rewrite_nav_links_preserves_trailing_slash() {
    let html = r#"<a href="/about/">About</a>"#;
    let result = rewrite_nav_links_to_html(html);
    // Trailing slash paths should be preserved as-is, not become /about/.html
    assert!(result.contains(r#"href="/about/""#));
    assert!(!result.contains("/about/.html"));
}

#[test]
fn rewrite_nav_links_with_query_string() {
    let html = r#"<a href="/search?q=test">Search</a>"#;
    let result = rewrite_nav_links_to_html(html);
    assert!(result.contains(r#"href="/search.html?q=test""#));
}

#[test]
fn rewrite_nav_links_dot_in_directory() {
    let html = r#"<a href="/foo/bar.baz/qux">Page</a>"#;
    let result = rewrite_nav_links_to_html(html);
    assert!(result.contains(r#"href="/foo/bar.baz/qux.html""#));
}

#[test]
fn inject_base_path_global_escapes_special_chars() {
    let html = "<html><head></head><body></body></html>";
    let malicious = r#"/rex";</script><script>alert(1)//"#;
    let result = inject_base_path_global(html, malicious);
    assert!(result.contains("__REX_BASE_PATH="));
    // The </script> inside the value must be escaped so the HTML parser
    // doesn't close the script tag prematurely and execute injected code.
    // Count that there's exactly one <script> open and one </script> close.
    assert_eq!(result.matches("<script>").count(), 1);
    assert_eq!(result.matches("</script>").count(), 1);
}
