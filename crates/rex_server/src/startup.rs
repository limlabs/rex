//! V8 isolate pool startup via native ESM module loading.
//!
//! Pre-bundles deps with rolldown as ESM modules, OXC-transforms user source
//! files, and loads everything into V8's native module system. This enables
//! per-file HMR invalidation (~100ms vs ~3500ms for full rolldown rebuild).

use crate::state::EsmState;
use anyhow::Result;
use rex_core::RexConfig;
use rex_router::ScanResult;
use rex_v8::{EsmSourceModule, IsolatePool};
use std::sync::Arc;
use tracing::debug;

/// Stub render functions for ESM bootstrap. Overwritten by `load_esm_modules()`
/// once the real entry module is evaluated.
pub const STUB_FUNCTIONS: &str = r#"
globalThis.__rex_pages = {};
globalThis.__rex_render_page = function() { return JSON.stringify({ body: '', head: '' }); };
globalThis.__rex_get_server_side_props = function() { return JSON.stringify({ props: {} }); };
globalThis.__rex_get_static_props = function() { return JSON.stringify({ props: {} }); };
"#;

/// ESM startup: pre-bundle deps as ESM, OXC-transform source files, load into V8.
/// ESM startup: pre-bundle deps as ESM, OXC-transform source files, load into V8.
///
/// If `pool` is provided, loads modules into the existing pool.
/// If `None`, creates a new pool. Returns the pool and ESM state.
pub async fn esm_startup(
    config: &RexConfig,
    scan: &ScanResult,
    build_id: &str,
    pool_size: usize,
    project_root_str: &str,
) -> Result<(IsolatePool, EsmState)> {
    let pool = IsolatePool::new(
        pool_size,
        Arc::new(STUB_FUNCTIONS.to_string()),
        Some(Arc::new(project_root_str.to_string())),
    )?;
    let esm_state = esm_load_modules(config, scan, build_id, &pool, None).await?;
    Ok((pool, esm_state))
}

/// Walk the ESM import graph and pre-compute RSC reference IDs.
///
/// This is a lightweight operation (~10-50ms) that does OXC parsing and BFS
/// graph walk — no rolldown bundling or V8 involvement. The result is used
/// to ensure the IIFE build (client bundles, SSR bundle) uses the same IDs
/// as the ESM module loader.
///
/// Returns `None` for pages-only projects (no app router).
pub fn esm_collect_ids(
    config: &RexConfig,
    scan: &ScanResult,
    build_id: &str,
) -> Result<Option<rex_build::precomputed_ids::PrecomputedIds>> {
    use rex_build::esm_transform;

    if scan.app_scan.is_none() {
        return Ok(None);
    }

    let known_specifiers = esm_transform::dep_specifiers(true);

    // Collect entry paths (same logic as in esm_load_modules)
    let mut entry_paths: Vec<std::path::PathBuf> = Vec::new();
    for route in &scan.routes {
        entry_paths.push(route.abs_path.clone());
    }
    for route in &scan.api_routes {
        entry_paths.push(route.abs_path.clone());
    }
    for tool in &scan.mcp_tools {
        entry_paths.push(tool.abs_path.clone());
    }
    if let Some(app_scan) = &scan.app_scan {
        for route in &app_scan.routes {
            entry_paths.push(route.page_path.clone());
            entry_paths.extend(route.layout_chain.iter().cloned());
        }
        for api_route in &app_scan.api_routes {
            entry_paths.push(api_route.handler_path.clone());
        }
    }

    debug!(
        entries = entry_paths.len(),
        "ESM collect IDs: walking import graph"
    );

    let collected = esm_transform::collect_source_modules_with_stubs(
        &entry_paths,
        &config.project_root,
        &known_specifiers,
        build_id,
    )?;

    debug!(
        client_boundaries = collected.client_boundaries.len(),
        server_actions = collected.server_action_modules.len(),
        extracted_actions = collected.extracted_actions.len(),
        "ESM collect IDs: pre-computed"
    );

    Ok(Some(
        rex_build::precomputed_ids::PrecomputedIds::from_collected(&collected, build_id),
    ))
}

