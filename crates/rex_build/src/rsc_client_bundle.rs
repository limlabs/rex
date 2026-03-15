//! RSC client bundle builder.
//!
//! Builds ESM client bundles for `"use client"` modules with code splitting.
//! Each client boundary module becomes a separate entry. Shared dependencies
//! (React) become shared chunks via manual code splitting.

use crate::client_manifest::{client_reference_id, ClientReferenceManifest};
use crate::rsc_build_config::{
    build_rex_aliases, react_treeshake_options, sanitize_filename, RscBuildContext,
};
use crate::rsc_stubs::generate_server_action_stub;
use anyhow::Result;
use std::fs;
use std::path::Path;
use tracing::debug;

/// Build client bundles for `"use client"` modules.
///
/// Each client boundary module becomes a separate entry. Rolldown handles
/// code splitting so shared dependencies (React) become shared chunks.
pub(crate) async fn build_rsc_client_bundles(
    ctx: &RscBuildContext<'_>,
    graph: &crate::rsc_graph::ModuleGraph,
    output_dir: &Path,
    client_manifest: &mut ClientReferenceManifest,
    server_action_modules: &[&crate::rsc_graph::ModuleInfo],
) -> Result<Vec<String>> {
    let client_boundaries = graph.client_boundary_modules();
    if client_boundaries.is_empty() && server_action_modules.is_empty() {
        return Ok(vec![]);
    }

    let hash = ctx.hash();

    // Create temporary entry files for rolldown.
    // The hydrate entry uses react-server-dom-webpack/client to parse React's
    // flight format and hydrate the SSR'd HTML with interactive client components.
    let entries_dir = output_dir.join("_rsc_client_entries");
    fs::create_dir_all(&entries_dir)?;

    let hydrate_code = include_str!("../../../runtime/client/rsc-hydrate.ts");
    let hydrate_path = entries_dir.join("__rsc_hydrate.ts");
    fs::write(&hydrate_path, hydrate_code)?;

    // Create entries: hydrate entry + each client boundary module
    let mut entries: Vec<rolldown::InputItem> = vec![rolldown::InputItem {
        name: Some("__rsc_hydrate".to_string()),
        import: hydrate_path.to_string_lossy().to_string(),
    }];

    entries.extend(client_boundaries.iter().map(|m| {
        let rel_path = m
            .path
            .strip_prefix(&ctx.config.project_root)
            .unwrap_or(&m.path);
        let name = sanitize_filename(&rel_path.to_string_lossy());
        rolldown::InputItem {
            name: Some(name),
            import: m.path.to_string_lossy().to_string(),
        }
    }));

    // Rex built-in aliases for client bundle (rex/link → client runtime, etc.)
    let mut client_aliases = build_rex_aliases()?;
    // Manual tsconfig paths (since we disable tsconfig auto-resolution to
    // prevent "jsx": "preserve" from leaving raw JSX in the bundle).
    client_aliases.extend(crate::build_utils::tsconfig_path_aliases(&ctx.project_root));

    // Generate server action stubs for "use server" modules in the client bundle.
    // Each stub replaces the real module with createServerReference calls.
    if !server_action_modules.is_empty() {
        let sa_stubs_dir = entries_dir.join("_server_action_stubs");
        fs::create_dir_all(&sa_stubs_dir)?;

        for module in server_action_modules {
            let rel_path = module
                .path
                .strip_prefix(&ctx.project_root)
                .unwrap_or(&module.path)
                .to_string_lossy()
                .replace('\\', "/");

            let stub_source = generate_server_action_stub(&rel_path, &module.exports, ctx.build_id);
            let stub_name = sanitize_filename(&rel_path);
            let stub_path = sa_stubs_dir.join(format!("{stub_name}.js"));
            fs::write(&stub_path, &stub_source)?;

            // Map original module path → stub for rolldown resolution
            client_aliases.push((
                module.path.to_string_lossy().to_string(),
                vec![Some(stub_path.to_string_lossy().to_string())],
            ));
        }
    }

    // Split React packages into cacheable vendor chunks:
    // 1. react-server-dom-webpack (flight client) — changes rarely, cached independently
    // 2. react + react-dom (core React) — changes rarely, shared across pages
    // Without manual splitting, rolldown inlines everything into __rsc_hydrate
    // because only the hydrate entry imports them (automatic splitting needs 2+ consumers).
    let rsc_flight_group = rolldown_common::MatchGroup {
        name: rolldown_common::MatchGroupName::Static("rsc-flight".to_string()),
        test: Some(rolldown_common::MatchGroupTest::Regex(
            rolldown_utils::js_regex::HybridRegex::new(
                "node_modules[\\\\/]react-server-dom-webpack",
            )
            .expect("valid regex"),
        )),
        priority: Some(20),
        ..Default::default()
    };
    let react_vendor_group = rolldown_common::MatchGroup {
        name: rolldown_common::MatchGroupName::Static("react-vendor".to_string()),
        test: Some(rolldown_common::MatchGroupTest::Regex(
            rolldown_utils::js_regex::HybridRegex::new("node_modules[\\\\/]react")
                .expect("valid regex"),
        )),
        priority: Some(10),
        ..Default::default()
    };

    let options = rolldown::BundlerOptions {
        input: Some(entries),
        cwd: Some(ctx.config.project_root.clone()),
        format: Some(rolldown::OutputFormat::Esm),
        dir: Some(output_dir.to_string_lossy().to_string()),
        entry_filenames: Some(format!("[name]-{hash}.js").into()),
        chunk_filenames: Some(format!("chunk-[name]-{hash}.js").into()),
        platform: Some(rolldown::Platform::Browser),
        module_types: Some(ctx.non_js_empty_module_types()),
        define: Some(ctx.define.iter().cloned().collect()),
        // Disable tsconfig auto-resolution — we manually parse paths into
        // aliases above. This prevents "jsx": "preserve" in tsconfig from
        // leaving raw JSX in the output.
        tsconfig: Some(rolldown_common::TsConfig::Auto(false)),
        minify: ctx.minify_options(),
        treeshake: react_treeshake_options(),
        manual_code_splitting: Some(rolldown_common::ManualCodeSplittingOptions {
            groups: Some(vec![rsc_flight_group, react_vendor_group]),
            ..Default::default()
        }),
        resolve: Some(rolldown::ResolveOptions {
            alias: Some(client_aliases),
            extensions: Some(vec![
                ".tsx".to_string(),
                ".ts".to_string(),
                ".jsx".to_string(),
                ".js".to_string(),
            ]),
            modules: Some(ctx.module_dirs.to_vec()),
            ..Default::default()
        }),
        ..Default::default()
    };

    // Static asset plugin: resolves image imports to URLs and copies files
    let client_asset_dir = output_dir.parent().unwrap_or(output_dir).join("assets");
    let plugins: Vec<std::sync::Arc<dyn rolldown::plugin::Pluginable>> = vec![std::sync::Arc::new(
        crate::static_asset::StaticAssetPlugin::new(client_asset_dir),
    )];

    let mut bundler = rolldown::Bundler::with_plugins(options, plugins)
        .map_err(|e| anyhow::anyhow!("Failed to create RSC client bundler: {e}"))?;

    let output = bundler.write().await.map_err(|e| {
        anyhow::anyhow!(
            "RSC client bundle failed:\n{}",
            crate::diagnostics::format_build_diagnostics(&e)
        )
    })?;

    // Clean up temporary entry files
    let _ = fs::remove_dir_all(&entries_dir);

    // Collect output chunks and update the client manifest.
    // Prefix with "rsc/" because the output dir is client_build_dir/rsc/
    // and the static file server mounts client_build_dir at /_rex/static/.
    //
    // The bootstrap chunk must come first so React/ReactDOM globals are set
    // before component chunks (loaded as type="module") execute.
    let chunks = collect_output_chunks(&output, &client_boundaries, ctx, client_manifest);

    debug!(
        count = chunks.len(),
        "RSC client bundles written to {}",
        output_dir.display()
    );
    Ok(chunks)
}

