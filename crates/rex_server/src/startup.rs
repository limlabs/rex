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

    // Collect entry paths for import graph walking
    let mut entry_paths: Vec<std::path::PathBuf> = Vec::new();
    for route in &scan.routes {
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
        rex_v8::esm_rsc_entry::generate_pages_esm_entry(&page_sources, SSR_RUNTIME)
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

/// Build RSC client bundles in the background and update HotState when done.
///
/// This is spawned after the server starts so it doesn't block startup.
/// Pages rendered before completion will lack client hydration scripts.
pub fn spawn_deferred_client_build(
    state: Arc<crate::state::AppState>,
    config: RexConfig,
    scan: ScanResult,
    build_id: String,
) {
    tokio::spawn(async move {
        use rex_build::rsc_build_config::RscBuildContext;
        use rex_build::rsc_graph::analyze_module_graph;

        let app_scan = match &scan.app_scan {
            Some(s) => s,
            None => return,
        };

        let module_dirs = match rex_build::resolve_modules_dirs(&config) {
            Ok(d) => d,
            Err(e) => {
                tracing::error!("Deferred client build: failed to resolve module dirs: {e:#}");
                return;
            }
        };

        let define_env = if config.dev {
            "\"development\""
        } else {
            "\"production\""
        };
        let define = vec![("process.env.NODE_ENV".to_string(), define_env.to_string())];

        let ctx = RscBuildContext::new(&config, &build_id, &define, &module_dirs);

        // Collect entries for module graph
        let mut entries: Vec<std::path::PathBuf> = Vec::new();
        if let Some(root_layout) = &app_scan.root_layout {
            entries.push(root_layout.clone());
        }
        for route in &app_scan.routes {
            entries.push(route.page_path.clone());
            entries.extend(route.layout_chain.iter().cloned());
        }
        entries.sort();
        entries.dedup();

        let graph = match analyze_module_graph(&entries, &config.project_root) {
            Ok(g) => g,
            Err(e) => {
                tracing::error!("Deferred client build: module graph analysis failed: {e:#}");
                return;
            }
        };

        let client_dir = config.client_build_dir().join("rsc");
        if let Err(e) = std::fs::create_dir_all(&client_dir) {
            tracing::error!("Deferred client build: failed to create client dir: {e:#}");
            return;
        }

        let server_action_modules = graph.server_action_modules();
        let mut client_manifest = rex_core::client_manifest::ClientReferenceManifest::new();

        let client_chunks = match rex_build::rsc_client_bundle::build_rsc_client_bundles(
            &ctx,
            &graph,
            &client_dir,
            &mut client_manifest,
            &server_action_modules,
        )
        .await
        {
            Ok(chunks) => chunks,
            Err(e) => {
                tracing::error!("Deferred client build failed: {e:#}");
                return;
            }
        };

        debug!(
            chunks = client_chunks.len(),
            "Deferred RSC client bundles built"
        );

        // Update HotState manifest with real client_chunks
        if let Ok(mut guard) = state.hot.write() {
            let mut hot = (**guard).clone();
            for (_pattern, assets) in hot.manifest.app_routes.iter_mut() {
                assets.client_chunks = client_chunks.clone();
            }
            hot.manifest.client_reference_manifest = Some(client_manifest);
            *guard = Arc::new(hot);
        }
    });
}
