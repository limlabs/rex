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
const STUB_FUNCTIONS: &str = r#"
globalThis.__rex_pages = {};
globalThis.__rex_render_page = function() { return JSON.stringify({ body: '', head: '' }); };
globalThis.__rex_get_server_side_props = function() { return JSON.stringify({ props: {} }); };
globalThis.__rex_get_static_props = function() { return JSON.stringify({ props: {} }); };
"#;

/// ESM startup: pre-bundle deps as ESM, OXC-transform source files, load into V8.
pub async fn esm_startup(
    config: &RexConfig,
    scan: &ScanResult,
    build_result: &rex_build::bundler::BuildResult,
    pool_size: usize,
    project_root_str: &str,
) -> Result<(IsolatePool, EsmState)> {
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
    }

    debug!(entries = entry_paths.len(), "Walking import graph for ESM");

    // Walk import graph and transform source files
    let collected = esm_transform::collect_source_modules(
        &entry_paths,
        &config.project_root,
        &known_specifiers,
    )?;

    debug!(
        source_modules = collected.source_modules.len(),
        extra_deps = collected.extra_dep_imports.len(),
        "Source modules collected"
    );

    // Generate entry source
    let entry_specifier = "rex://entry".to_string();
    let entry_source = if let Some(app_scan) = &scan.app_scan {
        let webpack_config = build_result
            .manifest
            .client_reference_manifest
            .as_ref()
            .map(|m| serde_json::to_string(m).unwrap_or_default())
            .unwrap_or_else(|| "{}".to_string());

        rex_v8::esm_rsc_entry::generate_rsc_esm_entry(
            app_scan,
            &config.project_root,
            &webpack_config,
            "", // server_actions_js
            "", // flight_runtime_js
            "", // metadata_runtime_js
        )
    } else {
        let page_sources: Vec<(String, std::path::PathBuf)> = scan
            .routes
            .iter()
            .map(|r| (r.module_name(), r.abs_path.clone()))
            .collect();
        rex_v8::esm_rsc_entry::generate_pages_esm_entry(&page_sources, SSR_RUNTIME)
    };

    // Add rex/* stub modules for framework imports (rex/head, rex/link, etc.)
    let mut all_dep_modules = dep_modules;
    let rex_stub = "export default function() { return null; }; export var Html = function() { return null; }; export var Head = function() { return null; }; export var Main = function() { return null; }; export var NextScript = function() { return null; };";
    for specifier in &["rex/head", "rex/link", "rex/image", "rex/document"] {
        all_dep_modules.push(EsmSourceModule {
            specifier: specifier.to_string(),
            source: rex_stub.to_string(),
        });
    }

    // Create pool with minimal bootstrap (just stub functions for SsrIsolate::new)
    let bootstrap = STUB_FUNCTIONS.to_string();

    debug!(pool_size, "Creating V8 isolate pool (ESM)");
    let pool = IsolatePool::new(
        pool_size,
        Arc::new(bootstrap),
        Some(Arc::new(project_root_str.to_string())),
    )?;

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

    let esm_state = EsmState {
        dep_modules: dep_modules_arc,
        source_modules: collected.source_modules,
        entry_specifier,
        entry_source,
    };

    Ok((pool, esm_state))
}
