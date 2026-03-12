#![allow(clippy::unwrap_used)]

use rex_build::client_manifest::ClientReferenceManifest;
use rex_build::rsc_entries::{generate_core_entry, generate_group_entry};
use rex_build::server_action_manifest::ServerActionManifest;
use rex_core::app_route::{AppRoute, AppScanResult, AppSegment};
use std::path::{Path, PathBuf};

fn empty_app_scan() -> AppScanResult {
    AppScanResult {
        root: AppSegment {
            segment: "".into(),
            layout: None,
            page: None,
            route: None,
            loading: None,
            error_boundary: None,
            not_found: None,
            children: vec![],
        },
        routes: vec![],
        api_routes: vec![],
        root_layout: None,
    }
}

#[test]
fn core_entry_exports_react_to_globalthis() {
    let entry = generate_core_entry(
        &empty_app_scan(),
        &ClientReferenceManifest::new(),
        &ServerActionManifest::default(),
        Path::new("/project"),
    );
    assert!(entry.contains("globalThis.__rex_react_ns = __react_ns"));
    assert!(entry.contains("renderToReadableStream"));
    assert!(entry.contains("__rex_webpack_bundler_config"));
    assert!(entry.contains("globalThis.__rex_app_layouts"));
    assert!(entry.contains("globalThis.__rex_app_pages"));
}

#[test]
fn core_entry_includes_flight_runtime() {
    let entry = generate_core_entry(
        &empty_app_scan(),
        &ClientReferenceManifest::new(),
        &ServerActionManifest::default(),
        Path::new("/project"),
    );
    assert!(entry.contains("__rex_render_flight"));
    assert!(entry.contains("__rex_render_rsc_to_html"));
}

#[test]
fn core_entry_with_api_routes() {
    let mut scan = empty_app_scan();
    scan.api_routes.push(rex_core::app_route::AppApiRoute {
        pattern: "/api/data".into(),
        handler_path: PathBuf::from("/project/app/api/data/route.ts"),
        dynamic_segments: vec![],
        specificity: 100,
    });
    let entry = generate_core_entry(
        &scan,
        &ClientReferenceManifest::new(),
        &ServerActionManifest::default(),
        Path::new("/project"),
    );
    assert!(entry.contains("__rex_app_route_handlers"));
    assert!(entry.contains("/api/data"));
}

#[test]
fn group_entry_registers_pages_and_layouts() {
    let route = AppRoute {
        pattern: "/dashboard".into(),
        page_path: PathBuf::from("/project/app/(app)/dashboard/page.tsx"),
        layout_chain: vec![PathBuf::from("/project/app/(app)/layout.tsx")],
        loading_chain: vec![],
        error_chain: vec![],
        dynamic_segments: vec![],
        specificity: 100,
        route_group: Some("app".into()),
    };
    let entry = generate_group_entry(&[&route]);
    assert!(entry.contains("__layout_mod_0_0"));
    assert!(entry.contains("__page_mod_0"));
    assert!(entry.contains("globalThis.__rex_app_pages[\"/dashboard\"]"));
    assert!(entry.contains("globalThis.__rex_app_layout_chains[\"/dashboard\"]"));
    assert!(entry.contains("globalThis.__rex_app_metadata_sources[\"/dashboard\"]"));
}

#[test]
fn group_entry_multiple_routes() {
    let route1 = AppRoute {
        pattern: "/".into(),
        page_path: PathBuf::from("/app/page.tsx"),
        layout_chain: vec![PathBuf::from("/app/layout.tsx")],
        loading_chain: vec![],
        error_chain: vec![],
        dynamic_segments: vec![],
        specificity: 100,
        route_group: Some("app".into()),
    };
    let route2 = AppRoute {
        pattern: "/about".into(),
        page_path: PathBuf::from("/app/about/page.tsx"),
        layout_chain: vec![PathBuf::from("/app/layout.tsx")],
        loading_chain: vec![],
        error_chain: vec![],
        dynamic_segments: vec![],
        specificity: 100,
        route_group: Some("app".into()),
    };
    let entry = generate_group_entry(&[&route1, &route2]);
    assert!(entry.contains("__page_mod_0"));
    assert!(entry.contains("__page_mod_1"));
    assert!(entry.contains("globalThis.__rex_app_pages[\"/\"]"));
    assert!(entry.contains("globalThis.__rex_app_pages[\"/about\"]"));
}

#[test]
fn group_entry_empty_layout_chain() {
    let route = AppRoute {
        pattern: "/bare".into(),
        page_path: PathBuf::from("/app/bare/page.tsx"),
        layout_chain: vec![],
        loading_chain: vec![],
        error_chain: vec![],
        dynamic_segments: vec![],
        specificity: 100,
        route_group: None,
    };
    let entry = generate_group_entry(&[&route]);
    // No layout imports but page should still register
    assert!(!entry.contains("__layout_mod"));
    assert!(entry.contains("__page_mod_0"));
    assert!(entry.contains("globalThis.__rex_app_pages[\"/bare\"]"));
    // Empty layout chain array
    assert!(entry.contains("globalThis.__rex_app_layout_chains[\"/bare\"] = []"));
}