/// Load ESM modules into an existing isolate pool.
///
/// `client_manifest` is used only for chunk URLs (not ref IDs) — the ESM path
/// computes its own ref IDs, which are authoritative.
pub async fn esm_load_modules(
    config: &RexConfig,
    scan: &ScanResult,
    build_id: &str,
    pool: &IsolatePool,
    _client_manifest: Option<&rex_core::client_manifest::ClientReferenceManifest>,
) -> Result<EsmState> {
    use rex_build::esm_transform;
    use rex_build::server_bundle::SSR_RUNTIME;

    let module_dirs = rex_build::resolve_modules_dirs(config)?;
    let has_app = scan.app_scan.is_some();

    // Pre-bundle deps as ESM modules (rolldown resolves + bundles each dep)
    let dep_bundles =
        rex_build::server_dep_bundle::build_dep_esm_modules(config, has_app, &module_dirs).await?;

    let dep_modules: Vec<EsmSourceModule> = dep_bundles
        .modules
        .into_iter()
        .map(|m| EsmSourceModule {
            specifier: m.specifier,
            source: m.source,
        })
        .collect();

    // Build known specifiers set (deps handled by rolldown, skip in import graph)
    let known_specifiers = esm_transform::dep_specifiers(has_app);

    // Collect entry paths for import graph walking.
    // MDX files should already be resolved to compiled .jsx by the caller.
    let mut entry_paths: Vec<std::path::PathBuf> = Vec::new();
    for route in &scan.routes {
        entry_paths.push(route.abs_path.clone());
    }
    for route in &scan.api_routes {
        entry_paths.push(route.abs_path.clone());
    }
    for tool in &scan.mcp_tools {
        entry_paths.push(tool.abs_path.clone());
    }
    if let Some(app_scan) = &scan.app_scan {
        for route in &app_scan.routes {
            entry_paths.push(route.page_path.clone());
            entry_paths.extend(route.layout_chain.iter().cloned());
        }
        for api_route in &app_scan.api_routes {
            entry_paths.push(api_route.handler_path.clone());
        }
    }

    debug!(entries = entry_paths.len(), "Walking import graph for ESM");

    // Walk import graph and transform source files.
    // For app router, generate client reference stubs for "use client" modules.
    let collected = if has_app {
        esm_transform::collect_source_modules_with_stubs(
            &entry_paths,
            &config.project_root,
            &known_specifiers,
            build_id,
        )?
    } else {
        esm_transform::collect_source_modules(
            &entry_paths,
            &config.project_root,
            &known_specifiers,
        )?
    };

    debug!(
        source_modules = collected.source_modules.len(),
        extra_deps = collected.extra_dep_imports.len(),
        "Source modules collected"
    );

    // Generate entry source
    let entry_specifier = "rex://entry".to_string();
    let entry_source = if let Some(app_scan) = &scan.app_scan {
        // ESM is the authority for ref IDs. Build webpack config from ESM-discovered
        // boundaries. Chunks are empty because:
        // - Server: modules are already loaded as ESM in V8
        // - SSR: modules are bundled in the SSR IIFE
        // - Client: hydration entry pre-loads from __REX_RSC_MODULE_MAP__
        let webpack_config = {
            let mut wpc = serde_json::Map::new();
            for boundary in &collected.client_boundaries {
                for (export, ref_id) in boundary.exports.iter().zip(boundary.ref_ids.iter()) {
                    wpc.insert(
                        ref_id.clone(),
                        serde_json::json!({ "id": ref_id, "name": export, "chunks": [] }),
                    );
                }
            }
            debug!(
                count = wpc.len(),
                "ESM: built webpack config from ESM boundaries"
            );
            serde_json::to_string(&serde_json::Value::Object(wpc)).unwrap_or_default()
        };

        // OXC-transform TypeScript runtimes to valid JS for V8
        let metadata_runtime_ts = include_str!("../../../runtime/server/metadata.ts");
        let flight_runtime_ts = include_str!("../../../runtime/rsc/flight.ts");
        let metadata_runtime_js =
            esm_transform::transform_to_esm(metadata_runtime_ts, "metadata.ts")?;
        let flight_runtime_js = esm_transform::transform_to_esm(flight_runtime_ts, "flight.ts")?;

        // Generate server action dispatch table from ESM-discovered "use server"
        // modules and inline extracted actions.
        let canonical_root = config
            .project_root
            .canonicalize()
            .unwrap_or_else(|_| config.project_root.clone());
        let has_actions =
            !collected.extracted_actions.is_empty() || !collected.server_action_modules.is_empty();
        let server_actions_js = if !has_actions {
            String::new()
        } else {
            let mut js = String::new();
            js.push_str("import { decodeReply, decodeAction, registerServerReference } from 'react-server-dom-webpack/server';\n");
            js.push_str("globalThis.__rex_decodeReply = decodeReply;\n");
            js.push_str("globalThis.__rex_decodeAction = decodeAction;\n");
            js.push_str(
                "globalThis.__rex_server_actions = globalThis.__rex_server_actions || {};\n",
            );

            // Module-level "use server" files
            for module in &collected.server_action_modules {
                debug!(
                    abs_path = %module.abs_path,
                    rel_path = %module.rel_path,
                    exports = ?module.exports,
                    "ESM: registering server action module"
                );
            }
            for (i, module) in collected.server_action_modules.iter().enumerate() {
                let import_var = format!("__sam_{i}");
                js.push_str(&format!(
                    "import * as {import_var} from '{}';\n",
                    module.abs_path,
                ));
                for export in &module.exports {
                    let action_id = rex_build::server_action_manifest::server_action_id(
                        &module.rel_path,
                        export,
                        build_id,
                    );
                    js.push_str(&format!(
                        "registerServerReference({import_var}.{export}, \"{action_id}\", \"{export}\");\n"
                    ));
                    js.push_str(&format!(
                        "globalThis.__rex_server_actions[\"{action_id}\"] = {import_var}.{export};\n"
                    ));
                }
            }

            // Inline extracted actions
            for (i, action) in collected.extracted_actions.iter().enumerate() {
                let abs_path = canonical_root.join(&action.rel_path);
                let abs_str = abs_path.to_string_lossy().replace('\\', "/");
                let import_var = format!("__sa_{i}");
                js.push_str(&format!(
                    "import {{ {} as {import_var} }} from '{abs_str}';\n",
                    action.action_name,
                ));
                js.push_str(&format!(
                    "globalThis.__rex_server_actions[\"{}\"] = {import_var};\n",
                    action.action_id,
                ));
            }

            js.push_str(
                "globalThis.__rex_server_action_manifest = globalThis.__rex_server_actions;\n",
            );
            js
        };

        let mut entry = rex_v8::esm_rsc_entry::generate_rsc_esm_entry(
            app_scan,
            &config.project_root,
            &webpack_config,
            &server_actions_js,
            &flight_runtime_js,
            &metadata_runtime_js,
        );

        // Append app-route handler runtime if app has API routes
        if !app_scan.api_routes.is_empty() {
            let app_route_runtime_ts = include_str!("../../../runtime/server/app-route-runtime.ts");
            let app_route_runtime_js =
                esm_transform::transform_to_esm(app_route_runtime_ts, "app-route-runtime.ts")?;
            entry.push_str("\n// --- App Route Runtime ---\n");
            entry.push_str(&app_route_runtime_js);
        }

        entry
    } else {
        let page_sources: Vec<(String, std::path::PathBuf)> = scan
            .routes
            .iter()
            .map(|r| (r.module_name(), r.abs_path.clone()))
            .collect();
        let api_sources: Vec<(String, std::path::PathBuf)> = scan
            .api_routes
            .iter()
            .map(|r| (r.module_name(), r.abs_path.clone()))
            .collect();
        let mcp_sources: Vec<(String, std::path::PathBuf)> = scan
            .mcp_tools
            .iter()
            .map(|t| (t.name.clone(), t.abs_path.clone()))
            .collect();
        let mcp_runtime_js = if mcp_sources.is_empty() {
            String::new()
        } else {
            let mcp_ts = include_str!("../../../runtime/server/mcp-runtime.ts");
            esm_transform::transform_to_esm(mcp_ts, "mcp-runtime.ts")?
        };
        rex_v8::esm_rsc_entry::generate_pages_esm_entry(
            &page_sources,
            &api_sources,
            &mcp_sources,
            SSR_RUNTIME,
            &mcp_runtime_js,
        )
    };

    // Bundle extra deps as native ESM using rolldown multi-entry bundling.
    // Each dep is a separate entry point; rolldown code-splits shared code into
    // chunks. All outputs are loaded directly as V8 ESM modules — no globalThis
    // intermediary, preserving class hierarchies and live bindings.
    let module_dirs = rex_build::resolve_modules_dirs(config)?;
    let mut extra_dep_modules = Vec::new();
    let mut dep_aliases: Vec<(String, String)> = Vec::new();
    if !collected.extra_dep_imports.is_empty() {
        // Externalize RSC-specific packages only. React itself is NOT externalized
        // because the pre-bundled React uses `react-server` conditions (missing
        // createContext, useState, etc.), while extra deps need the full React API
        // with standard conditions. Rolldown bundles a standard-conditions React
        // into the extra deps — this is the correct dual-instance architecture
        // (server React for RSC rendering, standard React for SSR/UI deps).
        let mut externals: Vec<String> = Vec::new();
        if has_app {
            externals.push("react-server-dom-webpack/server".to_string());
            externals.push("react-server-dom-webpack/client".to_string());
        }
        match rex_build::extra_dep_bundle::build_extra_deps_multi_entry(
            config,
            &collected.extra_dep_imports,
            &module_dirs,
            &externals,
        )
        .await
        {
            Ok(result) => {
                for (specifier, source) in result.modules {
                    debug!(
                        specifier = %specifier,
                        size = source.len(),
                        "Extra dep module loaded"
                    );
                    extra_dep_modules.push(EsmSourceModule { specifier, source });
                }
                dep_aliases = result.aliases;
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "Failed to bundle extra deps — using empty stubs"
                );
                for dep in &collected.extra_dep_imports {
                    extra_dep_modules.push(EsmSourceModule {
                        specifier: dep.specifier.clone(),
                        source: "export default {};".to_string(),
                    });
                }
            }
        }
    }

    // Add rex/* stub modules for framework imports.
    let mut all_dep_modules = dep_modules;
    all_dep_modules.extend(extra_dep_modules);
    let rex_stub = "export default function() { return null; }; export var Html = function() { return null; }; export var Head = function() { return null; }; export var Main = function() { return null; }; export var NextScript = function() { return null; };";
    for specifier in &["rex/head", "rex/link", "rex/image", "rex/document"] {
        all_dep_modules.push(EsmSourceModule {
            specifier: specifier.to_string(),
            source: rex_stub.to_string(),
        });
    }

    // Register all Node.js built-in polyfills and next/* stubs as ESM modules.
    // In the IIFE path, rolldown resolves these via aliases. In the ESM path,
    // we OXC-transform the TypeScript polyfill files and register them directly.
    let runtime_dir = rex_build::build_utils::runtime_server_dir()?;
    let polyfill_aliases = rex_build::build_utils::node_polyfill_aliases(&runtime_dir);
    for (specifier, targets) in &polyfill_aliases {
        if let Some(Some(target_path)) = targets.first() {
            let target = std::path::Path::new(target_path);
            if target.exists() {
                let ts_source = std::fs::read_to_string(target).unwrap_or_default();
                let filename = target
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let js = esm_transform::transform_to_esm(&ts_source, &filename)
                    .unwrap_or_else(|_| "export default {};".to_string());
                all_dep_modules.push(EsmSourceModule {
                    specifier: specifier.clone(),
                    source: js,
                });
            }
        }
    }

    // Add rex/actions stub from runtime/server/actions.ts
    {
        let actions_path = runtime_dir.join("actions.ts");
        if actions_path.exists() {
            let actions_ts = std::fs::read_to_string(&actions_path).unwrap_or_default();
            let actions_js = esm_transform::transform_to_esm(&actions_ts, "actions.ts")
                .unwrap_or_else(|_| rex_stub.to_string());
            all_dep_modules.push(EsmSourceModule {
                specifier: "rex/actions".to_string(),
                source: actions_js,
            });
        }
    }

    // Load ESM modules into all isolates.
    let polyfills_arc = Arc::new(dep_bundles.polyfills);
    let dep_modules_arc = Arc::new(all_dep_modules.clone());
    let source_modules_arc = Arc::new(collected.source_modules.clone());
    let entry_spec_arc = Arc::new(entry_specifier.clone());
    let entry_src_arc = Arc::new(entry_source.clone());

    let aliases_arc = Arc::new(dep_aliases);
    pool.load_esm_modules_all(
        polyfills_arc,
        dep_modules_arc.clone(),
        source_modules_arc,
        entry_spec_arc,
        entry_src_arc,
        aliases_arc.clone(),
    )
    .await?;

    debug!("ESM modules loaded into V8");

    Ok(EsmState {
        dep_modules: dep_modules_arc,
        source_modules: collected.source_modules,
        entry_specifier,
        entry_source,
        dep_aliases: aliases_arc,
    })
}
