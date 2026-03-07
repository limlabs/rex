#![allow(clippy::unwrap_used)]

mod common;

use common::{setup_mock_node_modules, setup_test_project, setup_test_project_full};
use rex_build::{build_bundles, AssetManifest};
use rex_core::{PageType, ProjectConfig, RexConfig, Route};
use rex_router::ScanResult;
use std::fs;
use std::path::PathBuf;

#[tokio::test]
async fn test_server_bundle_structure() {
    let (_tmp, config, scan) = setup_test_project(
        &[(
            "index.tsx",
            r#"
                export default function Home() {
                    return <div>Hello</div>;
                }
                "#,
        )],
        None,
    );
    let result = build_bundles(&config, &scan, &ProjectConfig::default())
        .await
        .unwrap();
    let bundle = fs::read_to_string(&result.server_bundle_path).unwrap();

    // V8 polyfills (injected as banner)
    assert!(
        bundle.contains("globalThis.process"),
        "should have process polyfill"
    );
    assert!(
        bundle.contains("MessageChannel"),
        "should have MessageChannel polyfill"
    );
    assert!(
        bundle.contains("globalThis.Buffer"),
        "should have Buffer polyfill"
    );

    // Page registry
    assert!(bundle.contains("__rex_pages"), "should init page registry");

    // SSR runtime functions
    assert!(
        bundle.contains("__rex_render_page"),
        "should have render function"
    );
    assert!(
        bundle.contains("__rex_get_server_side_props"),
        "should have GSSP executor"
    );
    assert!(
        bundle.contains("__rex_resolve_gssp"),
        "should have GSSP resolver"
    );
    assert!(
        bundle.contains("__REX_ASYNC__"),
        "should have async sentinel"
    );
}

#[tokio::test]
async fn test_server_bundle_iife_format() {
    let (_tmp, config, scan) = setup_test_project(
        &[(
            "index.tsx",
            r#"
                import React from 'react';
                export default function Home() {
                    return <div>Hello</div>;
                }
                export async function getServerSideProps() {
                    return { props: {} };
                }
                "#,
        )],
        None,
    );
    let result = build_bundles(&config, &scan, &ProjectConfig::default())
        .await
        .unwrap();
    let bundle = fs::read_to_string(&result.server_bundle_path).unwrap();

    // Should be IIFE format — no raw ESM import/export at top level
    assert!(
        !bundle.contains("\nimport "),
        "should not have ESM import statements"
    );
    assert!(
        !bundle.contains("\nexport "),
        "should not have ESM export statements"
    );

    // Should be self-contained (React bundled in, not externalized)
    assert!(
        bundle.contains("createElement"),
        "should contain bundled React createElement"
    );
}

#[tokio::test]
async fn test_server_bundle_multiple_pages() {
    let (_tmp, config, scan) = setup_test_project(
        &[
            (
                "index.tsx",
                "export default function Home() { return <div>Home</div>; }",
            ),
            (
                "about.tsx",
                "export default function About() { return <div>About</div>; }",
            ),
            (
                "blog/[slug].tsx",
                "export default function Post({ slug }) { return <div>{slug}</div>; }",
            ),
        ],
        None,
    );
    let result = build_bundles(&config, &scan, &ProjectConfig::default())
        .await
        .unwrap();
    let bundle = fs::read_to_string(&result.server_bundle_path).unwrap();

    assert!(
        bundle.contains("__rex_pages[\"index\"]") || bundle.contains("__rex_pages['index']"),
        "should have index page"
    );
    assert!(
        bundle.contains("__rex_pages[\"about\"]") || bundle.contains("__rex_pages['about']"),
        "should have about page"
    );
    assert!(
        bundle.contains("__rex_pages[\"blog/[slug]\"]")
            || bundle.contains("__rex_pages['blog/[slug]']"),
        "should have dynamic page"
    );
}

#[tokio::test]
async fn test_server_bundle_with_app() {
    let (_tmp, config, scan) = setup_test_project(
        &[(
            "index.tsx",
            "export default function Home() { return <div>Home</div>; }",
        )],
        Some(
            r#"
                export default function App({ Component, pageProps }) {
                    return <Component {...pageProps} />;
                }
                "#,
        ),
    );
    let result = build_bundles(&config, &scan, &ProjectConfig::default())
        .await
        .unwrap();
    let bundle = fs::read_to_string(&result.server_bundle_path).unwrap();

    assert!(bundle.contains("__rex_app"), "should register _app");
}

