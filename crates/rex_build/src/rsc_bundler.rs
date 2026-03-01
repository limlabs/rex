//! RSC bundle builder.
//!
//! Produces two bundles from an app/ directory scan:
//! 1. **Server RSC bundle** (IIFE): Contains all server components. At `"use client"`
//!    boundaries, imports are replaced with client reference stubs.
//! 2. **Client bundle** (ESM): Contains only `"use client"` components and their
//!    dependencies, with code splitting.
//!
//! Also produces a `ClientReferenceManifest` mapping reference IDs to chunk URLs.

use crate::client_manifest::{client_reference_id, ClientReferenceManifest};
use crate::rsc_graph::{analyze_module_graph, ModuleGraph};
use anyhow::Result;
use rex_core::app_route::AppScanResult;
use rex_core::RexConfig;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Result of the RSC bundle build.
#[derive(Debug)]
pub struct RscBuildResult {
    /// Path to the server RSC bundle (IIFE).
    pub server_bundle_path: PathBuf,
    /// Client reference manifest mapping ref IDs to chunk URLs.
    pub client_manifest: ClientReferenceManifest,
    /// Client chunk files produced (relative paths from client output dir).
    pub client_chunks: Vec<String>,
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
            .strip_prefix(&config.project_root)
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

    // Build server RSC bundle
    let server_bundle_path =
        build_rsc_server_bundle(config, app_scan, &graph, &server_dir, &stub_aliases, define)
            .await?;

    // Build client bundles for "use client" modules
    let client_chunks = build_rsc_client_bundles(
        config,
        &graph,
        &client_dir,
        build_id,
        define,
        &mut client_manifest,
    )
    .await?;

    // Clean up stubs
    let _ = fs::remove_dir_all(&stubs_dir);

    Ok(RscBuildResult {
        server_bundle_path,
        client_manifest,
        client_chunks,
    })
}

/// Generate a client reference stub module for a `"use client"` component.
///
/// For each export, produces:
/// ```js
/// export const Foo = { $$typeof: Symbol.for("react.client.reference"), $$id: "<refId>", $$name: "Foo" };
/// ```
fn generate_client_stub(rel_path: &str, exports: &[String], build_id: &str) -> String {
    let mut source = String::new();
    source.push_str("// Auto-generated client reference stub\n");

    for export in exports {
        let ref_id = client_reference_id(rel_path, export, build_id);
        let obj = format!(
            "{{ $$typeof: Symbol.for(\"react.client.reference\"), $$id: \"{ref_id}\", $$name: \"{export}\" }}"
        );

        if export == "default" {
            source.push_str(&format!("export default {obj};\n"));
        } else {
            source.push_str(&format!("export const {export} = {obj};\n"));
        }
    }

    source
}

