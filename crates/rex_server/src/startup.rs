//! V8 isolate pool startup strategies.
//!
//! - **ESM** (pages router): Native V8 ESM modules with dep IIFEs + OXC-transformed source.
//!   Enables fast HMR (~100ms vs ~3500ms for full rolldown rebuild).
//! - **IIFE** (app router): Monolithic rolldown bundles. RSC requires dual React instances
//!   (react-server + standard conditions) which ESM can't provide with one module namespace.

use crate::state::EsmState;
use anyhow::Result;
use rex_core::RexConfig;
use rex_router::ScanResult;
use rex_v8::IsolatePool;
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

/// IIFE startup: read rolldown-built server bundle from disk and load into isolate pool.
///
/// Used for app router (RSC needs dual React instances) and `rex start` (production).
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

/// ESM startup: pre-bundle deps, OXC-transform source files, load via V8 native ESM.
///
/// Used for pages router projects. Each source file is individually transformed
/// and loaded as a V8 ESM module, enabling per-file HMR invalidation.
pub async fn esm_startup(
    config: &RexConfig,
    scan: &ScanResult,
    _build_result: &rex_build::bundler::BuildResult,
    pool_size: usize,
    project_root_str: &str,
) -> Result<(IsolatePool, EsmState)> {
    use rex_build::esm_transform;
    use rex_build::server_bundle::SSR_RUNTIME;

    let module_dirs = rex_build::resolve_modules_dirs(config)?;

    // Build pages dep IIFE (standard React + renderToString)
    let iife_js = rex_build::server_dep_bundle::build_pages_dep_iife(config, &module_dirs).await?;
    let synthetic_modules = esm_transform::pages_synthetic_modules();
    let known_specifiers = esm_transform::pages_known_specifiers();

    // Collect entry paths from pages routes
    let entry_paths: Vec<std::path::PathBuf> =
        scan.routes.iter().map(|r| r.abs_path.clone()).collect();

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

    // Generate pages ESM entry
    let entry_specifier = "rex://entry".to_string();
    let page_sources: Vec<(String, std::path::PathBuf)> = scan
        .routes
        .iter()
        .map(|r| (r.module_name(), r.abs_path.clone()))
        .collect();
    let entry_source = rex_v8::esm_rsc_entry::generate_pages_esm_entry(&page_sources, SSR_RUNTIME);

    // Add rex/* synthetic modules (server-side stubs for framework components)
    let mut all_synthetic = synthetic_modules;
    let rex_stub = "({ default: function() { return null; } })";
    for specifier in &["rex/head", "rex/link", "rex/image", "rex/document"] {
        all_synthetic.push(rex_v8::SyntheticModuleDef {
            specifier: specifier.to_string(),
            export_names: vec![
                "default".into(),
                "Html".into(),
                "Head".into(),
                "Main".into(),
                "NextScript".into(),
            ],
            globals_expr: rex_stub.to_string(),
        });
    }

    // Build dep config
    let dep_config = Arc::new(esm_transform::build_dep_config(iife_js, all_synthetic));

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

    debug!("ESM modules loaded into V8");

    let esm_state = EsmState {
        dep_config,
        source_modules: collected.source_modules,
        entry_specifier,
        entry_source,
    };

    Ok((pool, esm_state))
}
