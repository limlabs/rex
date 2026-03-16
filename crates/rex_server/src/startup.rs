//! V8 isolate pool startup strategies.
//!
//! Two paths for initializing the V8 isolate pool:
//! - **IIFE**: Monolithic rolldown bundle evaluated as a script (fallback)
//! - **ESM**: Native V8 ESM modules with dep IIFEs + OXC-transformed source (fast HMR)

use crate::state::EsmState;
use anyhow::Result;
use rex_core::RexConfig;
use rex_router::ScanResult;
use rex_v8::IsolatePool;
use std::sync::Arc;
use tracing::debug;

/// Stub render functions for ESM bootstrap. These are overwritten by
/// `load_esm_modules()` once the real entry module is evaluated.
const STUB_FUNCTIONS: &str = r#"
globalThis.__rex_pages = {};
globalThis.__rex_render_page = function() { return JSON.stringify({ body: '', head: '' }); };
globalThis.__rex_get_server_side_props = function() { return JSON.stringify({ props: {} }); };
globalThis.__rex_get_static_props = function() { return JSON.stringify({ props: {} }); };
"#;

/// IIFE startup: read server bundle from disk and load into isolate pool.
pub fn iife_startup(
    build_result: &rex_build::bundler::BuildResult,
    pool_size: usize,
    project_root_str: &str,
) -> Result<IsolatePool> {
    let mut server_bundle = std::fs::read_to_string(&build_result.server_bundle_path)?;

    if let Some(rsc_path) = &build_result.manifest.rsc_server_bundle {
        let rsc_bundle = std::fs::read_to_string(rsc_path)?;
        server_bundle.push_str("\n;\n");
        server_bundle.push_str(&rsc_bundle);
    }
    if let Some(ssr_path) = &build_result.manifest.rsc_ssr_bundle {
        let ssr_bundle = std::fs::read_to_string(ssr_path)?;
        server_bundle.push_str("\n;\n");
        server_bundle.push_str(&ssr_bundle);
    }

    debug!(pool_size, "Creating V8 isolate pool (IIFE)");
    IsolatePool::new(
        pool_size,
        Arc::new(server_bundle),
        Some(Arc::new(project_root_str.to_string())),
    )
}

/// ESM startup: pre-bundle deps, transform source files, load via ESM modules.
pub async fn try_esm_startup(
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
    let has_pages = !scan.routes.is_empty() || scan.app.is_some();

    // Build dep IIFE(s)
    let dep_bundles =
        rex_build::server_dep_bundle::build_server_dep_bundles(config, has_app, &module_dirs)
            .await?;

    // Determine which dep IIFE and synthetic modules to use
    let (iife_js, synthetic_modules, known_specifiers) = if has_app {
        (
            dep_bundles
                .flight_iife
                .unwrap_or(dep_bundles.pages_iife.clone()),
            esm_transform::flight_synthetic_modules(),
            esm_transform::flight_known_specifiers(),
        )
    } else {
        (
            dep_bundles.pages_iife,
            esm_transform::pages_synthetic_modules(),
            esm_transform::pages_known_specifiers(),
        )
    };

    // Collect entry paths for import graph walking
    let mut entry_paths: Vec<std::path::PathBuf> = Vec::new();
    if has_pages {
        for route in &scan.routes {
            entry_paths.push(route.abs_path.clone());
        }
    }
    if let Some(app_scan) = &scan.app_scan {
        for route in &app_scan.routes {
            entry_paths.push(route.page_path.clone());
            entry_paths.extend(route.layout_chain.iter().cloned());
        }
    }

    // Walk import graph and transform source files
    let collected = esm_transform::collect_source_modules(
        &entry_paths,
        &config.project_root,
        &known_specifiers,
    )?;

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
            "", // flight_runtime_js (provided by dep IIFE)
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

    // Build dep config
    let dep_config = Arc::new(esm_transform::build_dep_config(iife_js, synthetic_modules));

    // Create pool with V8 polyfills + stub functions as bootstrap
    let bootstrap = format!("{}\n{}\n", rex_build::V8_POLYFILLS, STUB_FUNCTIONS);

    debug!(pool_size, "Creating V8 isolate pool (ESM)");
    let pool = IsolatePool::new(
        pool_size,
        Arc::new(bootstrap),
        Some(Arc::new(project_root_str.to_string())),
    )?;

    // Load ESM modules into all isolates
    let source_modules_arc = Arc::new(collected.source_modules.clone());
    let entry_spec_arc = Arc::new(entry_specifier.clone());
    let entry_src_arc = Arc::new(entry_source.clone());

    pool.load_esm_modules_all(
        dep_config.clone(),
        source_modules_arc,
        entry_spec_arc,
        entry_src_arc,
    )
    .await?;

    let esm_state = EsmState {
        dep_config,
        source_modules: collected.source_modules,
        entry_specifier,
        entry_source,
    };

    Ok((pool, esm_state))
}
