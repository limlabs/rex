//! RSC server flight bundle builder.
//!
//! Produces IIFE bundle(s) with `react-server` condition that contain all server
//! components. At `"use client"` boundaries, imports are replaced with client
//! reference stubs via rolldown aliases.
//!
//! When route groups are detected (e.g. `(app)`, `(payload)`), the builder
//! produces a **core IIFE** (React, runtime, server actions) plus **per-group
//! IIFEs** (layouts and pages), concatenated into a single output file. Group
//! IIFEs share the React instance from the core via `globalThis.__rex_react_ns`.

use crate::client_manifest::ClientReferenceManifest;
use crate::rsc_build_config::{build_rex_server_aliases, react_treeshake_options, RscBuildContext};
use crate::rsc_graph::ModuleGraph;
use crate::server_action_manifest::ServerActionManifest;
use anyhow::Result;
use rex_core::app_route::{AppRoute, AppScanResult};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::debug;

type AliasList = Vec<(String, Vec<Option<String>>)>;

/// Build the server RSC flight bundle (IIFE, `react-server` condition).
///
/// When routes span multiple route groups, builds a core IIFE plus per-group
/// IIFEs and concatenates them. Otherwise builds a single IIFE as before.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn build_rsc_server_bundle(
    ctx: &RscBuildContext<'_>,
    app_scan: &AppScanResult,
    _graph: &ModuleGraph,
    output_dir: &Path,
    stub_aliases: &[(PathBuf, PathBuf)],
    client_manifest: &ClientReferenceManifest,
    server_action_manifest: &ServerActionManifest,
    inline_action_targets: &[PathBuf],
) -> Result<PathBuf> {
    let groups = app_scan.routes_by_group();
    let has_route_groups = groups.len() > 1 || groups.iter().any(|(g, _)| g.is_some());

    if has_route_groups {
        build_grouped(
            ctx,
            app_scan,
            output_dir,
            stub_aliases,
            client_manifest,
            server_action_manifest,
            &groups,
            inline_action_targets,
        )
        .await
    } else {
        build_single(
            ctx,
            app_scan,
            output_dir,
            stub_aliases,
            client_manifest,
            server_action_manifest,
            inline_action_targets,
        )
        .await
    }
}

/// Build a single monolithic RSC server bundle (no route groups).
async fn build_single(
    ctx: &RscBuildContext<'_>,
    app_scan: &AppScanResult,
    output_dir: &Path,
    stub_aliases: &[(PathBuf, PathBuf)],
    client_manifest: &ClientReferenceManifest,
    server_action_manifest: &ServerActionManifest,
    inline_action_targets: &[PathBuf],
) -> Result<PathBuf> {
    let entries_dir = output_dir.join("_rsc_server_entry");
    fs::create_dir_all(&entries_dir)?;

    let entry_source = crate::rsc_entries::generate_server_entry(
        app_scan,
        client_manifest,
        server_action_manifest,
        &ctx.project_root,
    );
    let entry_path = entries_dir.join("rsc-server-entry.ts");
    fs::write(&entry_path, &entry_source)?;

    let (mut aliases, runtime_dir) = build_base_aliases(ctx)?;
    add_react_core_aliases(&mut aliases, ctx, &runtime_dir);
    apply_stub_overrides(&mut aliases, stub_aliases);

    let client_asset_dir = client_asset_dir_from(output_dir);
    let mut plugins = build_plugins(output_dir, &runtime_dir, &client_asset_dir);
    if !inline_action_targets.is_empty() {
        plugins.push(std::sync::Arc::new(
            crate::server_action_extract::InlineServerActionPlugin::new(
                inline_action_targets.to_vec(),
                ctx.project_root.clone(),
                ctx.build_id.to_string(),
            ),
        ));
    }
    let bundle_path = output_dir.join("rsc-server-bundle.js");

    run_iife_build(
        ctx,
        &entry_path,
        output_dir,
        "rsc-server-bundle",
        "rsc-server-bundle.js",
        aliases,
        plugins,
        Some(ctx.server_banner()),
    )
    .await?;

    let _ = fs::remove_dir_all(&entries_dir);
    crate::cjs_interop::patch_to_common_js(&bundle_path)?;

    debug!(path = %bundle_path.display(), "RSC server bundle written");
    Ok(bundle_path)
}

