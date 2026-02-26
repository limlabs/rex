use crate::hmr::HmrBroadcast;
use crate::watcher::{FileEvent, FileEventKind};
use anyhow::Result;
use rex_build::build_bundles;
use rex_core::RexConfig;
use rex_router::{scan_pages, RouteTrie};
use rex_server::handlers::AppState;
use std::sync::Arc;
use tracing::debug;

/// Handle a file change event: rebuild, reload isolates, update state, notify HMR clients
pub async fn handle_file_event(
    event: FileEvent,
    config: &RexConfig,
    state: &Arc<AppState>,
    hmr: &HmrBroadcast,
) -> Result<()> {
    debug!(path = %event.path.display(), kind = ?event.kind, "Processing file change");

    match event.kind {
        FileEventKind::PageModified => {
            // Rescan, rebuild, reload isolates, update hot state
            let scan = scan_pages(&config.pages_dir)?;
            let build_result = build_bundles(config, &scan).await?;

            // Read the new server bundle
            let bundle_js = std::fs::read_to_string(&build_result.server_bundle_path)?;
            let bundle_arc = Arc::new(bundle_js);

            // Reload all V8 isolates
            state.isolate_pool.reload_all(bundle_arc).await?;

            // Update hot state with new manifest and build_id
            {
                let mut hot = state.hot.write().unwrap();
                hot.manifest = build_result.manifest;
                hot.build_id = build_result.build_id;
            }

            // Notify HMR clients
            let rel_path = event
                .path
                .strip_prefix(&config.pages_dir)
                .unwrap_or(&event.path);
            hmr.send_update(&rel_path.to_string_lossy());

            debug!("Hot reload complete");
        }
        FileEventKind::PageAdded | FileEventKind::PageRemoved => {
            // Full rebuild: routes changed, need new trie + manifest
            let scan = scan_pages(&config.pages_dir)?;
            let build_result = build_bundles(config, &scan).await?;

            let bundle_js = std::fs::read_to_string(&build_result.server_bundle_path)?;
            let bundle_arc = Arc::new(bundle_js);

            state.isolate_pool.reload_all(bundle_arc).await?;

            // Update all hot state: route tries, manifest, build_id, feature flags
            {
                let mut hot = state.hot.write().unwrap();
                hot.route_trie = RouteTrie::from_routes(&scan.routes);
                hot.api_route_trie = RouteTrie::from_routes(&scan.api_routes);
                hot.manifest = build_result.manifest;
                hot.build_id = build_result.build_id;
                hot.has_custom_404 = scan.not_found.is_some();
                hot.has_custom_error = scan.error.is_some();
                hot.has_custom_document = scan.document.is_some();
            }

            // Signal full reload to clients
            hmr.send_full_reload();

            debug!("Full rebuild complete (route added/removed)");
        }
        FileEventKind::ConfigChanged => {
            hmr.send_full_reload();
        }
    }

    Ok(())
}
