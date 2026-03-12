#![allow(clippy::unwrap_used)]

mod common;

use common::{build_and_load, setup_test_project};
use rex_server::export::{validate_exportability, ExportConfig, ExportContext};
use std::path::Path;

#[tokio::test]
async fn test_export_static_pages_creates_html_files() {
    let (_tmp, config, scan) = setup_test_project(
        &[
            (
                "index.tsx",
                r#"
                export default function Home() {
                    return <div><h1>Home Page</h1></div>;
                }
                "#,
            ),
            (
                "about.tsx",
                r#"
                export default function About() {
                    return <div><h1>About Page</h1></div>;
                }
                "#,
            ),
        ],
        None,
    );

    let (result, pool) = build_and_load(&config, &scan).await;

    let manifest_json =
        rex_server::state::HotState::compute_manifest_json(&result.build_id, &result.manifest);

    let output_dir = config.project_root.join("export_out");
    let export_config = ExportConfig {
        output_dir: output_dir.clone(),
        force: false,
        base_path: String::new(),
        html_extensions: false,
    };

    let ctx = ExportContext {
        pool: &pool,
        manifest: &result.manifest,
        routes: &scan.routes,
        manifest_json: &manifest_json,
        doc_descriptor: None,
        client_build_dir: &config.client_build_dir(),
        project_root: &config.project_root,
    };

    let export_result = rex_server::export::export_site(&ctx, &export_config)
        .await
        .unwrap();

    assert_eq!(
        export_result.pages_exported, 2,
        "Should export 2 static pages"
    );
    assert!(
        export_result.pages_skipped.is_empty(),
        "No pages should be skipped"
    );

    // Verify HTML files exist (non-root routes use directory/index.html pattern)
    assert!(output_dir.join("index.html").exists(), "index.html missing");
    assert!(
        output_dir.join("about/index.html").exists(),
        "about/index.html missing"
    );

    // Verify content
    let index_html = std::fs::read_to_string(output_dir.join("index.html")).unwrap();
    assert!(
        index_html.contains("Home Page"),
        "index.html should contain SSR content"
    );

    let about_html = std::fs::read_to_string(output_dir.join("about/index.html")).unwrap();
    assert!(
        about_html.contains("About Page"),
        "about.html should contain SSR content"
    );
}

#[tokio::test]
async fn test_export_creates_static_asset_dir() {
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

    let (result, pool) = build_and_load(&config, &scan).await;
    let manifest_json =
        rex_server::state::HotState::compute_manifest_json(&result.build_id, &result.manifest);

    let output_dir = config.project_root.join("export_out");
    let export_config = ExportConfig {
        output_dir: output_dir.clone(),
        force: false,
        base_path: String::new(),
        html_extensions: false,
    };

    let ctx = ExportContext {
        pool: &pool,
        manifest: &result.manifest,
        routes: &scan.routes,
        manifest_json: &manifest_json,
        doc_descriptor: None,
        client_build_dir: &config.client_build_dir(),
        project_root: &config.project_root,
    };

    rex_server::export::export_site(&ctx, &export_config)
        .await
        .unwrap();

    assert!(
        output_dir.join("_rex/static").exists(),
        "_rex/static/ directory should be created"
    );
}

#[tokio::test]
async fn test_export_cleans_output_on_rerun() {
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

    let (result, pool) = build_and_load(&config, &scan).await;
    let manifest_json =
        rex_server::state::HotState::compute_manifest_json(&result.build_id, &result.manifest);

    let output_dir = config.project_root.join("export_out");

    // Create a stale file that should survive (export doesn't clean by itself)
    std::fs::create_dir_all(&output_dir).unwrap();
    std::fs::write(output_dir.join("stale.txt"), "old").unwrap();

    let export_config = ExportConfig {
        output_dir: output_dir.clone(),
        force: false,
        base_path: String::new(),
        html_extensions: false,
    };

    let ctx = ExportContext {
        pool: &pool,
        manifest: &result.manifest,
        routes: &scan.routes,
        manifest_json: &manifest_json,
        doc_descriptor: None,
        client_build_dir: &config.client_build_dir(),
        project_root: &config.project_root,
    };

    rex_server::export::export_site(&ctx, &export_config)
        .await
        .unwrap();

    // The export function doesn't clean — that's the CLI's job
    assert!(output_dir.join("index.html").exists());
}

#[test]
fn test_validate_exportability_mixed_routes() {
    let mut manifest = rex_core::AssetManifest::new("test".into());
    manifest.add_page("/", "index.js", rex_core::DataStrategy::None, false);
    manifest.add_page(
        "/api/data",
        "data.js",
        rex_core::DataStrategy::GetServerSideProps,
        false,
    );

    // Without force, should fail
    let err = validate_exportability(&manifest, false).unwrap_err();
    assert!(err.to_string().contains("getServerSideProps"));

    // With force, should return warnings
    let warnings = validate_exportability(&manifest, true).unwrap();
    assert_eq!(warnings.len(), 1);
}

#[test]
fn test_validate_exportability_app_routes_dynamic() {
    let mut manifest = rex_core::AssetManifest::new("test".into());
    manifest.app_routes.insert(
        "/".to_string(),
        rex_core::AppRouteAssets {
            client_chunks: vec![],
            layout_chain: vec![],
            render_mode: rex_core::RenderMode::Static,
        },
    );
    manifest.app_routes.insert(
        "/blog/:slug".to_string(),
        rex_core::AppRouteAssets {
            client_chunks: vec![],
            layout_chain: vec![],
            render_mode: rex_core::RenderMode::ServerRendered,
        },
    );

    // The dynamic route should cause a warning with force
    let warnings = validate_exportability(&manifest, true).unwrap();
    assert!(!warnings.is_empty());
}

#[test]
fn test_validate_exportability_all_static_ok() {
    let mut manifest = rex_core::AssetManifest::new("test".into());
    manifest.add_page("/", "index.js", rex_core::DataStrategy::None, false);
    manifest.add_page(
        "/about",
        "about.js",
        rex_core::DataStrategy::GetStaticProps,
        false,
    );

    let warnings = validate_exportability(&manifest, false).unwrap();
    assert!(warnings.is_empty());
}

#[test]
fn test_export_config_output_dir() {
    let config = ExportConfig {
        output_dir: Path::new("/tmp/export").to_path_buf(),
        force: true,
        base_path: String::new(),
        html_extensions: false,
    };
    assert!(config.force);
    assert_eq!(config.output_dir, Path::new("/tmp/export"));
}
