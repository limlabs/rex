use crate::hmr::HmrBroadcast;
use crate::watcher::{FileEvent, FileEventKind};
use anyhow::Result;
use rex_build::build_bundles;
use rex_core::RexConfig;
use rex_router::scan_pages;
use rex_v8::IsolatePool;
use std::sync::Arc;
use tracing::info;

/// Handle a file change event: rebuild and notify HMR clients
pub async fn handle_file_event(
    event: FileEvent,
    config: &RexConfig,
    pool: &IsolatePool,
    hmr: &HmrBroadcast,
) -> Result<()> {
    info!(path = %event.path.display(), kind = ?event.kind, "Processing file change");

    match event.kind {
        FileEventKind::PageModified => {
            // Incremental: rescan, rebuild, reload isolates
            let scan = scan_pages(&config.pages_dir)?;
            let build_result = build_bundles(config, &scan)?;

            // Read the new server bundle
            let bundle_js = std::fs::read_to_string(&build_result.server_bundle_path)?;
            let bundle_arc = Arc::new(bundle_js);

            // Reload all V8 isolates
            pool.reload_all(bundle_arc).await?;

            // Notify HMR clients
            let rel_path = event
                .path
                .strip_prefix(&config.pages_dir)
                .unwrap_or(&event.path);
            hmr.send_update(&rel_path.to_string_lossy());

            info!("Hot reload complete");
        }
        FileEventKind::PageAdded | FileEventKind::PageRemoved => {
            // Full rebuild needed for route changes
            let scan = scan_pages(&config.pages_dir)?;
            let build_result = build_bundles(config, &scan)?;

            let bundle_js = std::fs::read_to_string(&build_result.server_bundle_path)?;
            let bundle_arc = Arc::new(bundle_js);

            pool.reload_all(bundle_arc).await?;

            // Signal full reload to clients
            hmr.send_full_reload();

            info!("Full rebuild complete (route added/removed)");
        }
        FileEventKind::ConfigChanged => {
            hmr.send_full_reload();
        }
    }

    Ok(())
}
