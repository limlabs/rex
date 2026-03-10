//! On-demand project compiler for live mode.
//!
//! Compiles an entire project (scan + bundle) when requested.
//! Reuses rex_build's rolldown infrastructure.

use crate::cache::{BuildCache, CachedBuild};
use crate::source::SourceProvider;
use anyhow::{Context, Result};
use rex_core::{ProjectConfig, RexConfig};
use rex_router::scan_project;
use std::sync::Arc;
use std::time::SystemTime;
use tracing::{debug, info};

/// Compile a project from source, or return a cached build if fresh.
///
/// Uses timestamp-based invalidation: walks the pages directory and checks
/// if any file is newer than the cached build.
pub async fn compile_project(
    source: &dyn SourceProvider,
    cache: &BuildCache,
    _pool_size: usize,
) -> Result<Arc<CachedBuild>> {
    let root = source.root().to_path_buf();

    // Check if we have a cached build that's still fresh
    if let Some(cached) = cache.get() {
        let latest_mtime = latest_source_mtime(&root)?;
        if latest_mtime <= cached.source_mtime {
            debug!("Build cache hit for {}", root.display());
            return Ok(cached);
        }
        info!("Source changed, recompiling {}", root.display());
    } else {
        info!("First compilation for {}", root.display());
    }

    // Determine pages/app directory
    let pages_dir = if root.join("src/pages").is_dir() {
        root.join("src/pages")
    } else {
        root.join("pages")
    };

    let app_dir = if root.join("src/app").is_dir() {
        root.join("src/app")
    } else {
        root.join("app")
    };

    // Scan routes
    let scan =
        scan_project(&root, &pages_dir, &app_dir).context("Failed to scan project routes")?;

    if scan.routes.is_empty() && scan.app_scan.is_none() {
        anyhow::bail!(
            "No pages found in {} — expected pages/ or app/ directory",
            root.display()
        );
    }

    // Build config
    let config = RexConfig {
        project_root: root.clone(),
        pages_dir,
        app_dir,
        output_dir: root.join(".rex"),
        port: 0,
        dev: false,
    };

    let project_config = ProjectConfig::load(&root).unwrap_or_default();

    // Build bundles
    let build_result = rex_build::build_bundles(&config, &scan, &project_config)
        .await
        .context("Failed to build project bundles")?;

    // Read the server bundle
    let server_bundle_js = std::fs::read_to_string(&build_result.server_bundle_path)
        .context("Failed to read server bundle")?;

    let source_mtime = latest_source_mtime(&root)?;
    let build_number = cache.next_build_number();

    let cached = CachedBuild {
        server_bundle_js: Arc::new(server_bundle_js),
        build_id: build_result.build_id,
        manifest: build_result.manifest,
        scan,
        source_mtime,
        build_number,
    };

    cache.set(cached.clone());

    info!(
        build_number,
        build_id = %cached.build_id,
        routes = cached.scan.routes.len(),
        "Project compiled: {}",
        root.display()
    );

    // Return the cached version (wrapped in Arc)
    Ok(cache.get().expect("just set"))
}

/// Walk the pages/app directories and find the most recent mtime.
/// Public so `project.rs` can call it for timestamp-based cache checks.
pub fn latest_source_mtime_pub(root: &std::path::Path) -> Result<SystemTime> {
    latest_source_mtime(root)
}

fn latest_source_mtime(root: &std::path::Path) -> Result<SystemTime> {
    let mut latest = SystemTime::UNIX_EPOCH;

    for dir_name in &["pages", "src/pages", "app", "src/app"] {
        let dir = root.join(dir_name);
        if dir.is_dir() {
            walk_mtime(&dir, &mut latest)?;
        }
    }

    // Also check _app, _document at root level
    for special in &[
        "_app.tsx",
        "_app.ts",
        "_app.jsx",
        "_app.js",
        "_document.tsx",
        "_document.ts",
        "_document.jsx",
        "_document.js",
    ] {
        if let Ok(meta) = std::fs::metadata(root.join("pages").join(special)) {
            if let Ok(mtime) = meta.modified() {
                if mtime > latest {
                    latest = mtime;
                }
            }
        }
    }

    Ok(latest)
}

fn walk_mtime(dir: &std::path::Path, latest: &mut SystemTime) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk_mtime(&path, latest)?;
        } else {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if matches!(ext, "tsx" | "ts" | "jsx" | "js" | "css" | "mdx") {
                if let Ok(mtime) = entry.metadata()?.modified() {
                    if mtime > *latest {
                        *latest = mtime;
                    }
                }
            }
        }
    }
    Ok(())
}
