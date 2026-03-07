//! RSC bundle builder — orchestrator.
//!
//! Produces three bundles from an app/ directory scan:
//! 1. **Flight bundle** (IIFE, `react-server` condition): Contains all server components.
//!    At `"use client"` boundaries, imports are replaced with client reference stubs.
//!    Uses `renderToReadableStream` from `react-server-dom-webpack/server`.
//! 2. **SSR bundle** (IIFE, standard conditions): Contains `createFromReadableStream`
//!    and `renderToString` for converting flight data to HTML. Also includes client
//!    components for SSR rendering.
//! 3. **Client bundle** (ESM): Contains only `"use client"` components and their
//!    dependencies, with code splitting.
//!
//! Also produces a `ClientReferenceManifest` mapping reference IDs to chunk URLs.
//!
//! Implementation is split across focused modules:
//! - [`rsc_build_config`]: Shared context, aliases, treeshake options
//! - [`rsc_entries`]: Pure entry code generation (no I/O)
//! - [`rsc_stubs`]: Client reference and server action stub generators
//! - [`rsc_server_bundle`]: Server flight bundle builder
//! - [`rsc_ssr_bundle`]: SSR bundle builder
//! - [`rsc_client_bundle`]: Client bundle builder + manifest wiring

use crate::client_manifest::{client_reference_id, ClientReferenceManifest};
use crate::rsc_build_config::{sanitize_filename, RscBuildContext};
use crate::rsc_client_bundle::build_rsc_client_bundles;
use crate::rsc_graph::analyze_module_graph;
use crate::rsc_server_bundle::build_rsc_server_bundle;
use crate::rsc_ssr_bundle::build_rsc_ssr_bundle;
use crate::rsc_stubs::generate_client_stub;
use crate::server_action_manifest::{server_action_id, ServerActionManifest};
use anyhow::Result;
use rex_core::app_route::AppScanResult;
use rex_core::RexConfig;
use std::fs;
use std::path::PathBuf;

/// Result of the RSC bundle build.
#[derive(Debug)]
pub struct RscBuildResult {
    /// Path to the server RSC flight bundle (IIFE, `react-server` condition).
    pub server_bundle_path: PathBuf,
    /// Path to the SSR bundle (IIFE, standard conditions).
    pub ssr_bundle_path: PathBuf,
    /// Client reference manifest mapping ref IDs to chunk URLs.
    pub client_manifest: ClientReferenceManifest,
    /// Client chunk files produced (relative paths from client output dir).
    pub client_chunks: Vec<String>,
    /// Server action manifest mapping action IDs to their module/export.
    pub server_action_manifest: ServerActionManifest,
}

