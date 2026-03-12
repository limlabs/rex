#![allow(clippy::unwrap_used)]

mod common;

use common::setup_mock_node_modules;
use rex_core::{PageType, ProjectConfig, RexConfig, Route};
use rex_router::ScanResult;
use std::fs;
use std::path::PathBuf;

/// Build a project with font imports and verify the font CSS and preloads.
#[tokio::test]
async fn test_font_pages_router_build() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();

    setup_mock_node_modules(&root);

    let pages_dir = root.join("pages");
    fs::create_dir_all(&pages_dir).unwrap();

    let index_path = pages_dir.join("index.tsx");
    fs::write(
        &index_path,
        r#"import { Inter } from 'next/font/google'
const inter = Inter({ weight: '400', subsets: ['latin'], display: 'swap' })
export default function Home() {
    return <div className={inter.className}><h1>Hello</h1></div>;
}
"#,
    )
    .unwrap();

    let config = RexConfig::new(root).with_dev(true);
    let scan = ScanResult {
        routes: vec![Route {
            pattern: "/".to_string(),
            file_path: PathBuf::from("index.tsx"),
            abs_path: index_path,
            dynamic_segments: vec![],
            page_type: PageType::Regular,
            specificity: 10,
        }],
        api_routes: vec![],
        app: None,
        document: None,
        error: None,
        not_found: None,
        middleware: None,
        app_scan: None,
        mcp_tools: vec![],
    };

    let result = rex_build::build_bundles(&config, &scan, &ProjectConfig::default())
        .await
        .expect("build should succeed");

    // Font CSS should be present in the manifest
    assert!(
        !result.manifest.global_css.is_empty(),
        "Should have global CSS with font styles"
    );

    let has_font_css = result
        .manifest
        .global_css
        .iter()
        .any(|name| name.contains("fonts-"));
    assert!(
        has_font_css,
        "Should have a fonts-*.css entry in global_css"
    );

    // Verify font CSS content
    let font_css_entry = result
        .manifest
        .css_contents
        .iter()
        .find(|(k, _)| k.contains("fonts-"));
    assert!(
        font_css_entry.is_some(),
        "Should have font CSS in css_contents"
    );
    let (_, css_content) = font_css_entry.unwrap();

    // Font CSS should contain @font-face or @import (depending on network)
    assert!(
        css_content.contains("@font-face") || css_content.contains("@import"),
        "Font CSS should contain @font-face or @import fallback: {css_content}"
    );

    // Should contain the scoped font family name
    assert!(
        css_content.contains("__font_Inter_"),
        "Font CSS should contain scoped Inter family: {css_content}"
    );

    // Verify font class CSS is present
    assert!(
        css_content.contains("font-family:"),
        "Font CSS should contain font-family class rule: {css_content}"
    );
}

/// Verify that multiple fonts in a single page are handled correctly.
#[tokio::test]
async fn test_font_multiple_fonts_single_page() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();

    setup_mock_node_modules(&root);

    let pages_dir = root.join("pages");
    fs::create_dir_all(&pages_dir).unwrap();

    let index_path = pages_dir.join("index.tsx");
    fs::write(
        &index_path,
        r#"import { Inter, Roboto_Mono } from 'rex/font/google'
const inter = Inter({ weight: '400' })
const mono = Roboto_Mono({ weight: ['400', '700'], variable: '--font-mono' })
export default function Home() {
    return <div className={inter.className}><code className={mono.className}>code</code></div>;
}
"#,
    )
    .unwrap();

    let config = RexConfig::new(root).with_dev(true);
    let scan = ScanResult {
        routes: vec![Route {
            pattern: "/".to_string(),
            file_path: PathBuf::from("index.tsx"),
            abs_path: index_path,
            dynamic_segments: vec![],
            page_type: PageType::Regular,
            specificity: 10,
        }],
        api_routes: vec![],
        app: None,
        document: None,
        error: None,
        not_found: None,
        middleware: None,
        app_scan: None,
        mcp_tools: vec![],
    };

    let result = rex_build::build_bundles(&config, &scan, &ProjectConfig::default())
        .await
        .expect("build should succeed");

    // Both fonts should be in the CSS
    let font_css: String = result
        .manifest
        .css_contents
        .iter()
        .filter(|(k, _)| k.contains("fonts-"))
        .map(|(_, v)| v.clone())
        .collect();

    assert!(
        font_css.contains("__font_Inter_"),
        "Should contain Inter font: {font_css}"
    );
    assert!(
        font_css.contains("__font_Roboto_Mono_"),
        "Should contain Roboto Mono font: {font_css}"
    );

    // CSS variable should be declared
    assert!(
        font_css.contains("--font-mono"),
        "Should contain CSS variable: {font_css}"
    );
}

