use crate::hmr::HmrBroadcast;
use crate::scan_check::scan_contains_path;
use crate::watcher::{FileEvent, FileEventKind};
use anyhow::Result;
use rex_build::build_bundles_with_id;
use rex_core::RexConfig;
use rex_router::{scan_project, RouteTrie, ScanResult};
use rex_server::handlers;
use rex_server::state::{AppState, HotState};
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info};

/// Reload ESM modules and SSR bundle after a full rebuild.
async fn reload_esm_modules(
    state: &Arc<AppState>,
    config: &RexConfig,
    scan: &ScanResult,
    build_id: &str,
    client_manifest: Option<&rex_core::client_manifest::ClientReferenceManifest>,
    ssr_bundle_path: &Option<String>,
) -> Result<()> {
    let esm_scan = if let Some(app_scan) = &scan.app_scan {
        match rex_build::mdx::process_mdx_app_pages(
            app_scan,
            &config.server_build_dir(),
            &config.project_root,
        ) {
            Ok(processed) => {
                let mut s = scan.clone();
                s.app_scan = Some(processed);
                s
            }
            Err(_) => scan.clone(),
        }
    } else {
        scan.clone()
    };

    let esm_state = rex_server::startup::esm_load_modules(
        config,
        &esm_scan,
        build_id,
        &state.isolate_pool,
        client_manifest,
    )
    .await?;

    if let Some(ssr_path) = ssr_bundle_path {
        if let Ok(ssr_js) = std::fs::read_to_string(ssr_path) {
            state
                .isolate_pool
                .eval_script_all(Arc::new(ssr_js), "rsc-ssr-bundle.js")
                .await?;
        }
    }

    if let Some(esm_lock) = &state.esm {
        if let Ok(mut guard) = esm_lock.write() {
            *guard = esm_state;
        }
    }
    debug!("ESM modules reloaded after rebuild");
    Ok(())
}

