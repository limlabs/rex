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
    let esm_state = esm_load_modules(config, scan, build_id, &pool).await?;
    Ok((pool, esm_state))
}

/// Load ESM modules into an existing isolate pool.
///
/// Builds dep ESM modules, transforms source files, generates entry,
/// and loads everything into the pool's V8 isolates.
pub async fn esm_load_modules(
    config: &RexConfig,
    scan: &ScanResult,
    build_id: &str,
    pool: &IsolatePool,
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
        // Build webpack config from ESM-discovered client boundaries (not the IIFE
        // manifest, which may use different path resolution and produce mismatched IDs).
        let webpack_config = {
            let mut config = serde_json::Map::new();
            for boundary in &collected.client_boundaries {
                for (export, ref_id) in boundary.exports.iter().zip(boundary.ref_ids.iter()) {
                    config.insert(
                        ref_id.clone(),
                        serde_json::json!({
                            "id": ref_id,
                            "name": export,
                            "chunks": []
                        }),
                    );
                }
            }
            debug!(
                manifest_keys = ?config.keys().collect::<Vec<_>>(),
                "Webpack bundler config for RSC (from ESM stubs)"
            );
            serde_json::to_string(&serde_json::Value::Object(config)).unwrap_or_default()
        };

        // OXC-transform TypeScript runtimes to valid JS for V8
        let metadata_runtime_ts = include_str!("../../../runtime/server/metadata.ts");
        let flight_runtime_ts = include_str!("../../../runtime/rsc/flight.ts");
        let metadata_runtime_js =
            esm_transform::transform_to_esm(metadata_runtime_ts, "metadata.ts")?;
        let flight_runtime_js = esm_transform::transform_to_esm(flight_runtime_ts, "flight.ts")?;

        let mut entry = rex_v8::esm_rsc_entry::generate_rsc_esm_entry(
            app_scan,
            &config.project_root,
            &webpack_config,
            "", // server_actions_js — wired separately when manifest available
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

    // Bundle ALL extra deps in a single rolldown build so cross-dep imports
    // resolve correctly. Each dep becomes a separate entry point; rolldown handles
    // shared chunks and proper import resolution between them.
    let module_dirs = rex_build::resolve_modules_dirs(config)?;
    let mut extra_dep_modules = Vec::new();
    let extra_polyfills = String::new();
    if !collected.extra_dep_imports.is_empty() {
        // Build a single entry that re-exports all deps under their specifier names.
        // Each dep gets its own entry to preserve its specifier as the module name.
        let mut combined_entry = String::new();
        for dep in &collected.extra_dep_imports {
            let safe_name = dep.specifier.replace(['/', '-', '.', '@'], "_");
            combined_entry.push_str(&format!(
                "import * as {safe_name} from '{}';\nglobalThis.__rex_dep_{safe_name} = {safe_name};\n",
                dep.specifier
            ));
        }
        match rex_build::extra_dep_bundle::build_dep_esm_with_chunks(
            config,
            &combined_entry,
            "__all_extra_deps",
            &["default", "import", "module"],
            &module_dirs,
        )
        .await
        {
            Ok(chunks) => {
                for (specifier, source) in chunks {
                    debug!(
                        specifier = %specifier,
                        size = source.len(),
                        "Extra dep chunk loaded"
                    );
                    extra_dep_modules.push(EsmSourceModule { specifier, source });
                }
                // Create ESM wrappers that import the combined module (to force
                // evaluation) then re-export from globalThis slots
                for dep in &collected.extra_dep_imports {
                    let safe_name = dep.specifier.replace(['/', '-', '.', '@'], "_");
                    let mut wrapper = String::new();
                    // Import combined module to ensure it's evaluated before we read globals
                    wrapper.push_str("import '__all_extra_deps';\n");
                    wrapper.push_str(&format!(
                        "var __d = globalThis.__rex_dep_{safe_name} || {{}};\n"
                    ));
                    wrapper.push_str("export default __d.default || __d;\n");
                    for name in &dep.named_exports {
                        wrapper.push_str(&format!("export var {name} = __d.{name};\n"));
                    }
                    extra_dep_modules.push(EsmSourceModule {
                        specifier: dep.specifier.clone(),
                        source: wrapper,
                    });
                }
            }
            Err(e) => {
                debug!(
                    error = %e,
                    "Failed to bundle extra deps — using stubs"
                );
                for dep in &collected.extra_dep_imports {
                    let proxy_stub = "export default new Proxy(function(){return null},{get:function(_,p){if(p==='then')return undefined;return function(){return null}}});\n";
                    extra_dep_modules.push(EsmSourceModule {
                        specifier: dep.specifier.clone(),
                        source: proxy_stub.to_string(),
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
    // Append extra dep IIFE (if any) to polyfills — evaluated as script before ESM.
    let mut all_polyfills = dep_bundles.polyfills;
    if !extra_polyfills.is_empty() {
        all_polyfills.push('\n');
        all_polyfills.push_str(&extra_polyfills);
    }
    let polyfills_arc = Arc::new(all_polyfills);
    let dep_modules_arc = Arc::new(all_dep_modules.clone());
    let source_modules_arc = Arc::new(collected.source_modules.clone());
    let entry_spec_arc = Arc::new(entry_specifier.clone());
    let entry_src_arc = Arc::new(entry_source.clone());

    pool.load_esm_modules_all(
        polyfills_arc,
        dep_modules_arc.clone(),
        source_modules_arc,
        entry_spec_arc,
        entry_src_arc,
    )
    .await?;

    debug!("ESM modules loaded into V8");

    Ok(EsmState {
        dep_modules: dep_modules_arc,
        source_modules: collected.source_modules,
        entry_specifier,
        entry_source,
    })
}