/// Verify font support in app router layout files.
#[tokio::test]
async fn test_font_app_router_layout() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();

    // Use real node_modules from app-router fixture
    let fixture_nm = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("fixtures/app-router/node_modules");
    if !fixture_nm.exists() {
        eprintln!("Skipping test: fixtures/app-router/node_modules not found");
        return;
    }
    std::os::unix::fs::symlink(
        fixture_nm.canonicalize().unwrap(),
        root.join("node_modules"),
    )
    .unwrap();
    fs::write(
        root.join("package.json"),
        r#"{"name":"font-test","private":true}"#,
    )
    .unwrap();

    let app_dir = root.join("app");
    fs::create_dir_all(&app_dir).unwrap();

    let layout_path = app_dir.join("layout.tsx");
    fs::write(
        &layout_path,
        r#"import { Inter } from 'next/font/google'
const inter = Inter({ weight: '400', subsets: ['latin'] })
export default function RootLayout({ children }) {
    return <html className={inter.className}><body>{children}</body></html>;
}
"#,
    )
    .unwrap();

    let page_path = app_dir.join("page.tsx");
    fs::write(
        &page_path,
        "export default function Home() { return <h1>Hello</h1>; }\n",
    )
    .unwrap();

    let config = RexConfig::new(root.clone()).with_dev(true);

    use rex_core::app_route::{AppRoute, AppScanResult, AppSegment};

    let app_scan = AppScanResult {
        root: AppSegment {
            segment: String::new(),
            layout: Some(layout_path.clone()),
            page: Some(page_path.clone()),
            route: None,
            loading: None,
            error_boundary: None,
            not_found: None,
            children: vec![],
        },
        routes: vec![AppRoute {
            pattern: "/".to_string(),
            page_path: page_path.clone(),
            layout_chain: vec![layout_path.clone()],
            loading_chain: vec![None],
            error_chain: vec![None],
            dynamic_segments: vec![],
            specificity: 10,
            route_group: None,
        }],
        api_routes: vec![],
        root_layout: Some(layout_path),
    };

    // Use the pages router scan as empty, and app_scan for the app router
    let scan = ScanResult {
        routes: vec![],
        api_routes: vec![],
        app: None,
        document: None,
        error: None,
        not_found: None,
        middleware: None,
        app_scan: Some(app_scan),
        mcp_tools: vec![],
    };

    let result = rex_build::build_bundles(&config, &scan, &ProjectConfig::default())
        .await
        .expect("build should succeed with font imports in app router");

    // App router font CSS should be present
    let has_app_font_css = result
        .manifest
        .global_css
        .iter()
        .any(|name| name.contains("app-fonts-"));
    assert!(
        has_app_font_css,
        "Should have app-fonts-*.css in global_css: {:?}",
        result.manifest.global_css
    );

    // Font CSS should contain the scoped font family
    let app_font_css: String = result
        .manifest
        .css_contents
        .iter()
        .filter(|(k, _)| k.contains("app-fonts-"))
        .map(|(_, v)| v.clone())
        .collect();

    assert!(
        app_font_css.contains("__font_Inter_"),
        "App font CSS should contain scoped Inter family: {app_font_css}"
    );
}
