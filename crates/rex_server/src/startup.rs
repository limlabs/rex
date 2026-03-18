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

        rex_v8::esm_rsc_entry::generate_rsc_esm_entry(
            app_scan,
            &config.project_root,
            &webpack_config,
            "", // server_actions_js — wired separately when manifest available
            &flight_runtime_js,
            &metadata_runtime_js,
        )
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
        rex_v8::esm_rsc_entry::generate_pages_esm_entry(&page_sources, &api_sources, SSR_RUNTIME)
    };

    // Add rex/* and next/* stub modules for framework imports.
    // These are server-side stubs that return null components or no-op functions.
    let mut all_dep_modules = dep_modules;
    let rex_stub = "export default function() { return null; }; export var Html = function() { return null; }; export var Head = function() { return null; }; export var Main = function() { return null; }; export var NextScript = function() { return null; };";
    for specifier in &["rex/head", "rex/link", "rex/image", "rex/document"] {
        all_dep_modules.push(EsmSourceModule {
            specifier: specifier.to_string(),
            source: rex_stub.to_string(),
        });
    }

    // Add next/* compatibility stubs from runtime/server/ directory.
    // OXC-transform TypeScript stubs to valid JS for V8.
    let runtime_dir = rex_build::build_utils::runtime_server_dir()?;
    let next_stubs: &[(&str, &str)] = &[
        ("next/link", "next-link.ts"),
        ("next/image", "next-image.ts"),
        ("next/head", "head.ts"),
        ("next/navigation", "next-navigation.ts"),
        ("next/headers", "next-headers.ts"),
        ("next/cache", "next-cache.ts"),
        ("next/server", "next-server.ts"),
        ("next/font/google", "next-font.ts"),
        ("next/font/local", "next-font.ts"),
        ("next/dynamic", "next-dynamic.ts"),
        ("next/router", "next-router.ts"),
    ];
    for (specifier, filename) in next_stubs {
        let stub_path = runtime_dir.join(filename);
        if stub_path.exists() {
            let stub_ts = std::fs::read_to_string(&stub_path).unwrap_or_default();
            let stub_js = esm_transform::transform_to_esm(&stub_ts, filename)
                .unwrap_or_else(|_| "export default function() { return null; };".to_string());
            all_dep_modules.push(EsmSourceModule {
                specifier: specifier.to_string(),
                source: stub_js,
            });
        }
    }

    // Load ESM modules into all isolates
    let polyfills_arc = Arc::new(dep_bundles.polyfills);
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
