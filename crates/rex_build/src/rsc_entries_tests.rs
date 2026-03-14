use super::*;
use crate::client_manifest::ClientReferenceManifest;
use crate::rsc_graph::{ModuleGraph, ModuleInfo};
use crate::server_action_manifest::{ServerActionEntry, ServerActionManifest};
use rex_core::app_route::{AppRoute, AppScanResult, AppSegment};
use std::collections::HashMap;
use std::path::PathBuf;

fn make_basic_app_scan() -> AppScanResult {
    let layout_path = PathBuf::from("/project/app/layout.tsx");
    let page_path = PathBuf::from("/project/app/page.tsx");
    AppScanResult {
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
    }
}

#[test]
fn server_entry_contains_react_imports() {
    let scan = make_basic_app_scan();
    let manifest = ClientReferenceManifest::new();
    let sa_manifest = ServerActionManifest::new();

    let entry = generate_server_entry(&scan, &manifest, &sa_manifest, Path::new("/project"));

    assert!(entry.contains("import { createElement } from 'react'"));
    assert!(entry.contains("import { renderToReadableStream }"));
}

#[test]
fn server_entry_registers_pages() {
    let scan = make_basic_app_scan();
    let manifest = ClientReferenceManifest::new();
    let sa_manifest = ServerActionManifest::new();

    let entry = generate_server_entry(&scan, &manifest, &sa_manifest, Path::new("/project"));

    assert!(entry.contains("globalThis.__rex_app_pages"));
    assert!(entry.contains("globalThis.__rex_app_pages[\"/\"]"));
}

#[test]
fn server_entry_registers_layout_chains() {
    let scan = make_basic_app_scan();
    let manifest = ClientReferenceManifest::new();
    let sa_manifest = ServerActionManifest::new();

    let entry = generate_server_entry(&scan, &manifest, &sa_manifest, Path::new("/project"));

    assert!(entry.contains("globalThis.__rex_app_layout_chains"));
    assert!(entry.contains("globalThis.__rex_app_layout_chains[\"/\"]"));
}

#[test]
fn server_entry_embeds_webpack_config() {
    let scan = make_basic_app_scan();
    let mut manifest = ClientReferenceManifest::new();
    manifest.add("ref1", "/Counter.js".to_string(), "default".to_string());
    let sa_manifest = ServerActionManifest::new();

    let entry = generate_server_entry(&scan, &manifest, &sa_manifest, Path::new("/project"));

    assert!(entry.contains("__rex_webpack_bundler_config"));
    assert!(entry.contains("ref1"));
}

#[test]
fn server_entry_includes_flight_runtime() {
    let scan = make_basic_app_scan();
    let manifest = ClientReferenceManifest::new();
    let sa_manifest = ServerActionManifest::new();

    let entry = generate_server_entry(&scan, &manifest, &sa_manifest, Path::new("/project"));

    assert!(entry.contains("// --- RSC Flight Runtime ---"));
}

#[test]
fn server_entry_includes_metadata_runtime() {
    let scan = make_basic_app_scan();
    let manifest = ClientReferenceManifest::new();
    let sa_manifest = ServerActionManifest::new();

    let entry = generate_server_entry(&scan, &manifest, &sa_manifest, Path::new("/project"));

    assert!(entry.contains("// --- Metadata Runtime ---"));
    assert!(entry.contains("metadataToHtml"));
}

#[test]
fn server_entry_registers_metadata_sources() {
    let scan = make_basic_app_scan();
    let manifest = ClientReferenceManifest::new();
    let sa_manifest = ServerActionManifest::new();

    let entry = generate_server_entry(&scan, &manifest, &sa_manifest, Path::new("/project"));

    assert!(entry.contains("globalThis.__rex_app_metadata_sources"));
    assert!(entry.contains("globalThis.__rex_app_metadata_sources[\"/\"]"));
    // Should contain both the layout module and the page module
    assert!(entry.contains("__layout_mod_0_0"));
    assert!(entry.contains("__page_mod_0"));
}

#[test]
fn server_entry_with_server_actions() {
    let scan = make_basic_app_scan();
    let manifest = ClientReferenceManifest::new();
    let mut sa_manifest = ServerActionManifest::new();
    sa_manifest.actions.insert(
        "action_123".to_string(),
        ServerActionEntry {
            module_path: "app/actions.ts".to_string(),
            export_name: "increment".to_string(),
        },
    );

    let entry = generate_server_entry(&scan, &manifest, &sa_manifest, Path::new("/project"));

    assert!(entry.contains("registerServerReference"));
    assert!(entry.contains("globalThis.__rex_server_actions"));
    assert!(entry.contains("action_123"));
    assert!(entry.contains("globalThis.__rex_decodeReply = decodeReply"));
    assert!(entry.contains("globalThis.__rex_decodeAction = decodeAction"));
    assert!(
        entry.contains("globalThis.__rex_server_action_manifest = globalThis.__rex_server_actions")
    );
}

#[test]
fn server_entry_without_server_actions_omits_registration() {
    let scan = make_basic_app_scan();
    let manifest = ClientReferenceManifest::new();
    let sa_manifest = ServerActionManifest::new();

    let entry = generate_server_entry(&scan, &manifest, &sa_manifest, Path::new("/project"));

    // The registration block should not appear (the flight runtime itself
    // references __rex_server_actions for dispatch, so only check the
    // registration marker).
    assert!(!entry.contains("// --- Server Actions Registration ---"));
    assert!(!entry.contains("registerServerReference"));
}

