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
    /// Module graph for detecting dynamic function usage per route.
    pub module_graph: crate::rsc_graph::ModuleGraph,
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
    if let Some(root_layout) = &app_scan.root_layout {
        entries.push(root_layout.clone());
    }
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

    // Extract inline "use server" functions from JSX. Register extracted
    // actions in the manifest; the InlineServerActionPlugin handles source
    // transformation at bundle time (preserving relative import resolution).
    let mut inline_action_targets: Vec<PathBuf> = Vec::new();
    for module in graph.unextracted_server_action_modules() {
        let Ok(source) = std::fs::read_to_string(&module.path) else {
            continue;
        };
        let Some(result) =
            crate::server_action_extract::extract_inline_server_actions(&source, &module.path)
        else {
            continue;
        };

        inline_action_targets.push(module.path.clone());
        let rel_path = module
            .path
            .strip_prefix(&config.project_root)
            .unwrap_or(&module.path)
            .to_string_lossy()
            .replace('\\', "/");
        for action in &result.actions {
            let action_id =
                crate::server_action_manifest::server_action_id(&rel_path, &action.name, build_id);
            server_action_manifest.add(&action_id, rel_path.clone(), action.name.clone());
        }
        tracing::debug!(file = %rel_path, count = result.actions.len(), "Extracted inline server actions");
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
        &inline_action_targets,
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
        module_graph: graph,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rex_core::app_route::{AppRoute, AppScanResult, AppSegment};
    use std::path::Path;

    /// Assert that an IIFE bundle has no unresolved external parameters.
    ///
    /// Rolldown wraps IIFE bundles as `(function(ext) { ... })(ext)` when it treats
    /// an import as external. The trailing `)(ext);` will fail at V8 eval time because
    /// `ext` is not a global. This check catches missing resolve aliases at build time.
    fn assert_no_iife_externals(bundle_js: &str, label: &str) {
        // The IIFE footer pattern: `})(some_identifier);` at the end of the bundle.
        // A self-contained IIFE ends with `})();\n` (no arguments).
        let trimmed = bundle_js.trim_end();
        if let Some(tail) = trimmed.strip_suffix(';') {
            if let Some(before_paren) = tail.strip_suffix(')') {
                // Find the matching '(' for the IIFE call arguments
                if let Some(args_start) = before_paren.rfind(")(") {
                    let args = &before_paren[args_start + 2..];
                    assert!(
                        args.is_empty(),
                        "{label} has unresolved IIFE externals: `({args})` — \
                         this means rolldown treated an import as external. \
                         Add a resolve alias for the missing module."
                    );
                }
            }
        }
    }

    /// Symlink the real node_modules from fixtures/app-router into a test directory.
    fn setup_rsc_node_modules(root: &Path) {
        let fixture_nm =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/app-router/node_modules");
        assert!(
            fixture_nm.exists(),
            "fixtures/app-router/node_modules not found — run `cd fixtures/app-router && npm install`"
        );
        std::os::unix::fs::symlink(
            fixture_nm.canonicalize().unwrap(),
            root.join("node_modules"),
        )
        .unwrap();
        // package.json is needed so the build doesn't try to use built-in modules
        fs::write(
            root.join("package.json"),
            r#"{"name":"rsc-test","private":true}"#,
        )
        .unwrap();
    }

    #[tokio::test]
    async fn test_rsc_build_produces_bundles() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();

        setup_rsc_node_modules(&root);

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

        // Verify bundle has no unresolved externals in IIFE wrapper.
        // Unresolved imports produce `(function(some_external) { ... })(some_external);`
        // which fails at runtime since `some_external` is undefined.
        assert_no_iife_externals(&server_content, "RSC server bundle");
        assert_no_iife_externals(&ssr_content, "RSC SSR bundle");
    }

    #[tokio::test]
    async fn test_rsc_build_with_server_actions() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();

        setup_rsc_node_modules(&root);

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

        // Verify bundle has no unresolved externals in IIFE wrapper
        assert_no_iife_externals(&server_content, "RSC server bundle");
    }

    #[tokio::test]
    async fn test_client_bundle_uses_stubs_for_server_actions_imported_by_client_component() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();

        setup_rsc_node_modules(&root);

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

        setup_rsc_node_modules(&root);

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