/// Collect output chunks from rolldown and update the client manifest.
///
/// Separated from the async function for clarity and potential testability.
fn collect_output_chunks(
    output: &rolldown::BundleOutput,
    client_boundaries: &[&crate::rsc_graph::ModuleInfo],
    ctx: &RscBuildContext<'_>,
    client_manifest: &mut ClientReferenceManifest,
) -> Vec<String> {
    let mut bootstrap_chunks = Vec::new();
    let mut component_chunks = Vec::new();

    // Pre-compute boundary lookup: sanitized_name → (rel_path, &exports)
    let boundary_lookup: std::collections::HashMap<String, (String, &[String])> = client_boundaries
        .iter()
        .map(|module| {
            let rel_path = module
                .path
                .strip_prefix(&ctx.config.project_root)
                .unwrap_or(&module.path)
                .to_string_lossy()
                .replace('\\', "/");
            let module_name = sanitize_filename(&rel_path);
            (module_name, (rel_path, module.exports.as_slice()))
        })
        .collect();

    for item in &output.assets {
        match item {
            rolldown_common::Output::Chunk(chunk) => {
                let filename = chunk.filename.to_string();
                let prefixed = format!("rsc/{filename}");

                if chunk.is_entry {
                    let name = chunk.name.to_string();

                    if name == "__rsc_hydrate" || name == "__rsc_bootstrap" {
                        // Hydrate/bootstrap entry — must load first
                        bootstrap_chunks.push(prefixed);
                    } else {
                        // Client component entry — update manifest via O(1) lookup
                        component_chunks.push(prefixed);
                        if let Some((rel_path, exports)) = boundary_lookup.get(&name) {
                            let chunk_url = format!("/_rex/static/rsc/{filename}");
                            for export in *exports {
                                let ref_id = client_reference_id(rel_path, export, ctx.build_id);
                                client_manifest.add(&ref_id, chunk_url.clone(), export.clone());
                            }
                        }
                    }
                } else {
                    // Shared chunks (code-split React, etc.)
                    component_chunks.push(prefixed);
                }
            }
            rolldown_common::Output::Asset(asset) => {
                component_chunks.push(format!("rsc/{}", asset.filename));
            }
        }
    }

    // Bootstrap first, then shared chunks, then component entries
    let mut chunks = bootstrap_chunks;
    chunks.extend(component_chunks);
    chunks
}
