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

    // Build aliases: rex/* built-ins + Node.js polyfills + tsconfig paths +
    // "use client" module stubs.
    // Client reference stubs must take priority over rex server aliases.
    // e.g. rex/link has "use client" — its stub (a reference object) must win
    // over the server alias (plain <a>) so the flight data contains a client
    // reference and the client can hydrate with the interactive Link component.
    let mut aliases = build_rex_server_aliases()?;
    let runtime_dir = crate::build_utils::runtime_server_dir()?;
    aliases.extend(crate::build_utils::node_polyfill_aliases(&runtime_dir));
    aliases.extend(ctx.mdx_aliases.clone());
    // tsconfig auto-resolution is disabled (to prevent rex/* overrides), so we
    // manually parse tsconfig paths for user aliases like @/ → src/.
    aliases.extend(crate::build_utils::tsconfig_path_aliases(&ctx.project_root));

    // Stub react-dom for the RSC server bundle. The flight bundle uses
    // react-server-dom-webpack (not react-dom), but some node_modules packages
    // (e.g. react-datepicker via PayloadCMS) that leak past "use client"
    // boundaries import react-dom. This stub prevents "Class extends undefined"
    // crashes without pulling in real DOM code.
    // TODO: Fix the RSC graph walker to detect "use client" in bare specifier
    // imports (node_modules), which would properly stub these modules instead.
    let react_dom_stub = runtime_dir.join("react-dom-server-stub.ts");
    if react_dom_stub.exists() {
        let stub = react_dom_stub.to_string_lossy().to_string();
        aliases.push(("react-dom".to_string(), vec![Some(stub.clone())]));
        aliases.push(("react-dom/client".to_string(), vec![Some(stub.clone())]));
        aliases.push(("react-dom/server".to_string(), vec![Some(stub.clone())]));
    }

    // Use the React server bridge which re-exports the react-server build
    // AND adds missing client APIs (createContext, useState, etc.) as stubs.
    // Many real-world libraries (PayloadCMS, etc.) use these APIs in components
    // that end up in the server bundle. The bridge preserves __SERVER_INTERNALS
    // needed by react-server-dom-webpack.
    let bridge_path = runtime_dir.join("react-server-bridge.ts");
    let react_server_cjs = ctx
        .project_root
        .join("node_modules/react/cjs/react.react-server.production.js");
    if bridge_path.exists() && react_server_cjs.exists() {
        aliases.push((
            "react".to_string(),
            vec![Some(bridge_path.to_string_lossy().to_string())],
        ));
        // Internal alias for the bridge to import the actual react-server CJS
        aliases.push((
            "react-server-cjs-internal".to_string(),
            vec![Some(react_server_cjs.to_string_lossy().to_string())],
        ));
    }

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

    let mut bundler = rolldown::Bundler::new(options)
        .map_err(|e| anyhow::anyhow!("Failed to create RSC server bundler: {e}"))?;

    bundler
        .write()
        .await
        .map_err(|e| anyhow::anyhow!("RSC server bundle failed: {e:?}"))?;

    let _ = fs::remove_dir_all(&entries_dir);

    let bundle_path = output_dir.join("rsc-server-bundle.js");

    // Patch __toCommonJS to handle ESM→CJS interop for default exports.
    // rolldown's __toCommonJS wraps ESM modules into namespace objects, but
    // CJS consumers (e.g. pg, undici) expect `require('module')` to return
    // the default export directly when it's a constructor/function.
    crate::cjs_interop::patch_to_common_js(&bundle_path)?;

    debug!(path = %bundle_path.display(), "RSC server bundle written");
    Ok(bundle_path)
}
