use crate::hmr::HmrBroadcast;
use crate::watcher::{FileEvent, FileEventKind};
use anyhow::Result;
use rex_build::build_bundles;
use rex_core::RexConfig;
use rex_router::{scan_project, RouteTrie};
use rex_server::handlers::{self, AppState, HotState};
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
        FileEventKind::PageModified | FileEventKind::CssModified | FileEventKind::MiddlewareModified => {
            let t0 = Instant::now();

            // Rescan, rebuild, reload isolates, update hot state
            let scan = scan_project(&config.project_root, &config.pages_dir)?;
            let t_scan = t0.elapsed();

            // Get project_config from current hot state for build aliases
            let project_config = {
                let guard = state.hot.read().map_err(|e| anyhow::anyhow!("HotState lock poisoned: {e}"))?;
                guard.project_config.clone()
            };

            let build_result = build_bundles(config, &scan, &project_config).await?;
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
            let hmr_manifest_json = serde_json::json!({
                "build_id": &build_result.build_id,
                "pages": &build_result.manifest.pages,
            });

            // Snapshot the old hot state (Arc clone, cheap)
            let old_hot = {
                let guard = state.hot.read().map_err(|e| anyhow::anyhow!("HotState lock poisoned: {e}"))?;
                Arc::clone(&guard)
            };

            // Recompute document descriptor after reload
            let document_descriptor = if old_hot.has_custom_document {
                handlers::compute_document_descriptor(&state.isolate_pool).await
            } else {
                None
            };

            // Update hot state atomically with new Arc
            let manifest_json = HotState::compute_manifest_json(&build_result.build_id, &build_result.manifest);
            let mut hot_guard = state.hot.write().map_err(|e| anyhow::anyhow!("HotState lock poisoned: {e}"))?;
            *hot_guard = Arc::new(HotState {
                has_middleware: scan.middleware.is_some(),
                middleware_matchers: build_result.manifest.middleware_matchers.clone(),
                manifest: build_result.manifest,
                build_id: build_result.build_id,
                manifest_json,
                document_descriptor,
                // Preserve unchanged fields
                route_trie: old_hot.route_trie.clone(),
                api_route_trie: old_hot.api_route_trie.clone(),
                has_custom_404: old_hot.has_custom_404,
                has_custom_error: old_hot.has_custom_error,
                has_custom_document: old_hot.has_custom_document,
                project_config: old_hot.project_config.clone(),
            });

            // Notify HMR clients with the new manifest
            let rel_path = event
                .path
                .strip_prefix(&config.pages_dir)
                .unwrap_or(&event.path);
            hmr.send_update(&rel_path.to_string_lossy(), hmr_manifest_json);

            debug!("Hot reload complete");
        }
        FileEventKind::PageRemoved => {
            // Full rebuild: routes changed, need new trie + manifest
            let scan = scan_project(&config.project_root, &config.pages_dir)?;

            // Get project_config from current hot state for build aliases
            let project_config = {
                let guard = state.hot.read().map_err(|e| anyhow::anyhow!("HotState lock poisoned: {e}"))?;
                guard.project_config.clone()
            };

            let build_result = build_bundles(config, &scan, &project_config).await?;

            let bundle_js = std::fs::read_to_string(&build_result.server_bundle_path)?;
            let bundle_arc = Arc::new(bundle_js);

            state.isolate_pool.reload_all(bundle_arc).await?;

            // Snapshot old state for project_config
            let old_hot = {
                let guard = state.hot.read().map_err(|e| anyhow::anyhow!("HotState lock poisoned: {e}"))?;
                Arc::clone(&guard)
            };

            let has_custom_document = scan.document.is_some();
            let document_descriptor = if has_custom_document {
                handlers::compute_document_descriptor(&state.isolate_pool).await
            } else {
                None
            };

            let manifest_json = HotState::compute_manifest_json(&build_result.build_id, &build_result.manifest);

            // Update all hot state atomically with new Arc
            let mut hot_guard = state.hot.write().map_err(|e| anyhow::anyhow!("HotState lock poisoned: {e}"))?;
            *hot_guard = Arc::new(HotState {
                route_trie: RouteTrie::from_routes(&scan.routes),
                api_route_trie: RouteTrie::from_routes(&scan.api_routes),
                has_middleware: scan.middleware.is_some(),
                middleware_matchers: build_result.manifest.middleware_matchers.clone(),
                manifest: build_result.manifest,
                build_id: build_result.build_id,
                has_custom_404: scan.not_found.is_some(),
                has_custom_error: scan.error.is_some(),
                has_custom_document,
                project_config: old_hot.project_config.clone(),
                manifest_json,
                document_descriptor,
            });

            // Signal full reload to clients
            hmr.send_full_reload();

            debug!("Full rebuild complete (route added/removed)");
        }
    }

    Ok(())
}
