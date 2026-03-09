//! RSC server flight bundle builder.
//!
//! Produces an IIFE bundle with `react-server` condition that contains all server
//! components. At `"use client"` boundaries, imports are replaced with client
//! reference stubs via rolldown aliases.

use crate::client_manifest::ClientReferenceManifest;
use crate::rsc_build_config::{build_rex_server_aliases, react_treeshake_options, RscBuildContext};
use crate::rsc_entries::generate_server_entry;
use crate::rsc_graph::ModuleGraph;
use crate::server_action_manifest::ServerActionManifest;
use anyhow::Result;
use rex_core::app_route::AppScanResult;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Build the server RSC flight bundle (IIFE, `react-server` condition).
///
/// This bundle includes all server components, with `"use client"` modules
/// replaced by reference stubs via rolldown aliases.
/// Uses `renderToReadableStream` from `react-server-dom-webpack/server`.
pub(crate) async fn build_rsc_server_bundle(
    ctx: &RscBuildContext<'_>,
    app_scan: &AppScanResult,
    _graph: &ModuleGraph,
    output_dir: &Path,
    stub_aliases: &[(PathBuf, PathBuf)],
    client_manifest: &ClientReferenceManifest,
    server_action_manifest: &ServerActionManifest,
) -> Result<PathBuf> {
    let entries_dir = output_dir.join("_rsc_server_entry");
    fs::create_dir_all(&entries_dir)?;

    // Generate entry source (pure function)
    let entry_source = generate_server_entry(
        app_scan,
        client_manifest,
        server_action_manifest,
        &ctx.project_root,
    );

    let entry_path = entries_dir.join("rsc-server-entry.ts");
    fs::write(&entry_path, &entry_source)?;

    // Build aliases: rex/* built-ins + "use client" module stubs.
    // Client reference stubs must take priority over rex server aliases.
    // e.g. rex/link has "use client" — its stub (a reference object) must win
    // over the server alias (plain <a>) so the flight data contains a client
    // reference and the client can hydrate with the interactive Link component.
    let mut aliases = build_rex_server_aliases()?;
    let stub_keys: std::collections::HashSet<String> = stub_aliases
        .iter()
        .map(|(p, _)| p.to_string_lossy().to_string())
        .collect();
    aliases.retain(|(k, _)| !stub_keys.contains(k));
    for (original, stub) in stub_aliases {
        aliases.push((
            original.to_string_lossy().to_string(),
            vec![Some(stub.to_string_lossy().to_string())],
        ));
    }

    let options = rolldown::BundlerOptions {
        input: Some(vec![rolldown::InputItem {
            name: Some("rsc-server-bundle".to_string()),
            import: entry_path.to_string_lossy().to_string(),
        }]),
        cwd: Some(ctx.config.project_root.clone()),
        format: Some(rolldown::OutputFormat::Iife),
        dir: Some(output_dir.to_string_lossy().to_string()),
        entry_filenames: Some("rsc-server-bundle.js".to_string().into()),
        platform: Some(rolldown::Platform::Browser),
        module_types: Some(ctx.non_js_empty_module_types()),
        define: Some(ctx.define.iter().cloned().collect()),
        banner: Some(rolldown::AddonOutputOption::String(Some(
            ctx.server_banner(),
        ))),
        // Disable tsconfig auto-resolution for the RSC server bundle.
        // tsconfig `paths` (e.g. "rex/*") would override our stub aliases,
        // resolving to the real module instead of the client reference stub.
        // The entry uses absolute paths, so tsconfig isn't needed here.
        tsconfig: Some(rolldown_common::TsConfig::Auto(false)),
        minify: ctx.minify_options(),
        treeshake: react_treeshake_options(),
        resolve: Some(rolldown::ResolveOptions {
            alias: Some(aliases),
            condition_names: Some(vec![
                "react-server".to_string(),
                "browser".to_string(),
                "import".to_string(),
                "default".to_string(),
            ]),
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

    let mut bundler = rolldown::Bundler::new(options)
        .map_err(|e| anyhow::anyhow!("Failed to create RSC server bundler: {e}"))?;

    bundler
        .write()
        .await
        .map_err(|e| anyhow::anyhow!("RSC server bundle failed: {e:?}"))?;

    let _ = fs::remove_dir_all(&entries_dir);

    let bundle_path = output_dir.join("rsc-server-bundle.js");
    debug!(path = %bundle_path.display(), "RSC server bundle written");
    Ok(bundle_path)
}
