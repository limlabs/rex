use anyhow::Result;
use rex_build::build_bundles;
use rex_core::{ProjectConfig, RexConfig};
use rex_router::scan_project;
use rex_server::export::{export_site, validate_exportability, ExportConfig, ExportContext};
use rex_server::state::HotState;
use rex_v8::{init_v8, IsolatePool};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::debug;

/// Run the `rex export` command: build + pre-render all static pages to disk.
pub async fn cmd_export(
    root: PathBuf,
    output: Option<PathBuf>,
    force: bool,
    base_path: String,
    html_extensions: bool,
) -> Result<()> {
    let root = std::fs::canonicalize(&root)?;
    let config = RexConfig::new(root);
    config.validate()?;

    let start = std::time::Instant::now();

    // 1. Scan routes
    let scan = scan_project(&config.project_root, &config.pages_dir, &config.app_dir)?;
    debug!(routes = scan.routes.len(), "Routes scanned");

    // 2. Build bundles
    let project_config = ProjectConfig::load(&config.project_root)?;
    let mut build_result = build_bundles(&config, &scan, &project_config).await?;
    debug!(build_id = %build_result.build_id, "Build complete");

    // 3. Validate exportability
    let warnings = validate_exportability(&build_result.manifest, force)?;
    for w in &warnings {
        eprintln!("  \x1b[33m⚠\x1b[0m {w}");
    }

    // 4. Initialize V8 + isolate pool
    init_v8();

    let mut server_bundle = std::fs::read_to_string(&build_result.server_bundle_path)?;

    // Append RSC bundles if present
    if let Some(rsc_path) = &build_result.manifest.rsc_server_bundle {
        if let Ok(rsc_bundle) = std::fs::read_to_string(rsc_path) {
            server_bundle.push_str("\n;\n");
            server_bundle.push_str(&rsc_bundle);
        }
    }
    if let Some(ssr_path) = &build_result.manifest.rsc_ssr_bundle {
        if let Ok(ssr_bundle) = std::fs::read_to_string(ssr_path) {
            server_bundle.push_str("\n;\n");
            server_bundle.push_str(&ssr_bundle);
        }
    }

    let pool_size = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(8);
    let project_root_str = config.project_root.to_string_lossy().to_string();
    let pool = IsolatePool::new(
        pool_size,
        Arc::new(server_bundle),
        Some(Arc::new(project_root_str)),
    )?;

    // 5. Compute manifest JSON and document descriptor
    let manifest_json =
        HotState::compute_manifest_json(&build_result.build_id, &build_result.manifest);

    let document_descriptor = if scan.document.is_some() {
        rex_server::handlers::compute_document_descriptor(&pool).await
    } else {
        None
    };

    // 6. Run the export
    let output_dir = output.unwrap_or_else(|| config.output_dir.join("export"));
    // Clean previous export
    if output_dir.exists() {
        std::fs::remove_dir_all(&output_dir)?;
    }

    // Normalize base_path: strip trailing slash, ensure leading slash if non-empty
    let base_path = if base_path.is_empty() {
        String::new()
    } else {
        let bp = base_path.trim_end_matches('/');
        if bp.starts_with('/') {
            bp.to_string()
        } else {
            format!("/{bp}")
        }
    };

    let export_config = ExportConfig {
        output_dir: output_dir.clone(),
        force,
        base_path,
        html_extensions,
    };

    let mut ctx = ExportContext {
        pool: &pool,
        manifest: &mut build_result.manifest,
        routes: &scan.routes,
        manifest_json: &manifest_json,
        doc_descriptor: document_descriptor.as_ref(),
        client_build_dir: &config.client_build_dir(),
        project_root: &config.project_root,
    };

    let result = export_site(&mut ctx, &export_config).await?;

    let elapsed = start.elapsed();

    // 7. Print summary
    eprintln!();
    for path in &result.pages_exported_list {
        eprintln!("    \x1b[32m\u{25cb}\x1b[0m \x1b[2m{path}\x1b[0m");
    }
    eprintln!();
    eprintln!(
        "  \x1b[32;1m\u{2713} Exported {} pages in {:.1}s\x1b[0m",
        result.pages_exported,
        elapsed.as_secs_f64()
    );
    eprintln!("  \x1b[2m\u{2192} {}\x1b[0m", output_dir.display());

    if !result.pages_skipped.is_empty() {
        eprintln!();
        for (pattern, reason) in &result.pages_skipped {
            eprintln!("  \x1b[2m\u{25cb} {pattern} (skipped: {reason})\x1b[0m");
        }
    }

    eprintln!();
    Ok(())
}