/// Build RSC bundles for an app/ directory.
///
/// This is called from `build_bundles` when an `AppScanResult` is present.
pub async fn build_rsc_bundles(
    config: &RexConfig,
    app_scan: &AppScanResult,
    build_id: &str,
    define: &[(String, String)],
) -> Result<RscBuildResult> {
    let server_dir = config.server_build_dir().join("rsc");
    let client_dir = config.client_build_dir().join("rsc");
    fs::create_dir_all(&server_dir)?;
    fs::create_dir_all(&client_dir)?;

    let module_dirs = crate::bundler::resolve_modules_dirs(config)?;
    let ctx = RscBuildContext::new(config, build_id, define, &module_dirs);

    // Collect all entry points from the app scan
    let mut entries: Vec<PathBuf> = Vec::new();
    entries.push(app_scan.root_layout.clone());
    for route in &app_scan.routes {
        entries.push(route.page_path.clone());
        entries.extend(route.layout_chain.iter().cloned());
    }
    entries.sort();
    entries.dedup();

    // Analyze the module graph
    let graph = analyze_module_graph(&entries, &config.project_root)?;

    // Generate client reference stubs for "use client" modules
    let stubs_dir = server_dir.join("_client_stubs");
    fs::create_dir_all(&stubs_dir)?;

    let client_boundaries = graph.client_boundary_modules();
    let mut stub_aliases: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut client_manifest = ClientReferenceManifest::new();

    for module in &client_boundaries {
        let rel_path = module
            .path
            .strip_prefix(&ctx.project_root)
            .unwrap_or(&module.path)
            .to_string_lossy()
            .replace('\\', "/");

        // Generate stub file with client reference objects
        let stub_source = generate_client_stub(&rel_path, &module.exports, build_id);
        let stub_name = sanitize_filename(&rel_path);
        let stub_path = stubs_dir.join(format!("{stub_name}.js"));
        fs::write(&stub_path, &stub_source)?;

        // Map original module path → stub path for rolldown aliases
        stub_aliases.push((module.path.clone(), stub_path));

        // Register in manifest (chunk URLs filled in after client build)
        for export in &module.exports {
            let ref_id = client_reference_id(&rel_path, export, build_id);
            // Placeholder chunk URL — updated after client bundle build
            client_manifest.add(&ref_id, String::new(), export.clone());
        }
    }

    // Build server action manifest from "use server" modules
    let server_action_modules = graph.server_action_modules();
    let mut server_action_manifest = ServerActionManifest::new();
    for module in &server_action_modules {
        let rel_path = module
            .path
            .strip_prefix(&ctx.project_root)
            .unwrap_or(&module.path)
            .to_string_lossy()
            .replace('\\', "/");
        for export in &module.exports {
            let action_id = server_action_id(&rel_path, export, build_id);
            server_action_manifest.add(&action_id, rel_path.clone(), export.clone());
        }
    }

    // Also register function-level "use server" exports
    let inline_action_modules = graph.inline_server_action_modules();
    for module in &inline_action_modules {
        let rel_path = module
            .path
            .strip_prefix(&ctx.project_root)
            .unwrap_or(&module.path)
            .to_string_lossy()
            .replace('\\', "/");
        for export in &module.server_functions {
            let action_id = server_action_id(&rel_path, export, build_id);
            server_action_manifest.add(&action_id, rel_path.clone(), export.clone());
        }
    }

    // Build rex/* → stub aliases for client boundaries discovered via rex/* imports.
    // The stub_aliases map absolute paths, but rolldown also needs the specifier alias
    // (e.g. "rex/link" → stub) for when source code uses `import Link from 'rex/link'`.
    let pkg_src = ctx.project_root.join("node_modules/@limlabs/rex/src");
    let rex_client_specifiers = ["link", "head", "router", "image"];
    for name in &rex_client_specifiers {
        let specifier = format!("rex/{name}");
        for ext in &["tsx", "ts", "jsx", "js"] {
            let candidate = pkg_src.join(format!("{name}.{ext}"));
            if candidate.exists() {
                if let Ok(canonical) = candidate.canonicalize() {
                    // If this file is a client boundary (has a stub), add specifier → stub alias
                    if let Some((_orig, stub)) = stub_aliases.iter().find(|(p, _)| *p == canonical)
                    {
                        stub_aliases.push((PathBuf::from(&specifier), stub.clone()));
                    }
                    break;
                }
            }
        }
    }

    // Build client bundles first so manifest is populated before server bundle
    let client_chunks = build_rsc_client_bundles(
        &ctx,
        &graph,
        &client_dir,
        &mut client_manifest,
        &server_action_modules,
    )
    .await?;

    // Build server RSC flight bundle (after client build so manifest is populated)
    let server_bundle_path = build_rsc_server_bundle(
        &ctx,
        app_scan,
        &graph,
        &server_dir,
        &stub_aliases,
        &client_manifest,
        &server_action_manifest,
    )
    .await?;

    // Build SSR bundle (after client build so manifest is populated)
    let ssr_bundle_path = build_rsc_ssr_bundle(&ctx, &graph, &server_dir, &client_manifest).await?;

    // Clean up stubs
    let _ = fs::remove_dir_all(&stubs_dir);

    Ok(RscBuildResult {
        server_bundle_path,
        ssr_bundle_path,
        client_manifest,
        client_chunks,
        server_action_manifest,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rex_core::app_route::{AppRoute, AppScanResult, AppSegment};
    use std::path::Path;

    fn setup_rsc_mock_node_modules(root: &Path) {
        let nm = root.join("node_modules");

        // react
        let react_dir = nm.join("react");
        fs::create_dir_all(&react_dir).unwrap();
        fs::write(
            react_dir.join("package.json"),
            r#"{"name":"react","version":"19.0.0","main":"index.js"}"#,
        )
        .unwrap();
        fs::write(
            react_dir.join("index.js"),
            "export function createElement(type, props, ...children) { return { type, props, children }; }\nexport default { createElement };\n",
        )
        .unwrap();
        fs::write(
            react_dir.join("jsx-runtime.js"),
            "export function jsx(type, props) { return { type, props }; }\nexport function jsxs(type, props) { return { type, props }; }\nexport const Fragment = 'Fragment';\n",
        )
        .unwrap();
        fs::write(
            react_dir.join("jsx-dev-runtime.js"),
            "export function jsxDEV(type, props) { return { type, props }; }\nexport const Fragment = 'Fragment';\n",
        )
        .unwrap();

        // react-dom
        let react_dom_dir = nm.join("react-dom");
        fs::create_dir_all(&react_dom_dir).unwrap();
        fs::write(
            react_dom_dir.join("package.json"),
            r#"{"name":"react-dom","version":"19.0.0","main":"index.js","exports":{".":{"default":"./index.js"},"./client":{"default":"./client.js"},"./server":{"default":"./server.js"}}}"#,
        )
        .unwrap();
        fs::write(react_dom_dir.join("index.js"), "export default {};\n").unwrap();
        fs::write(
            react_dom_dir.join("client.js"),
            "export function hydrateRoot() {}\nexport function createRoot() {}\n",
        )
        .unwrap();
        fs::write(
            react_dom_dir.join("server.js"),
            "export function renderToString(el) { return '<div></div>'; }\n",
        )
        .unwrap();

        // react-server-dom-webpack
        let rsdw_dir = nm.join("react-server-dom-webpack");
        fs::create_dir_all(&rsdw_dir).unwrap();
        fs::write(
            rsdw_dir.join("package.json"),
            r#"{"name":"react-server-dom-webpack","version":"19.0.0","main":"index.js","exports":{".":{"default":"./index.js"},"./client":{"default":"./client.js"},"./server":{"default":"./server.js"}}}"#,
        )
        .unwrap();
        fs::write(rsdw_dir.join("index.js"), "export default {};\n").unwrap();
        fs::write(
            rsdw_dir.join("client.js"),
            "export function createFromReadableStream(s) { return {}; }\nexport function createServerReference(id, callServer) { return function(...args) { return callServer(id, args); }; }\nexport function encodeReply(value) { return Promise.resolve(JSON.stringify(value)); }\n",
        )
        .unwrap();
        fs::write(
            rsdw_dir.join("server.js"),
            "export function renderToReadableStream(el, config) { return new ReadableStream(); }\nexport function registerServerReference(fn, id, name) { return fn; }\nexport function decodeReply(body, manifest) { if (typeof body === 'string') { return Promise.resolve(JSON.parse(body)); } return Promise.resolve([]); }\nexport function decodeAction(body, manifest) { return Promise.resolve(null); }\n",
        )
        .unwrap();
    }

    #[tokio::test]
    async fn test_rsc_build_produces_bundles() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();

        setup_rsc_mock_node_modules(&root);

        // Create app directory with layout + page
        let app_dir = root.join("app");
        fs::create_dir_all(&app_dir).unwrap();

        let layout_path = app_dir.join("layout.tsx");
        fs::write(
            &layout_path,
            "export default function RootLayout({ children }) { return children; }\n",
        )
        .unwrap();

        let page_path = app_dir.join("page.tsx");
        fs::write(
            &page_path,
            "export default function Home() { return 'Hello'; }\n",
        )
        .unwrap();

        // Create a "use client" component
        let comp_dir = root.join("components");
        fs::create_dir_all(&comp_dir).unwrap();
        let counter_path = comp_dir.join("Counter.tsx");
        fs::write(
            &counter_path,
            "\"use client\";\nexport default function Counter() { return 'count'; }\n",
        )
        .unwrap();

        let config = rex_core::RexConfig::new(root.clone()).with_dev(true);

        let app_scan = AppScanResult {
            root: AppSegment {
                segment: String::new(),
                layout: Some(layout_path.clone()),
                page: Some(page_path.clone()),
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
            }],
            root_layout: layout_path,
        };

        let define = vec![(
            "process.env.NODE_ENV".to_string(),
            "\"development\"".to_string(),
        )];

        let result = build_rsc_bundles(&config, &app_scan, "test-build-id", &define)
            .await
            .expect("build_rsc_bundles should succeed");

        // Server bundle file exists
        assert!(
            result.server_bundle_path.exists(),
            "Server bundle should exist at {:?}",
            result.server_bundle_path
        );

        // Server bundle is non-empty
        let server_content = fs::read_to_string(&result.server_bundle_path).unwrap();
        assert!(
            !server_content.is_empty(),
            "Server bundle should not be empty"
        );

        // SSR bundle file exists
        assert!(
            result.ssr_bundle_path.exists(),
            "SSR bundle should exist at {:?}",
            result.ssr_bundle_path
        );

        // SSR bundle is non-empty
        let ssr_content = fs::read_to_string(&result.ssr_bundle_path).unwrap();
        assert!(!ssr_content.is_empty(), "SSR bundle should not be empty");

        // Client manifest was created (may be empty if no "use client" modules in entries)
        // Verify the manifest struct exists and is accessible
        let _ = &result.client_manifest.entries;
    }

    #[tokio::test]
    async fn test_rsc_build_with_server_actions() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();

        setup_rsc_mock_node_modules(&root);

        let app_dir = root.join("app");
        fs::create_dir_all(&app_dir).unwrap();

        let layout_path = app_dir.join("layout.tsx");
        fs::write(
            &layout_path,
            "export default function RootLayout({ children }) { return children; }\n",
        )
        .unwrap();

        // Page that imports from a "use server" module
        let page_path = app_dir.join("page.tsx");
        fs::write(
            &page_path,
            "import { increment } from './actions';\nexport default function Home() { return 'Hello'; }\n",
        )
        .unwrap();

        // "use server" module
        let actions_path = app_dir.join("actions.ts");
        fs::write(
            &actions_path,
            "\"use server\";\nexport async function increment(n: number) { return n + 1; }\nexport async function decrement(n: number) { return n - 1; }\n",
        )
        .unwrap();

        let config = rex_core::RexConfig::new(root.clone()).with_dev(true);

        let app_scan = AppScanResult {
            root: AppSegment {
                segment: String::new(),
                layout: Some(layout_path.clone()),
                page: Some(page_path.clone()),
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
            }],
            root_layout: layout_path,
        };

        let define = vec![(
            "process.env.NODE_ENV".to_string(),
            "\"development\"".to_string(),
        )];

        let result = build_rsc_bundles(&config, &app_scan, "test-sa-build", &define)
            .await
            .expect("build_rsc_bundles should succeed");

        // Server action manifest should have 2 actions
        assert_eq!(
            result.server_action_manifest.actions.len(),
            2,
            "Should have 2 server actions (increment + decrement)"
        );

        // Verify actions are in the manifest
        let has_increment = result
            .server_action_manifest
            .actions
            .values()
            .any(|a| a.export_name == "increment");
        assert!(has_increment, "Manifest should contain increment action");

        let has_decrement = result
            .server_action_manifest
            .actions
            .values()
            .any(|a| a.export_name == "decrement");
        assert!(has_decrement, "Manifest should contain decrement action");

        // Server bundle should contain server action dispatch code
        let server_content = fs::read_to_string(&result.server_bundle_path).unwrap();
        assert!(
            server_content.contains("__rex_server_actions"),
            "Server bundle should contain action dispatch table"
        );
        assert!(
            server_content.contains("__rex_call_server_action"),
            "Server bundle should contain action call function"
        );
    }

    #[tokio::test]
    async fn test_client_bundle_uses_stubs_for_server_actions_imported_by_client_component() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();

        setup_rsc_mock_node_modules(&root);

        let app_dir = root.join("app");
        fs::create_dir_all(&app_dir).unwrap();

        let layout_path = app_dir.join("layout.tsx");
        fs::write(
            &layout_path,
            "export default function RootLayout({ children }) { return children; }\n",
        )
        .unwrap();

        // Server page imports a client component (not the actions directly)
        let page_path = app_dir.join("page.tsx");
        fs::write(
            &page_path,
            "import ActionCounter from '../components/ActionCounter';\nexport default function Home() { return 'Hello'; }\n",
        )
        .unwrap();

        // "use client" component imports from a "use server" module
        let comp_dir = root.join("components");
        fs::create_dir_all(&comp_dir).unwrap();
        fs::write(
            comp_dir.join("ActionCounter.tsx"),
            "\"use client\";\nimport { increment } from '../app/actions';\nexport default function ActionCounter() { return 'count: ' + increment(0); }\n",
        )
        .unwrap();

        // "use server" module
        fs::write(
            app_dir.join("actions.ts"),
            "\"use server\";\nexport async function increment(n: number) { return n + 1; }\n",
        )
        .unwrap();

        let config = rex_core::RexConfig::new(root.clone()).with_dev(true);

        let app_scan = AppScanResult {
            root: AppSegment {
                segment: String::new(),
                layout: Some(layout_path.clone()),
                page: Some(page_path.clone()),
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
            }],
            root_layout: layout_path,
        };

        let define = vec![(
            "process.env.NODE_ENV".to_string(),
            "\"development\"".to_string(),
        )];

        let result = build_rsc_bundles(&config, &app_scan, "test-sa-client", &define)
            .await
            .expect("build_rsc_bundles should succeed");

        // Server action manifest should have the increment action
        assert_eq!(
            result.server_action_manifest.actions.len(),
            1,
            "Should have 1 server action (increment)"
        );

        // Find the client bundle for ActionCounter
        let client_dir = root.join(".rex/build/client/rsc");
        let action_counter_chunk = fs::read_dir(&client_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| e.file_name().to_string_lossy().contains("ActionCounter"))
            .expect("ActionCounter client chunk should exist");

        let client_content = fs::read_to_string(action_counter_chunk.path()).unwrap();

        // The client bundle must use createServerReference, NOT inline the function body
        assert!(
            client_content.contains("createServerReference"),
            "Client bundle should use createServerReference proxy, not inline the function. Got: {client_content}"
        );
        assert!(
            !client_content.contains("return n + 1"),
            "Client bundle should NOT contain the server action implementation. Got: {client_content}"
        );
    }

    #[tokio::test]
    async fn test_rsc_build_no_client_boundaries() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();

        setup_rsc_mock_node_modules(&root);

        let app_dir = root.join("app");
        fs::create_dir_all(&app_dir).unwrap();

        let layout_path = app_dir.join("layout.tsx");
        fs::write(
            &layout_path,
            "export default function RootLayout({ children }) { return children; }\n",
        )
        .unwrap();

        let page_path = app_dir.join("page.tsx");
        fs::write(
            &page_path,
            "export default function Home() { return 'Hello server only'; }\n",
        )
        .unwrap();

        let config = rex_core::RexConfig::new(root.clone()).with_dev(true);

        let app_scan = AppScanResult {
            root: AppSegment {
                segment: String::new(),
                layout: Some(layout_path.clone()),
                page: Some(page_path.clone()),
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
            }],
            root_layout: layout_path,
        };

        let define = vec![(
            "process.env.NODE_ENV".to_string(),
            "\"development\"".to_string(),
        )];

        let result = build_rsc_bundles(&config, &app_scan, "test-no-client", &define)
            .await
            .expect("build_rsc_bundles should succeed");

        // No client boundaries → empty manifest entries (only placeholder entries if any)
        assert!(
            result.client_manifest.entries.is_empty(),
            "Manifest should be empty with no client boundaries"
        );

        // Server action manifest should be empty
        assert!(
            result.server_action_manifest.actions.is_empty(),
            "No server actions expected"
        );
    }
}