/// Handle a file change event: rebuild, reload isolates, update state, notify HMR clients
pub async fn handle_file_event(
    event: FileEvent,
    config: &RexConfig,
    state: &Arc<AppState>,
    hmr: &HmrBroadcast,
    last_scan: &mut Option<ScanResult>,
) -> Result<()> {
    info!(path = %event.path.display(), kind = ?event.kind, "Processing file change");

    match event.kind {
        FileEventKind::PageModified
        | FileEventKind::CssModified
        | FileEventKind::MiddlewareModified
        | FileEventKind::McpModified
        | FileEventKind::SourceModified => {
            let t0 = Instant::now();

            // ESM fast path: for source/page changes, try re-transforming just
            // the changed file instead of a full rolldown rebuild.
            if matches!(
                event.kind,
                FileEventKind::PageModified | FileEventKind::SourceModified
            ) {
                match crate::rebuild_esm::try_esm_fast_path(state, &event.path).await {
                    Ok(true) => {
                        let elapsed = t0.elapsed();
                        info!(
                            esm_ms = elapsed.as_millis(),
                            path = %event.path.display(),
                            "ESM fast path rebuild"
                        );

                        // Invalidate browser transform cache for this file
                        if let Some(cache) = state.browser_transform_cache.get() {
                            let canonical = event
                                .path
                                .canonicalize()
                                .unwrap_or_else(|_| event.path.clone());
                            cache.invalidate(&canonical.to_string_lossy());
                        }

                        // Compute browser URL for the changed file
                        let rel = event
                            .path
                            .strip_prefix(&config.project_root)
                            .unwrap_or(&event.path);
                        let url = format!("/_rex/src/{}", rel.to_string_lossy());

                        // Find which route pattern this file belongs to (if any)
                        let route = {
                            let hot = state.hot.read().ok();
                            hot.and_then(|h| {
                                h.route_paths.iter().find_map(|(pattern, path)| {
                                    if path == &event.path {
                                        Some(pattern.clone())
                                    } else {
                                        None
                                    }
                                })
                            })
                        };

                        hmr.send_module_update(&url, route.as_deref());
                        debug!("ESM fast path complete — sent module update");
                        return Ok(());
                    }
                    Ok(false) => {
                        debug!("ESM fast path not available, falling back to full rebuild");
                    }
                    Err(e) => {
                        debug!("ESM fast path error: {e:#}, falling back to full rebuild");
                    }
                }
            }

            // Determine if we can skip the filesystem rescan
            let can_skip_scan = match event.kind {
                // Content-only changes never add/remove routes (but need a cached scan)
                FileEventKind::CssModified
                | FileEventKind::SourceModified
                | FileEventKind::MiddlewareModified => last_scan.is_some(),
                // Page/MCP edits: skip only if the path is already known (not a new file)
                FileEventKind::PageModified | FileEventKind::McpModified => last_scan
                    .as_ref()
                    .is_some_and(|s| scan_contains_path(s, &event.path)),
                _ => false,
            };

            let (scan, scan_skipped) = if can_skip_scan {
                // Safe to reuse cached scan — no route structure changes
                (
                    last_scan
                        .clone()
                        .expect("last_scan must be Some when can_skip_scan is true"),
                    true,
                )
            } else {
                let fresh = scan_project(&config.project_root, &config.pages_dir, &config.app_dir)?;
                *last_scan = Some(fresh.clone());
                (fresh, false)
            };

            let t_scan = t0.elapsed();

            // Get project_config from current hot state for build aliases
            let project_config = {
                let guard = state
                    .hot
                    .read()
                    .map_err(|e| anyhow::anyhow!("HotState lock poisoned: {e}"))?;
                guard.project_config.clone()
            };

            // Generate build_id early so ESM collection and IIFE build share the same ID.
            let build_id = rex_build::build_utils::generate_build_id();

            // ESM collection: walk import graph to pre-compute RSC reference IDs.
            // The IIFE build then uses these IDs for consistency.
            let esm_scan = if let Some(app_scan) = &scan.app_scan {
                match rex_build::mdx::process_mdx_app_pages(
                    app_scan,
                    &config.server_build_dir(),
                    &config.project_root,
                ) {
                    Ok(processed) => {
                        let mut s = scan.clone();
                        s.app_scan = Some(processed);
                        s
                    }
                    Err(_) => scan.clone(),
                }
            } else {
                scan.clone()
            };
            let precomputed_ids =
                rex_server::startup::esm_collect_ids(config, &esm_scan, &build_id)?;

            let build_result = build_bundles_with_id(
                config,
                &scan,
                &project_config,
                Some(&build_id),
                precomputed_ids.as_ref(),
            )
            .await?;
            let t_bundle = t0.elapsed();

            info!(
                scan_ms = if scan_skipped {
                    0
                } else {
                    t_scan.as_millis() as u64
                },
                bundle_ms = (t_bundle - t_scan).as_millis(),
                total_ms = t_bundle.as_millis(),
                scan_skipped,
                "Rebuild complete"
            );

            // Build manifest JSON for HMR before moving into hot state
            let hmr_manifest_json = serde_json::json!({
                "build_id": &build_result.build_id,
                "pages": &build_result.manifest.pages,
                "app_routes": &build_result.manifest.app_routes,
            });

            // Snapshot the old hot state (Arc clone, cheap)
            let old_hot = {
                let guard = state
                    .hot
                    .read()
                    .map_err(|e| anyhow::anyhow!("HotState lock poisoned: {e}"))?;
                Arc::clone(&guard)
            };

            // Reuse route tries when scan was skipped (no structural changes)
            let (app_route_trie, app_api_route_trie) = if scan_skipped {
                (
                    old_hot.app_route_trie.clone(),
                    old_hot.app_api_route_trie.clone(),
                )
            } else {
                let art = scan
                    .app_scan
                    .as_ref()
                    .map(|app| RouteTrie::from_routes(&app.to_routes()))
                    .or_else(|| old_hot.app_route_trie.clone());

                let aart = scan
                    .app_scan
                    .as_ref()
                    .and_then(|app| {
                        if app.api_routes.is_empty() {
                            None
                        } else {
                            Some(RouteTrie::from_routes(&app.to_api_routes()))
                        }
                    })
                    .or_else(|| old_hot.app_api_route_trie.clone());

                (art, aart)
            };

            // Recompute document descriptor after reload
            let has_custom_document = if scan_skipped {
                old_hot.has_custom_document
            } else {
                scan.document.is_some()
            };
            let document_descriptor = if has_custom_document {
                handlers::compute_document_descriptor(&state.isolate_pool).await
            } else {
                None
            };

            // Update hot state atomically with new Arc.
            // Scoped block ensures write guard is dropped before async ESM reload.
            let (new_build_id, new_client_manifest, new_ssr_bundle_path) = {
                let manifest_json =
                    HotState::compute_manifest_json(&build_result.build_id, &build_result.manifest);
                let new_build_id = build_result.build_id.clone();
                let new_client_manifest = build_result.manifest.client_reference_manifest.clone();
                let new_ssr_bundle_path = build_result.manifest.rsc_ssr_bundle.clone();
                let mut hot_guard = state
                    .hot
                    .write()
                    .map_err(|e| anyhow::anyhow!("HotState lock poisoned: {e}"))?;
                *hot_guard = Arc::new(HotState {
                    has_middleware: scan.middleware.is_some(),
                    middleware_matchers: build_result.manifest.middleware_matchers.clone(),
                    manifest: build_result.manifest,
                    build_id: build_result.build_id,
                    manifest_json,
                    document_descriptor,
                    has_mcp_tools: !scan.mcp_tools.is_empty(),
                    // Dev mode: no pre-rendering (always dynamic for fast iteration)
                    prerendered: std::collections::HashMap::new(),
                    prerendered_app: std::collections::HashMap::new(),
                    // Rebuild pages router tries when scan was refreshed (new page added)
                    route_trie: if scan_skipped {
                        old_hot.route_trie.clone()
                    } else {
                        RouteTrie::from_routes(&scan.routes)
                    },
                    api_route_trie: if scan_skipped {
                        old_hot.api_route_trie.clone()
                    } else {
                        RouteTrie::from_routes(&scan.api_routes)
                    },
                    has_custom_404: if scan_skipped {
                        old_hot.has_custom_404
                    } else {
                        scan.not_found.is_some()
                    },
                    has_custom_error: if scan_skipped {
                        old_hot.has_custom_error
                    } else {
                        scan.error.is_some()
                    },
                    has_custom_document: if scan_skipped {
                        old_hot.has_custom_document
                    } else {
                        scan.document.is_some()
                    },
                    project_config: old_hot.project_config.clone(),
                    app_route_trie,
                    app_api_route_trie,
                    import_map_json: old_hot.import_map_json.clone(),
                    route_paths: if scan_skipped {
                        old_hot.route_paths.clone()
                    } else {
                        let mut map = std::collections::HashMap::new();
                        for route in &scan.routes {
                            map.insert(route.pattern.clone(), route.abs_path.clone());
                        }
                        if let Some(app) = &scan.app {
                            map.insert("/_app".to_string(), app.abs_path.clone());
                        }
                        map
                    },
                });

                (new_build_id, new_client_manifest, new_ssr_bundle_path)
            }; // hot_guard dropped here

            // Reload ESM modules with new build's manifest and SSR bundle.
            reload_esm_modules(
                state,
                config,
                &scan,
                &new_build_id,
                new_client_manifest.as_ref(),
                &new_ssr_bundle_path,
            )
            .await?;

            // Notify HMR clients with the new manifest
            let rel_path = event
                .path
                .strip_prefix(&config.pages_dir)
                .or_else(|_| event.path.strip_prefix(&config.app_dir))
                .unwrap_or(&event.path);
            hmr.send_update(&rel_path.to_string_lossy(), hmr_manifest_json);

            debug!("Hot reload complete");
        }
        FileEventKind::PageRemoved => {
            // Full rebuild: routes changed, need new trie + manifest
            let scan = scan_project(&config.project_root, &config.pages_dir, &config.app_dir)?;
            *last_scan = Some(scan.clone());

            // Get project_config from current hot state for build aliases
            let project_config = {
                let guard = state
                    .hot
                    .read()
                    .map_err(|e| anyhow::anyhow!("HotState lock poisoned: {e}"))?;
                guard.project_config.clone()
            };

            // Generate build_id early for ESM/IIFE consistency.
            let build_id = rex_build::build_utils::generate_build_id();
            let esm_scan_removed = if let Some(app_scan) = &scan.app_scan {
                match rex_build::mdx::process_mdx_app_pages(
                    app_scan,
                    &config.server_build_dir(),
                    &config.project_root,
                ) {
                    Ok(processed) => {
                        let mut s = scan.clone();
                        s.app_scan = Some(processed);
                        s
                    }
                    Err(_) => scan.clone(),
                }
            } else {
                scan.clone()
            };
            let precomputed_ids =
                rex_server::startup::esm_collect_ids(config, &esm_scan_removed, &build_id)?;

            let build_result = build_bundles_with_id(
                config,
                &scan,
                &project_config,
                Some(&build_id),
                precomputed_ids.as_ref(),
            )
            .await?;

            // Snapshot old state for project_config
            let old_hot = {
                let guard = state
                    .hot
                    .read()
                    .map_err(|e| anyhow::anyhow!("HotState lock poisoned: {e}"))?;
                Arc::clone(&guard)
            };

            // Build app route trie if app scan is present
            let app_route_trie = scan
                .app_scan
                .as_ref()
                .map(|app| RouteTrie::from_routes(&app.to_routes()));

            // Build app API route trie if route.ts files exist
            let app_api_route_trie = scan.app_scan.as_ref().and_then(|app| {
                if app.api_routes.is_empty() {
                    None
                } else {
                    Some(RouteTrie::from_routes(&app.to_api_routes()))
                }
            });

            let has_custom_document = scan.document.is_some();
            let document_descriptor = if has_custom_document {
                handlers::compute_document_descriptor(&state.isolate_pool).await
            } else {
                None
            };

            let manifest_json =
                HotState::compute_manifest_json(&build_result.build_id, &build_result.manifest);

            let (new_build_id, new_client_manifest, new_ssr_bundle_path) = {
                let mut hot_guard = state
                    .hot
                    .write()
                    .map_err(|e| anyhow::anyhow!("HotState lock poisoned: {e}"))?;
                let bid = build_result.build_id.clone();
                let cm = build_result.manifest.client_reference_manifest.clone();
                let ssr = build_result.manifest.rsc_ssr_bundle.clone();
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
                    app_route_trie,
                    app_api_route_trie,
                    has_mcp_tools: !scan.mcp_tools.is_empty(),
                    prerendered: std::collections::HashMap::new(),
                    prerendered_app: std::collections::HashMap::new(),
                    import_map_json: old_hot.import_map_json.clone(),
                    route_paths: {
                        let mut map = std::collections::HashMap::new();
                        for route in &scan.routes {
                            map.insert(route.pattern.clone(), route.abs_path.clone());
                        }
                        if let Some(app) = &scan.app {
                            map.insert("/_app".to_string(), app.abs_path.clone());
                        }
                        map
                    },
                });
                (bid, cm, ssr)
            };

            // Reload ESM modules with new build
            reload_esm_modules(
                state,
                config,
                &scan,
                &new_build_id,
                new_client_manifest.as_ref(),
                &new_ssr_bundle_path,
            )
            .await?;

            hmr.send_full_reload();
            debug!("Full rebuild complete (route added/removed)");
        }
    }

    Ok(())
}