/// Build core IIFE + per-group IIFEs, concatenated into a single output file.
#[allow(clippy::too_many_arguments)]
async fn build_grouped(
    ctx: &RscBuildContext<'_>,
    app_scan: &AppScanResult,
    output_dir: &Path,
    stub_aliases: &[(PathBuf, PathBuf)],
    client_manifest: &ClientReferenceManifest,
    server_action_manifest: &ServerActionManifest,
    groups: &[(Option<String>, Vec<&AppRoute>)],
    inline_action_targets: &[PathBuf],
) -> Result<PathBuf> {
    let entries_dir = output_dir.join("_rsc_server_entry");
    fs::create_dir_all(&entries_dir)?;

    let (base_aliases, runtime_dir) = build_base_aliases(ctx)?;
    let banner = ctx.server_banner();

    // --- Build core IIFE (React, runtime, server actions, no layouts/pages) ---
    let core_source = crate::rsc_entries::generate_core_entry(
        app_scan,
        client_manifest,
        server_action_manifest,
        &ctx.project_root,
    );
    let core_entry_path = entries_dir.join("rsc-core-entry.ts");
    fs::write(&core_entry_path, &core_source)?;

    let mut core_aliases = base_aliases.clone();
    add_react_core_aliases(&mut core_aliases, ctx, &runtime_dir);
    apply_stub_overrides(&mut core_aliases, stub_aliases);

    let core_out_dir = output_dir.join("_rsc_core");
    fs::create_dir_all(&core_out_dir)?;
    let client_asset_dir = client_asset_dir_from(output_dir);
    let mut core_plugins = build_plugins(&core_out_dir, &runtime_dir, &client_asset_dir);
    if !inline_action_targets.is_empty() {
        core_plugins.push(std::sync::Arc::new(
            crate::server_action_extract::InlineServerActionPlugin::new(
                inline_action_targets.to_vec(),
                ctx.project_root.clone(),
                ctx.build_id.to_string(),
            ),
        ));
    }

    run_iife_build(
        ctx,
        &core_entry_path,
        &core_out_dir,
        "rsc-core",
        "rsc-core.js",
        core_aliases,
        core_plugins,
        Some(banner),
    )
    .await?;

    let core_js = fs::read_to_string(core_out_dir.join("rsc-core.js"))?;

    // --- Build per-group IIFEs ---
    let mut group_js_parts: Vec<String> = Vec::new();

    for (group_name, routes) in groups {
        let label = group_name.as_deref().unwrap_or("default");
        let group_source = crate::rsc_entries::generate_group_entry(routes);
        let group_entry_path = entries_dir.join(format!("rsc-group-{label}-entry.ts"));
        fs::write(&group_entry_path, &group_source)?;

        let mut group_aliases = base_aliases.clone();
        add_react_group_aliases(&mut group_aliases, &runtime_dir);
        apply_stub_overrides(&mut group_aliases, stub_aliases);

        let group_out_dir = output_dir.join(format!("_rsc_group_{label}"));
        fs::create_dir_all(&group_out_dir)?;
        let mut group_plugins = build_plugins(&group_out_dir, &runtime_dir, &client_asset_dir);
        if !inline_action_targets.is_empty() {
            group_plugins.push(std::sync::Arc::new(
                crate::server_action_extract::InlineServerActionPlugin::new(
                    inline_action_targets.to_vec(),
                    ctx.project_root.clone(),
                    ctx.build_id.to_string(),
                ),
            ));
        }

        let filename = format!("rsc-group-{label}.js");
        run_iife_build(
            ctx,
            &group_entry_path,
            &group_out_dir,
            &format!("rsc-group-{label}"),
            &filename,
            group_aliases,
            group_plugins,
            None, // no banner — V8 polyfills already loaded by core
        )
        .await?;

        let group_js = fs::read_to_string(group_out_dir.join(&filename))?;
        group_js_parts.push(group_js);

        let _ = fs::remove_dir_all(&group_out_dir);
        debug!(group = label, "RSC group bundle built");
    }

    // --- Concatenate core + groups into single output ---
    let mut combined = core_js;
    for part in &group_js_parts {
        combined.push_str("\n;\n");
        combined.push_str(part);
    }

    let bundle_path = output_dir.join("rsc-server-bundle.js");
    fs::write(&bundle_path, &combined)?;

    let _ = fs::remove_dir_all(&entries_dir);
    let _ = fs::remove_dir_all(&core_out_dir);
    crate::cjs_interop::patch_to_common_js(&bundle_path)?;

    debug!(
        path = %bundle_path.display(),
        groups = groups.len(),
        "RSC grouped server bundle written"
    );
    Ok(bundle_path)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Derive the client asset output directory from the server RSC output dir.
///
/// Server output: `{build_root}/server/rsc/`
/// Client assets: `{build_root}/client/assets/`
fn client_asset_dir_from(server_rsc_dir: &Path) -> PathBuf {
    server_rsc_dir
        .parent() // server/
        .and_then(|p| p.parent()) // build root
        .unwrap_or(server_rsc_dir)
        .join("client")
        .join("assets")
}

/// Build base resolve aliases shared by all RSC server bundles.
/// Excludes React alias (varies per bundle type) and stub overrides.
fn build_base_aliases(ctx: &RscBuildContext<'_>) -> Result<(AliasList, PathBuf)> {
    let mut aliases = build_rex_server_aliases()?;
    let runtime_dir = crate::build_utils::runtime_server_dir()?;
    aliases.extend(crate::build_utils::node_polyfill_aliases(&runtime_dir));
    aliases.extend(ctx.mdx_aliases.clone());

    let tsconfig_aliases = crate::build_utils::tsconfig_path_aliases(&ctx.project_root);
    aliases.extend(tsconfig_aliases.into_iter().filter(|(k, _)| k != "rex"));

    // Stub react-dom — the flight bundle uses react-server-dom-webpack, not react-dom.
    let react_dom_stub = runtime_dir.join("react-dom-server-stub.ts");
    if react_dom_stub.exists() {
        let stub = react_dom_stub.to_string_lossy().to_string();
        aliases.push(("react-dom".to_string(), vec![Some(stub.clone())]));
        aliases.push(("react-dom/client".to_string(), vec![Some(stub.clone())]));
        aliases.push(("react-dom/server".to_string(), vec![Some(stub.clone())]));
    }

    Ok((aliases, runtime_dir))
}

/// Add React server bridge aliases for the core/single bundle.
fn add_react_core_aliases(aliases: &mut AliasList, ctx: &RscBuildContext<'_>, runtime_dir: &Path) {
    let bridge_path = runtime_dir.join("react-server-bridge.ts");
    let react_variant = if ctx.config.dev {
        "development"
    } else {
        "production"
    };
    let react_server_cjs = ctx.project_root.join(format!(
        "node_modules/react/cjs/react.react-server.{react_variant}.js"
    ));
    if bridge_path.exists() && react_server_cjs.exists() {
        aliases.push((
            "react".to_string(),
            vec![Some(bridge_path.to_string_lossy().to_string())],
        ));
        aliases.push((
            "react-server-cjs-internal".to_string(),
            vec![Some(react_server_cjs.to_string_lossy().to_string())],
        ));
    }
}

/// Add React group shim aliases for per-group bundles.
/// These resolve `react` to a shim that reads from `globalThis.__rex_react_ns`.
fn add_react_group_aliases(aliases: &mut AliasList, runtime_dir: &Path) {
    let shim = runtime_dir
        .join("react-group-shim.ts")
        .to_string_lossy()
        .to_string();
    let jsx_shim = runtime_dir
        .join("react-jsx-group-shim.ts")
        .to_string_lossy()
        .to_string();
    let jsx_dev_shim = runtime_dir
        .join("react-jsx-dev-group-shim.ts")
        .to_string_lossy()
        .to_string();
    aliases.push(("react".to_string(), vec![Some(shim)]));
    aliases.push(("react/jsx-runtime".to_string(), vec![Some(jsx_shim)]));
    aliases.push((
        "react/jsx-dev-runtime".to_string(),
        vec![Some(jsx_dev_shim)],
    ));
}

/// Apply `"use client"` stub alias overrides (must be last to take priority).
fn apply_stub_overrides(aliases: &mut AliasList, stub_aliases: &[(PathBuf, PathBuf)]) {
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
}

/// Build shared rolldown plugins for RSC server bundles.
fn build_plugins(
    output_dir: &Path,
    runtime_dir: &Path,
    client_asset_dir: &Path,
) -> Vec<std::sync::Arc<dyn rolldown::plugin::Pluginable>> {
    let empty_stub = runtime_dir.join("empty.ts").to_string_lossy().to_string();

    let polyfill_plugin: std::sync::Arc<dyn rolldown::plugin::Pluginable> =
        std::sync::Arc::new(crate::server_bundle::NodePolyfillResolvePlugin::new(
            vec![
                ("@vercel/og".to_string(), empty_stub.clone()),
                (
                    "next/dist/compiled/@vercel/og".to_string(),
                    empty_stub.clone(),
                ),
                ("next/og".to_string(), empty_stub.clone()),
            ],
            empty_stub,
        ));

    let css_module_plugin: std::sync::Arc<dyn rolldown::plugin::Pluginable> = std::sync::Arc::new(
        crate::server_bundle::CssModulePlugin::new(output_dir.join("_css_module_proxies")),
    );

    let use_client_plugin: std::sync::Arc<dyn rolldown::plugin::Pluginable> =
        std::sync::Arc::new(crate::use_client_detect::UseClientDetectPlugin::new());

    // Stub packages that use Node.js native modules and can never run in V8.
    // pg/pg-pool/pg-protocol are NOT stubbed here — the RSC server bundle needs
    // real database access via Rex's net.Socket TCP polyfill. They ARE stubbed
    // in the SSR bundle (rsc_ssr_bundle.rs) which only hydrates HTML.
    let heavy_stub_plugin: std::sync::Arc<dyn rolldown::plugin::Pluginable> =
        std::sync::Arc::new(crate::server_bundle::HeavyPackageStubPlugin::new(vec![
            "node_modules/@aws-sdk/".to_string(),
            "node_modules/@smithy/".to_string(),
            "node_modules/pg-native/".to_string(),
            "node_modules/@node-rs/".to_string(),
            "node_modules/undici/".to_string(),
        ]));

    let static_asset_plugin: std::sync::Arc<dyn rolldown::plugin::Pluginable> = std::sync::Arc::new(
        crate::static_asset::StaticAssetPlugin::new(client_asset_dir.to_path_buf()),
    );

    vec![
        use_client_plugin,
        polyfill_plugin,
        css_module_plugin,
        heavy_stub_plugin,
        static_asset_plugin,
    ]
}

/// Run a single IIFE rolldown build.
#[allow(clippy::too_many_arguments)]
async fn run_iife_build(
    ctx: &RscBuildContext<'_>,
    entry_path: &Path,
    output_dir: &Path,
    entry_name: &str,
    output_filename: &str,
    aliases: AliasList,
    plugins: Vec<std::sync::Arc<dyn rolldown::plugin::Pluginable>>,
    banner: Option<String>,
) -> Result<()> {
    let options = rolldown::BundlerOptions {
        input: Some(vec![rolldown::InputItem {
            name: Some(entry_name.to_string()),
            import: entry_path.to_string_lossy().to_string(),
        }]),
        cwd: Some(ctx.config.project_root.clone()),
        format: Some(rolldown::OutputFormat::Iife),
        dir: Some(output_dir.to_string_lossy().to_string()),
        entry_filenames: Some(output_filename.to_string().into()),
        platform: Some(rolldown::Platform::Browser),
        module_types: Some(ctx.non_js_empty_module_types()),
        define: Some(ctx.define.iter().cloned().collect()),
        banner: banner.map(|b| rolldown::AddonOutputOption::String(Some(b))),
        tsconfig: Some(rolldown_common::TsConfig::Auto(false)),
        minify: ctx.minify_options(),
        shim_missing_exports: Some(true),
        treeshake: react_treeshake_options(),
        resolve: Some(rolldown::ResolveOptions {
            alias: Some(aliases),
            condition_names: Some(vec![
                "react-server".to_string(),
                "workerd".to_string(),
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

    let mut bundler = rolldown::Bundler::with_plugins(options, plugins)
        .map_err(|e| anyhow::anyhow!("Failed to create RSC bundler ({entry_name}): {e}"))?;

    if let Err(e) = bundler.write().await {
        if !crate::diagnostics::is_all_missing_exports(&e) {
            let formatted = crate::diagnostics::format_build_diagnostics(&e);
            tracing::error!("RSC bundle ({entry_name}) diagnostics:\n{formatted}");
            return Err(anyhow::anyhow!(
                "RSC bundle ({entry_name}) failed:\n{formatted}"
            ));
        }
        tracing::warn!(
            "RSC bundle ({entry_name}) had {} shimmed missing export(s)",
            e.len()
        );
    }

    Ok(())
}