#[test]
fn server_entry_multiple_routes() {
    let layout_path = PathBuf::from("/project/app/layout.tsx");
    let page1 = PathBuf::from("/project/app/page.tsx");
    let page2 = PathBuf::from("/project/app/about/page.tsx");
    let scan = AppScanResult {
        root: AppSegment {
            segment: String::new(),
            layout: Some(layout_path.clone()),
            page: Some(page1.clone()),
            route: None,
            loading: None,
            error_boundary: None,
            not_found: None,
            children: vec![],
        },
        routes: vec![
            AppRoute {
                pattern: "/".to_string(),
                page_path: page1,
                layout_chain: vec![layout_path.clone()],
                loading_chain: vec![None],
                error_chain: vec![None],
                dynamic_segments: vec![],
                specificity: 10,
                route_group: None,
            },
            AppRoute {
                pattern: "/about".to_string(),
                page_path: page2,
                layout_chain: vec![layout_path.clone()],
                loading_chain: vec![None],
                error_chain: vec![None],
                dynamic_segments: vec![],
                specificity: 10,
                route_group: None,
            },
        ],
        api_routes: vec![],
        root_layout: Some(layout_path),
    };

    let manifest = ClientReferenceManifest::new();
    let sa_manifest = ServerActionManifest::new();
    let entry = generate_server_entry(&scan, &manifest, &sa_manifest, Path::new("/project"));

    assert!(entry.contains("globalThis.__rex_app_pages[\"/\"]"));
    assert!(entry.contains("globalThis.__rex_app_pages[\"/about\"]"));
}

fn make_graph_with_client_boundary() -> ModuleGraph {
    let mut modules = HashMap::new();
    modules.insert(
        PathBuf::from("/project/components/Counter.tsx"),
        ModuleInfo {
            path: PathBuf::from("/project/components/Counter.tsx"),
            is_client: true,
            is_server: false,
            uses_dynamic_functions: false,
            imports: vec![],
            exports: vec!["default".to_string()],
            server_functions: vec![],
            has_unextracted_server_directives: false,
        },
    );
    ModuleGraph { modules }
}

#[test]
fn ssr_entry_contains_react_imports() {
    let graph = ModuleGraph::default();
    let manifest = ClientReferenceManifest::new();
    let sa_manifest = ServerActionManifest::new();
    let entry = generate_ssr_entry(
        &graph,
        &manifest,
        &sa_manifest,
        Path::new("/project"),
        "build1",
    );

    assert!(entry.contains("import { createElement } from 'react'"));
    assert!(entry.contains("import { renderToReadableStream } from 'react-dom/server'"));
    assert!(entry.contains("import { createFromReadableStream }"));
}

#[test]
fn ssr_entry_imports_client_components() {
    let graph = make_graph_with_client_boundary();
    let manifest = ClientReferenceManifest::new();
    let sa_manifest = ServerActionManifest::new();
    let entry = generate_ssr_entry(
        &graph,
        &manifest,
        &sa_manifest,
        Path::new("/project"),
        "build1",
    );

    assert!(entry.contains("import * as __ssr_client_0"));
    assert!(entry.contains("Counter.tsx"));
}

#[test]
fn ssr_entry_registers_modules() {
    let graph = make_graph_with_client_boundary();
    let manifest = ClientReferenceManifest::new();
    let sa_manifest = ServerActionManifest::new();
    let entry = generate_ssr_entry(
        &graph,
        &manifest,
        &sa_manifest,
        Path::new("/project"),
        "build1",
    );

    assert!(entry.contains("globalThis.__rex_ssr_modules__"));
}

#[test]
fn ssr_entry_embeds_webpack_manifest() {
    let graph = ModuleGraph::default();
    let mut manifest = ClientReferenceManifest::new();
    manifest.add("ref1", "/chunk.js".to_string(), "default".to_string());
    let sa_manifest = ServerActionManifest::new();
    let entry = generate_ssr_entry(
        &graph,
        &manifest,
        &sa_manifest,
        Path::new("/project"),
        "build1",
    );

    assert!(entry.contains("__rex_webpack_ssr_manifest"));
    assert!(entry.contains("ref1"));
}

#[test]
fn ssr_entry_includes_runtime() {
    let graph = ModuleGraph::default();
    let manifest = ClientReferenceManifest::new();
    let sa_manifest = ServerActionManifest::new();
    let entry = generate_ssr_entry(
        &graph,
        &manifest,
        &sa_manifest,
        Path::new("/project"),
        "build1",
    );

    assert!(entry.contains("// --- RSC SSR Pass Runtime ---"));
}

#[test]
fn ssr_entry_with_server_actions_generates_server_module_map() {
    let graph = ModuleGraph::default();
    let manifest = ClientReferenceManifest::new();
    let mut sa_manifest = ServerActionManifest::new();
    sa_manifest.actions.insert(
        "sa_abc".to_string(),
        ServerActionEntry {
            module_path: "app/actions.ts".to_string(),
            export_name: "increment".to_string(),
        },
    );
    let entry = generate_ssr_entry(
        &graph,
        &manifest,
        &sa_manifest,
        Path::new("/project"),
        "build1",
    );

    assert!(entry.contains("__rex_webpack_server_module_map"));
    assert!(entry.contains("sa_abc"));
    assert!(entry.contains("increment"));
    // Should register stub function in __rex_ssr_modules__
    assert!(entry.contains("globalThis.__rex_ssr_modules__[\"sa_abc\"]"));
}
