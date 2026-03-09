//! RSC SSR bundle builder.
//!
//! Produces an IIFE bundle (standard conditions) that consumes flight data and
//! produces HTML using `createFromReadableStream` and `renderToString`.
//! Also includes all `"use client"` components for SSR rendering.

use crate::client_manifest::ClientReferenceManifest;
use crate::rsc_build_config::{build_rex_aliases, react_treeshake_options, RscBuildContext};
use crate::rsc_entries::generate_ssr_entry;
use crate::rsc_graph::ModuleGraph;
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Build the SSR bundle (IIFE, standard conditions).
///
/// This bundle consumes flight data and produces HTML using:
/// - `createFromReadableStream` from `react-server-dom-webpack/client`
/// - `renderToString` from `react-dom/server`
///
/// It also includes all `"use client"` components for SSR rendering.
pub(crate) async fn build_rsc_ssr_bundle(
    ctx: &RscBuildContext<'_>,
    graph: &ModuleGraph,
    output_dir: &Path,
    client_manifest: &ClientReferenceManifest,
) -> Result<PathBuf> {
    let entries_dir = output_dir.join("_rsc_ssr_entry");
    fs::create_dir_all(&entries_dir)?;

    // Generate entry source (pure function)
    let entry_source = generate_ssr_entry(graph, client_manifest, &ctx.project_root, ctx.build_id);

    let entry_path = entries_dir.join("rsc-ssr-entry.ts");
    fs::write(&entry_path, &entry_source)?;

    // Rex built-in aliases for SSR bundle (rex/link → client runtime, etc.)
    let ssr_aliases = build_rex_aliases()?;

    let options = rolldown::BundlerOptions {
        input: Some(vec![rolldown::InputItem {
            name: Some("rsc-ssr-bundle".to_string()),
            import: entry_path.to_string_lossy().to_string(),
        }]),
        cwd: Some(ctx.config.project_root.clone()),
        format: Some(rolldown::OutputFormat::Iife),
        dir: Some(output_dir.to_string_lossy().to_string()),
        entry_filenames: Some("rsc-ssr-bundle.js".to_string().into()),
        platform: Some(rolldown::Platform::Browser),
        module_types: Some(ctx.non_js_empty_module_types()),
        define: Some(ctx.define.iter().cloned().collect()),
        banner: Some(rolldown::AddonOutputOption::String(Some(
            ctx.server_banner(),
        ))),
        tsconfig: Some(rolldown_common::TsConfig::Auto(true)),
        minify: ctx.minify_options(),
        treeshake: react_treeshake_options(),
        resolve: Some(rolldown::ResolveOptions {
            alias: Some(ssr_aliases),
            condition_names: Some(vec![
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
        .map_err(|e| anyhow::anyhow!("Failed to create RSC SSR bundler: {e}"))?;

    bundler
        .write()
        .await
        .map_err(|e| anyhow::anyhow!("RSC SSR bundle failed: {e:?}"))?;

    let _ = fs::remove_dir_all(&entries_dir);

    let bundle_path = output_dir.join("rsc-ssr-bundle.js");
    debug!(path = %bundle_path.display(), "RSC SSR bundle written");
    Ok(bundle_path)
}