#[tokio::test]
async fn test_client_bundles_per_page() {
    let (_tmp, config, scan) = setup_test_project(
        &[
            (
                "index.tsx",
                "export default function Home() { return <div>Home</div>; }",
            ),
            (
                "about.tsx",
                "export default function About() { return <div>About</div>; }",
            ),
        ],
        None,
    );
    let result = build_bundles(&config, &scan, &ProjectConfig::default())
        .await
        .unwrap();
    let client_dir = config.client_build_dir();
    let build_hash = &result.build_id[..8];

    // Each page should have its own client chunk
    let index_path = client_dir.join(format!("index-{build_hash}.js"));
    let about_path = client_dir.join(format!("about-{build_hash}.js"));
    assert!(index_path.exists(), "index client chunk should exist");
    assert!(about_path.exists(), "about client chunk should exist");

    // Client chunks should have hydration bootstrap
    let index_js = fs::read_to_string(&index_path).unwrap();
    assert!(
        index_js.contains("hydrateRoot"),
        "should have hydration code"
    );
    assert!(
        index_js.contains("__REX_DATA__"),
        "should reference data element"
    );

    // Client chunks should NOT have getServerSideProps
    assert!(
        !index_js.contains("getServerSideProps"),
        "client chunk should strip GSSP"
    );
}

#[tokio::test]
async fn test_manifest_contents() {
    let (_tmp, config, scan) = setup_test_project(
        &[
            (
                "index.tsx",
                "export default function Home() { return <div>Home</div>; }",
            ),
            (
                "about.tsx",
                "export default function About() { return <div>About</div>; }",
            ),
        ],
        None,
    );
    let result = build_bundles(&config, &scan, &ProjectConfig::default())
        .await
        .unwrap();

    // Manifest should track both pages
    assert!(
        result.manifest.pages.contains_key("/"),
        "manifest should have index route"
    );
    assert!(
        result.manifest.pages.contains_key("/about"),
        "manifest should have about route"
    );

    // JS filenames should include build hash
    let hash = &result.build_id[..8];
    assert!(
        result.manifest.pages["/"].js.contains(hash),
        "JS filename should include build hash"
    );

    // Manifest should be saved to disk
    let manifest_path = config.manifest_path();
    assert!(manifest_path.exists(), "manifest.json should be written");

    let loaded = AssetManifest::load(&manifest_path).unwrap();
    assert_eq!(loaded.build_id, result.build_id);
    assert_eq!(loaded.pages.len(), 2);
}

#[tokio::test]
async fn test_server_bundle_with_document() {
    let (_tmp, config, scan) = setup_test_project_full(
        &[(
            "index.tsx",
            "export default function Home() { return <div>Home</div>; }",
        )],
        None,
        Some(
            r#"
                import React from 'react';
                export default function Document() {
                    return React.createElement('html', { lang: 'en' });
                }
                "#,
        ),
    );
    let result = build_bundles(&config, &scan, &ProjectConfig::default())
        .await
        .unwrap();
    let bundle = fs::read_to_string(&result.server_bundle_path).unwrap();

    assert!(
        bundle.contains("__rex_document"),
        "should register _document"
    );
}

