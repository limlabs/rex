//! RSC bundle builder — orchestrates flight, SSR, and client bundle builds.
//!
//! In dev mode (`skip_server_iife`), only the client bundle is built;
//! server/SSR rendering uses V8 native ESM modules instead of IIFE bundles.

use crate::client_manifest::ClientReferenceManifest;
use crate::precomputed_ids::PrecomputedIds;
use crate::rsc_build_config::{sanitize_filename, RscBuildContext};
use crate::rsc_client_bundle::build_rsc_client_bundles;
use crate::rsc_graph::analyze_module_graph;
use crate::rsc_server_bundle::build_rsc_server_bundle;
use crate::rsc_ssr_bundle::build_rsc_ssr_bundle;
use crate::rsc_stubs::generate_client_stub;
use crate::server_action_manifest::ServerActionManifest;
use anyhow::Result;
use rex_core::app_route::AppScanResult;
use rex_core::RexConfig;
use std::fs;
use std::path::PathBuf;

/// Result of the RSC bundle build.
#[derive(Debug)]
pub struct RscBuildResult {
    /// Path to the server RSC flight bundle (IIFE, `react-server` condition).
    /// `None` when `skip_server_iife` is set (ESM dev mode).
    pub server_bundle_path: Option<PathBuf>,
    /// Path to the SSR bundle (IIFE, standard conditions).
    /// `None` when `skip_server_iife` is set (ESM dev mode).
    pub ssr_bundle_path: Option<PathBuf>,
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
/// When `skip_server_iife` is true, server/SSR IIFE builds are skipped (ESM replaces them).
/// When `precomputed` is provided, IDs are looked up from the ESM walk instead of computed.
pub async fn build_rsc_bundles(
    config: &RexConfig,
    app_scan: &AppScanResult,
    build_id: &str,
    define: &[(String, String)],
    skip_server_iife: bool,
    precomputed: Option<&PrecomputedIds>,
) -> Result<RscBuildResult> {
    let server_dir = config.server_build_dir().join("rsc");
    let client_dir = config.client_build_dir().join("rsc");
    fs::create_dir_all(&server_dir)?;
    fs::create_dir_all(&client_dir)?;

    let module_dirs = crate::bundler::resolve_modules_dirs(config)?;
    let ctx = RscBuildContext::new(config, build_id, define, &module_dirs, precomputed);

    // Collect all entry points from the app scan
    let mut entries: Vec<PathBuf> = Vec::new();
    if let Some(root_layout) = &app_scan.root_layout {
        entries.push(root_layout.clone());
    }
    for route in &app_scan.routes {
        entries.push(route.page_path.clone());
        entries.extend(route.layout_chain.iter().cloned());
    }
    // MDX pages are compiled to .jsx before the module graph walk, so the walker
    // never sees `import { useMDXComponents }`. Add it as an explicit entry so
    // its "use client" imports get proper client reference stubs.
    if let Some(mdx_components) = rex_mdx::find_mdx_components(&config.project_root) {
        let mdx_path = PathBuf::from(&mdx_components);
        if mdx_path.exists() {
            entries.push(mdx_path);
        }
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

        // Generate stub file with client reference objects.
        // Use pre-computed IDs from ESM walk when available.
        let stub_source =
            generate_client_stub(&rel_path, &module.exports, build_id, ctx.precomputed_ids);
        let stub_name = sanitize_filename(&rel_path);
        let stub_path = stubs_dir.join(format!("{stub_name}.js"));
        fs::write(&stub_path, &stub_source)?;

        // Map original module path → stub path for rolldown aliases
        stub_aliases.push((module.path.clone(), stub_path));

        // Register in manifest (chunk URLs filled in after client build)
        for export in &module.exports {
            let ref_id = ctx.resolve_client_ref_id(&rel_path, export);
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
            let action_id = ctx.resolve_server_action_id(&rel_path, export);
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
            let action_id = ctx.resolve_server_action_id(&rel_path, export);
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
            let action_id = ctx.resolve_server_action_id(&rel_path, &action.name);
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

    // Build client bundles (always needed — browser requires them for hydration)
    let client_chunks = build_rsc_client_bundles(
        &ctx,
        &graph,
        &client_dir,
        &mut client_manifest,
        &server_action_modules,
    )
    .await?;

    // In ESM dev mode, skip the server flight IIFE (ESM replaces it).
    // The SSR bundle is ALWAYS built — it provides __rex_rsc_flight_to_html
    // for converting flight data to server-rendered HTML.
    let server_bundle_path = if skip_server_iife {
        None
    } else {
        Some(
            build_rsc_server_bundle(
                &ctx,
                app_scan,
                &graph,
                &server_dir,
                &stub_aliases,
                &client_manifest,
                &server_action_manifest,
                &inline_action_targets,
            )
            .await?,
        )
    };

    let ssr_bundle_path = Some(
        build_rsc_ssr_bundle(
            &ctx,
            &graph,
            &server_dir,
            &client_manifest,
            &server_action_manifest,
        )
        .await?,
    );

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

// Tests moved to crates/rex_build/tests/rsc_bundler_tests.rs
