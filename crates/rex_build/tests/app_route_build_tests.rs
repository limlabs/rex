#![allow(clippy::unwrap_used)]

mod common;

use common::setup_test_project;
use rex_build::build_bundles;
use rex_core::app_route::{AppApiRoute, AppScanResult, AppSegment};
use rex_core::{ProjectConfig, RexConfig};
use std::fs;

fn empty_root_segment() -> AppSegment {
    AppSegment {
        segment: "".into(),
        layout: None,
        page: None,
        route: None,
        loading: None,
        error_boundary: None,
        not_found: None,
        children: vec![],
    }
}

/// Test that the server bundle (build_server_bundle) includes app route handler
/// registrations when pages/ routes coexist with app/ api_routes.
#[tokio::test]
async fn test_server_bundle_app_route_handlers() {
    let (_tmp, config, mut scan) = setup_test_project(
        &[(
            "index.tsx",
            "export default function Home() { return <div>Home</div>; }",
        )],
        None,
    );
    let app_dir = config.project_root.join("app").join("api").join("hello");
    fs::create_dir_all(&app_dir).unwrap();
    let handler_path = app_dir.join("route.ts");
    fs::write(
        &handler_path,
        "export function GET(req) { return new Response('ok'); }",
    )
    .unwrap();

    scan.app_scan = Some(AppScanResult {
        root: empty_root_segment(),
        routes: vec![],
        api_routes: vec![AppApiRoute {
            pattern: "/api/hello".into(),
            handler_path: handler_path.clone(),
            dynamic_segments: vec![],
            specificity: 100,
        }],
        root_layout: None,
    });

    let result = build_bundles(&config, &scan, &ProjectConfig::default())
        .await
        .unwrap();
    let bundle = fs::read_to_string(&result.server_bundle_path).unwrap();
    assert!(
        bundle.contains("__rex_app_route_handlers"),
        "server bundle should register app route handlers"
    );
}

/// Test that the minimal server bundle (build_minimal_server_bundle) includes
/// app route handler registrations when there are no pages/ routes.
#[tokio::test]
async fn test_minimal_server_bundle_app_route_handlers() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    common::setup_mock_node_modules(&root);
    fs::create_dir_all(root.join("pages")).unwrap();

    let app_dir = root.join("app").join("api").join("data");
    fs::create_dir_all(&app_dir).unwrap();
    let handler_path = app_dir.join("route.ts");
    fs::write(
        &handler_path,
        "export function GET() { return new Response('data'); }",
    )
    .unwrap();

    let config = RexConfig::new(root).with_dev(true);
    let scan = rex_router::ScanResult {
        routes: vec![],
        api_routes: vec![],
        app: None,
        document: None,
        error: None,
        not_found: None,
        middleware: None,
        app_scan: Some(AppScanResult {
            root: empty_root_segment(),
            routes: vec![],
            api_routes: vec![AppApiRoute {
                pattern: "/api/data".into(),
                handler_path,
                dynamic_segments: vec![],
                specificity: 100,
            }],
            root_layout: None,
        }),
        mcp_tools: vec![],
    };

    let result = build_bundles(&config, &scan, &ProjectConfig::default())
        .await
        .unwrap();
    let bundle = fs::read_to_string(&result.server_bundle_path).unwrap();
    assert!(
        bundle.contains("__rex_app_route_handlers"),
        "minimal bundle should register app route handlers"
    );
    assert!(
        bundle.contains("/api/data"),
        "minimal bundle should contain route pattern"
    );
}