#[tokio::test]
async fn test_global_css_from_app() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();

    setup_mock_node_modules(&root);

    let pages_dir = root.join("pages");
    let styles_dir = root.join("styles");
    fs::create_dir_all(&pages_dir).unwrap();
    fs::create_dir_all(&styles_dir).unwrap();

    // Create CSS file
    fs::write(styles_dir.join("globals.css"), "body { color: red; }").unwrap();

    // Create index page
    let index_path = pages_dir.join("index.tsx");
    fs::write(
        &index_path,
        "export default function Home() { return <div>Home</div>; }",
    )
    .unwrap();

    // Create _app that imports CSS
    let app_path = pages_dir.join("_app.tsx");
    fs::write(
        &app_path,
        "import '../styles/globals.css';\nexport default function App({ Component, pageProps }) { return <Component {...pageProps} />; }",
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
        app: Some(Route {
            pattern: String::new(),
            file_path: PathBuf::from("_app.tsx"),
            abs_path: app_path,
            dynamic_segments: vec![],
            page_type: PageType::App,
            specificity: 0,
        }),
        document: None,
        error: None,
        not_found: None,
        middleware: None,
        app_scan: None,
        mcp_tools: vec![],
    };

    let result = build_bundles(&config, &scan, &ProjectConfig::default())
        .await
        .unwrap();

    // Manifest should have global CSS
    assert_eq!(
        result.manifest.global_css.len(),
        1,
        "should have 1 global CSS file"
    );
    assert!(
        result.manifest.global_css[0].starts_with("globals-"),
        "CSS filename should be globals-*"
    );
    assert!(
        result.manifest.global_css[0].ends_with(".css"),
        "CSS filename should end in .css"
    );

    // CSS file should exist in client output
    let client_dir = config.client_build_dir();
    let css_path = client_dir.join(&result.manifest.global_css[0]);
    assert!(css_path.exists(), "CSS file should exist in client output");
    let css_content = fs::read_to_string(&css_path).unwrap();
    assert!(
        css_content.contains("color: red"),
        "CSS file should have original content"
    );

    // Manifest should be loadable and retain global_css
    let loaded = AssetManifest::load(&config.manifest_path()).unwrap();
    assert_eq!(loaded.global_css.len(), 1);
}

#[tokio::test]
async fn test_client_bundle_app_wrapping() {
    let (_tmp, config, scan) = setup_test_project(
        &[(
            "index.tsx",
            "export default function Home() { return <div>Home</div>; }",
        )],
        Some(
            r#"
                export default function App({ Component, pageProps }) {
                    return <Component {...pageProps} />;
                }
                "#,
        ),
    );
    let result = build_bundles(&config, &scan, &ProjectConfig::default())
        .await
        .unwrap();

    // _app client chunk should exist
    assert!(
        result.manifest.app_script.is_some(),
        "should have app_script in manifest"
    );
    let app_script = result.manifest.app_script.as_ref().unwrap();
    assert!(
        app_script.starts_with("_app-"),
        "app script should be named _app-*"
    );

    // Client page chunk should have _app wrapping logic
    let client_dir = config.client_build_dir();
    let index_js =
        fs::read_to_string(client_dir.join(result.manifest.pages["/"].js.clone())).unwrap();
    assert!(
        index_js.contains("__REX_APP__"),
        "page hydration should check for __REX_APP__"
    );
}

#[tokio::test]
async fn test_next_import_shims() {
    let (_tmp, config, scan) = setup_test_project(
        &[(
            "index.tsx",
            r#"
                import Head from 'next/head';
                import Link from 'next/link';
                export default function Home() {
                    return <div><Head><title>Test</title></Head><Link href="/about">About</Link></div>;
                }
                "#,
        )],
        None,
    );

    // Should build without errors — next/* aliases resolve to rex runtime stubs
    let result = build_bundles(&config, &scan, &ProjectConfig::default())
        .await
        .unwrap();

    // Server bundle should contain the page
    let bundle = fs::read_to_string(&result.server_bundle_path).unwrap();
    assert!(
        bundle.contains("__rex_pages"),
        "server bundle should register pages"
    );

    // Client bundle should exist for the page
    assert!(
        result.manifest.pages.contains_key("/"),
        "manifest should have index page"
    );
}

