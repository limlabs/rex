use crate::hmr::HmrBroadcast;
use crate::watcher::{FileEvent, FileEventKind};
use anyhow::Result;
use rex_build::build_bundles;
use rex_core::RexConfig;
use rex_router::{scan_pages, RouteTrie};
use rex_server::handlers::AppState;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info};

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
            let t0 = Instant::now();

            // Rescan, rebuild, reload isolates, update hot state
            let scan = scan_pages(&config.pages_dir)?;
            let t_scan = t0.elapsed();

            let build_result = build_bundles(config, &scan).await?;
            let t_bundle = t0.elapsed();

            // Read the new server bundle
            let bundle_js = std::fs::read_to_string(&build_result.server_bundle_path)?;
            let bundle_arc = Arc::new(bundle_js);

            // Reload all V8 isolates
            state.isolate_pool.reload_all(bundle_arc).await?;
            let t_reload = t0.elapsed();

            info!(
                scan_ms = t_scan.as_millis(),
                bundle_ms = (t_bundle - t_scan).as_millis(),
                v8_reload_ms = (t_reload - t_bundle).as_millis(),
                total_ms = t_reload.as_millis(),
                "Rebuild complete"
            );

            // Build manifest JSON for HMR before moving into hot state
            let manifest_json = serde_json::json!({
                "build_id": &build_result.build_id,
                "pages": &build_result.manifest.pages,
            });

            // Update hot state with new manifest and build_id
            {
                let mut hot = state.hot.write().unwrap();
                hot.manifest = build_result.manifest;
                hot.build_id = build_result.build_id;
            }

            // Notify HMR clients with the new manifest
            let rel_path = event
                .path
                .strip_prefix(&config.pages_dir)
                .unwrap_or(&event.path);
            hmr.send_update(&rel_path.to_string_lossy(), manifest_json);

            debug!("Hot reload complete");
        }
        FileEventKind::PageRemoved => {
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
    }

    Ok(())
}