/// Build the server RSC bundle (IIFE).
///
/// This bundle includes all server components, with `"use client"` modules
/// replaced by reference stubs via rolldown aliases.
async fn build_rsc_server_bundle(
    config: &RexConfig,
    app_scan: &AppScanResult,
    _graph: &ModuleGraph,
    output_dir: &Path,
    stub_aliases: &[(PathBuf, PathBuf)],
    define: &[(String, String)],
) -> Result<PathBuf> {
    let entries_dir = output_dir.join("_rsc_server_entry");
    fs::create_dir_all(&entries_dir)?;

    let mut entry = String::new();

    // React imports
    entry.push_str("import { createElement } from 'react';\n");
    entry.push_str("import { renderToString } from 'react-dom/server';\n");
    entry.push_str("var __rex_createElement = createElement;\n");
    entry.push_str("var __rex_renderToString = renderToString;\n\n");

    // Register layouts
    entry.push_str("globalThis.__rex_app_layouts = {};\n");
    for (i, route) in app_scan.routes.iter().enumerate() {
        for (j, layout) in route.layout_chain.iter().enumerate() {
            let layout_path = layout.to_string_lossy().replace('\\', "/");
            let layout_key = format!("layout_{i}_{j}");
            entry.push_str(&format!(
                "import {{ default as {layout_key} }} from '{layout_path}';\n"
            ));
        }
    }

    // Register pages
    entry.push_str("\nglobalThis.__rex_app_pages = {};\n");
    for (i, route) in app_scan.routes.iter().enumerate() {
        let page_path = route.page_path.to_string_lossy().replace('\\', "/");
        let page_var = format!("__app_page_{i}");
        entry.push_str(&format!(
            "import {{ default as {page_var} }} from '{page_path}';\n"
        ));
        let pattern = &route.pattern;
        entry.push_str(&format!(
            "globalThis.__rex_app_pages[\"{pattern}\"] = {page_var};\n"
        ));
    }

    // Register layout chains per route
    entry.push_str("\nglobalThis.__rex_app_layout_chains = {};\n");
    for (i, route) in app_scan.routes.iter().enumerate() {
        let layout_vars: Vec<String> = (0..route.layout_chain.len())
            .map(|j| format!("layout_{i}_{j}"))
            .collect();
        let array = format!("[{}]", layout_vars.join(", "));
        let pattern = &route.pattern;
        entry.push_str(&format!(
            "globalThis.__rex_app_layout_chains[\"{pattern}\"] = {array};\n"
        ));
    }

    // RSC runtime: flight protocol serializer + render functions
    let flight_runtime = include_str!("../../../runtime/rsc/flight.js");
    entry.push_str("\n// --- RSC Flight Runtime ---\n");
    entry.push_str(flight_runtime);

    let entry_path = entries_dir.join("rsc-server-entry.js");
    fs::write(&entry_path, &entry)?;

    // CSS → empty module
    let mut module_types = rustc_hash::FxHashMap::default();
    module_types.insert(".css".to_string(), rolldown::ModuleType::Empty);

    // Build aliases: map "use client" modules to their stubs
    let mut aliases: Vec<(String, Vec<Option<String>>)> = Vec::new();
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
        cwd: Some(config.project_root.clone()),
        format: Some(rolldown::OutputFormat::Iife),
        dir: Some(output_dir.to_string_lossy().to_string()),
        entry_filenames: Some("rsc-server-bundle.js".to_string().into()),
        platform: Some(rolldown::Platform::Browser),
        module_types: Some(module_types),
        define: Some(define.iter().cloned().collect()),
        banner: Some(rolldown::AddonOutputOption::String(Some(
            crate::bundler::V8_POLYFILLS.to_string(),
        ))),
        tsconfig: Some(rolldown_common::TsConfig::Auto(true)),
        resolve: Some(rolldown::ResolveOptions {
            alias: Some(aliases),
            extensions: Some(vec![
                ".tsx".to_string(),
                ".ts".to_string(),
                ".jsx".to_string(),
                ".js".to_string(),
            ]),
            modules: Some(vec![
                config
                    .project_root
                    .join("node_modules")
                    .to_string_lossy()
                    .to_string(),
                "node_modules".to_string(),
            ]),
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

/// Build client bundles for `"use client"` modules.
///
/// Each client boundary module becomes a separate entry. Rolldown handles
/// code splitting so shared dependencies (React) become shared chunks.
async fn build_rsc_client_bundles(
    config: &RexConfig,
    graph: &ModuleGraph,
    output_dir: &Path,
    build_id: &str,
    define: &[(String, String)],
    client_manifest: &mut ClientReferenceManifest,
) -> Result<Vec<String>> {
    let client_boundaries = graph.client_boundary_modules();
    if client_boundaries.is_empty() {
        return Ok(vec![]);
    }

    let hash = &build_id[..8.min(build_id.len())];

    // Create a hydration bootstrap entry that imports react + react-dom/client
    // and sets window globals so rsc-runtime.js can access them.
    // Rolldown will code-split React into shared chunks between this and component entries.
    let entries_dir = output_dir.join("_rsc_client_entries");
    fs::create_dir_all(&entries_dir)?;

    let bootstrap_code = r#"import React from 'react';
import * as ReactDOMClient from 'react-dom/client';
window.React = React;
window.ReactDOM = ReactDOMClient;
"#;
    let bootstrap_path = entries_dir.join("__rsc_bootstrap.js");
    fs::write(&bootstrap_path, bootstrap_code)?;

    // Create entries: bootstrap + each client boundary module
    let mut entries: Vec<rolldown::InputItem> = vec![rolldown::InputItem {
        name: Some("__rsc_bootstrap".to_string()),
        import: bootstrap_path.to_string_lossy().to_string(),
    }];

    entries.extend(client_boundaries.iter().map(|m| {
        let rel_path = m.path.strip_prefix(&config.project_root).unwrap_or(&m.path);
        let name = sanitize_filename(&rel_path.to_string_lossy());
        rolldown::InputItem {
            name: Some(name),
            import: m.path.to_string_lossy().to_string(),
        }
    }));

    // CSS → empty module
    let mut module_types = rustc_hash::FxHashMap::default();
    module_types.insert(".css".to_string(), rolldown::ModuleType::Empty);

    let options = rolldown::BundlerOptions {
        input: Some(entries),
        cwd: Some(config.project_root.clone()),
        format: Some(rolldown::OutputFormat::Esm),
        dir: Some(output_dir.to_string_lossy().to_string()),
        entry_filenames: Some(format!("[name]-{hash}.js").into()),
        chunk_filenames: Some(format!("chunk-[name]-{hash}.js").into()),
        platform: Some(rolldown::Platform::Browser),
        module_types: Some(module_types),
        define: Some(define.iter().cloned().collect()),
        tsconfig: Some(rolldown_common::TsConfig::Auto(true)),
        resolve: Some(rolldown::ResolveOptions {
            extensions: Some(vec![
                ".tsx".to_string(),
                ".ts".to_string(),
                ".jsx".to_string(),
                ".js".to_string(),
            ]),
            modules: Some(vec![
                config
                    .project_root
                    .join("node_modules")
                    .to_string_lossy()
                    .to_string(),
                "node_modules".to_string(),
            ]),
            ..Default::default()
        }),
        ..Default::default()
    };

    let mut bundler = rolldown::Bundler::new(options)
        .map_err(|e| anyhow::anyhow!("Failed to create RSC client bundler: {e}"))?;

    let output = bundler
        .write()
        .await
        .map_err(|e| anyhow::anyhow!("RSC client bundle failed: {e:?}"))?;

    // Clean up temporary entry files
    let _ = fs::remove_dir_all(&entries_dir);

    // Collect output chunks and update the client manifest.
    // Prefix with "rsc/" because the output dir is client_build_dir/rsc/
    // and the static file server mounts client_build_dir at /_rex/static/.
    //
    // The bootstrap chunk must come first so React/ReactDOM globals are set
    // before component chunks (loaded as type="module") execute.
    let mut bootstrap_chunks = Vec::new();
    let mut component_chunks = Vec::new();

    for item in &output.assets {
        match item {
            rolldown_common::Output::Chunk(chunk) => {
                let filename = chunk.filename.to_string();
                let prefixed = format!("rsc/{filename}");

                if chunk.is_entry {
                    let name = chunk.name.to_string();

                    if name == "__rsc_bootstrap" {
                        // Bootstrap entry — loads React/ReactDOM globals
                        bootstrap_chunks.push(prefixed);
                    } else {
                        // Client component entry — update manifest
                        component_chunks.push(prefixed);
                        for module in &client_boundaries {
                            let rel_path = module
                                .path
                                .strip_prefix(&config.project_root)
                                .unwrap_or(&module.path)
                                .to_string_lossy()
                                .replace('\\', "/");
                            let module_name = sanitize_filename(&rel_path);

                            if module_name == name {
                                let chunk_url = format!("/_rex/static/rsc/{filename}");

                                for export in &module.exports {
                                    let ref_id = client_reference_id(&rel_path, export, build_id);
                                    client_manifest.add(&ref_id, chunk_url.clone(), export.clone());
                                }
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

    debug!(
        count = chunks.len(),
        "RSC client bundles written to {}",
        output_dir.display()
    );
    Ok(chunks)
}

/// Sanitize a file path into a valid chunk name.
fn sanitize_filename(path: &str) -> String {
    path.replace(['/', '\\', '.'], "_")
        .trim_matches('_')
        .to_string()
}

// The RSC runtime is now loaded from `runtime/rsc/flight.js` via `include_str!`.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_stub_default_export() {
        let stub = generate_client_stub("components/Counter.tsx", &["default".to_string()], "abc");
        assert!(stub.contains("export default"));
        assert!(stub.contains("react.client.reference"));
        assert!(stub.contains("$$name: \"default\""));
    }

    #[test]
    fn generate_stub_named_exports() {
        let stub = generate_client_stub(
            "utils.tsx",
            &["Counter".to_string(), "Input".to_string()],
            "abc",
        );
        assert!(stub.contains("export const Counter"));
        assert!(stub.contains("export const Input"));
        assert!(!stub.contains("export default"));
    }

    #[test]
    fn sanitize_path() {
        assert_eq!(
            sanitize_filename("components/Counter.tsx"),
            "components_Counter_tsx"
        );
        assert_eq!(sanitize_filename("app/page.tsx"), "app_page_tsx");
    }
}