#[tokio::test]
async fn test_css_modules() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();

    setup_mock_node_modules(&root);

    let pages_dir = root.join("pages");
    let styles_dir = root.join("styles");
    fs::create_dir_all(&pages_dir).unwrap();
    fs::create_dir_all(&styles_dir).unwrap();

    // Create a CSS module file
    fs::write(
        styles_dir.join("Home.module.css"),
        ".container { padding: 20px; }\n.title { font-size: 24px; color: blue; }\n",
    )
    .unwrap();

    // Create a page that imports the CSS module
    let index_path = pages_dir.join("index.tsx");
    fs::write(
        &index_path,
        r#"import styles from '../styles/Home.module.css';
export default function Home() {
    return <div className={styles.container}><h1 className={styles.title}>Hello</h1></div>;
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

    let result = build_bundles(&config, &scan, &ProjectConfig::default())
        .await
        .unwrap();

    // Server bundle should contain the CSS module class mapping
    let bundle = fs::read_to_string(&result.server_bundle_path).unwrap();
    assert!(
        bundle.contains("Home_container_"),
        "server bundle should contain scoped class name for container"
    );
    assert!(
        bundle.contains("Home_title_"),
        "server bundle should contain scoped class name for title"
    );

    // Scoped CSS file should exist in client output
    let client_dir = config.client_build_dir();
    let css_files: Vec<_> = fs::read_dir(&client_dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().to_string_lossy().contains("Home.module-"))
        .collect();
    assert_eq!(css_files.len(), 1, "should have 1 scoped CSS module file");

    let scoped_css = fs::read_to_string(css_files[0].path()).unwrap();
    assert!(
        scoped_css.contains("Home_container_"),
        "scoped CSS should have rewritten class names"
    );
    assert!(
        scoped_css.contains("padding: 20px"),
        "scoped CSS should preserve property values"
    );
    assert!(
        !scoped_css.contains(".container"),
        "scoped CSS should not have original class names"
    );

    // Manifest should track CSS module file for the page
    let page_assets = result.manifest.pages.get("/").expect("should have / page");
    assert!(
        !page_assets.css.is_empty(),
        "page should have CSS assets in manifest"
    );
    assert!(
        page_assets.css[0].contains("Home.module-"),
        "CSS asset should be the scoped module file"
    );
}

/// Test that a project with no pages (app-only) produces a minimal server bundle.
#[tokio::test]
async fn test_minimal_server_bundle_no_pages() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();

    setup_mock_node_modules(&root);

    // Create pages dir but no page files — simulates an app-only project
    let pages_dir = root.join("pages");
    fs::create_dir_all(&pages_dir).unwrap();

    let config = RexConfig::new(root).with_dev(true);
    let scan = ScanResult {
        routes: vec![],
        api_routes: vec![],
        app: None,
        document: None,
        error: None,
        not_found: None,
        middleware: None,
        app_scan: None,
        mcp_tools: vec![],
    };

    let result = build_bundles(&config, &scan, &ProjectConfig::default())
        .await
        .expect("minimal build should succeed");

    let bundle = fs::read_to_string(&result.server_bundle_path).unwrap();

    // Should have stub render functions
    assert!(
        bundle.contains("__rex_render_page"),
        "minimal bundle should have render stub"
    );
    assert!(
        bundle.contains("__rex_get_server_side_props"),
        "minimal bundle should have GSSP stub"
    );
    assert!(
        bundle.contains("__rex_render_document"),
        "minimal bundle should have document stub"
    );

    // Should have V8 polyfills
    assert!(
        bundle.contains("globalThis.process"),
        "minimal bundle should have process polyfill"
    );

    // Manifest should be empty (no pages)
    assert!(
        result.manifest.pages.is_empty(),
        "app-only manifest should have no page entries"
    );
}

/// Test that a project with no package.json and no node_modules can still
/// build using the embedded React packages (zero-config mode).
#[tokio::test]
async fn test_build_without_package_json() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();

    // NO setup_mock_node_modules, NO package.json — pure zero-config
    let pages_dir = root.join("pages");
    fs::create_dir_all(&pages_dir).unwrap();

    let index_path = pages_dir.join("index.tsx");
    fs::write(
        &index_path,
        "export default function Home() { return <div>Hello Zero Config</div>; }",
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

    let result = build_bundles(&config, &scan, &ProjectConfig::default())
        .await
        .expect("build should succeed without package.json");

    // Server bundle should exist and contain React
    let bundle = fs::read_to_string(&result.server_bundle_path).unwrap();
    assert!(
        bundle.contains("__rex_render_page"),
        "should have render function"
    );
    assert!(bundle.contains("__rex_pages"), "should init page registry");

    // Client bundles should exist
    assert!(
        !result.manifest.pages.is_empty(),
        "should have page entries in manifest"
    );
}
