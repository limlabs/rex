use crate::hmr::HmrBroadcast;
use crate::watcher::{FileEvent, FileEventKind};
use anyhow::Result;
use rex_build::build_bundles;
use rex_build::bundler::BuildResult;
use rex_build::transform::TransformCache;
use rex_core::RexConfig;
use rex_router::{scan_project, RouteTrie, ScanResult};
use rex_server::handlers;
use rex_server::state::{AppState, HotState};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info};

/// Check if a file path is already known in a `ScanResult` (i.e. it's an existing file,
/// not a newly created one).
fn scan_contains_path(scan: &ScanResult, path: &Path) -> bool {
    // Pages router: routes + api_routes
    if scan
        .routes
        .iter()
        .chain(scan.api_routes.iter())
        .any(|r| r.abs_path == path)
    {
        return true;
    }

    // Special pages: _app, _document, _error, 404
    let specials = [&scan.app, &scan.document, &scan.error, &scan.not_found];
    if specials
        .iter()
        .any(|s| s.as_ref().is_some_and(|r| r.abs_path == path))
    {
        return true;
    }

    // Middleware
    if scan.middleware.as_deref() == Some(path) {
        return true;
    }

    // MCP tools
    if scan.mcp_tools.iter().any(|t| t.abs_path == path) {
        return true;
    }

    // App router
    if let Some(app) = &scan.app_scan {
        if app.root_layout.as_deref() == Some(path) {
            return true;
        }
        for route in &app.routes {
            if route.page_path == path {
                return true;
            }
            if route.layout_chain.iter().any(|p| p == path) {
                return true;
            }
            if route
                .loading_chain
                .iter()
                .any(|p| p.as_deref() == Some(path))
            {
                return true;
            }
            if route.error_chain.iter().any(|p| p.as_deref() == Some(path)) {
                return true;
            }
        }
        for api in &app.api_routes {
            if api.handler_path == path {
                return true;
            }
        }
    }

    false
}

/// Read the server bundle from disk, appending RSC bundles if present.
fn read_server_bundle(build_result: &BuildResult) -> Result<Arc<String>> {
    let mut bundle_js = std::fs::read_to_string(&build_result.server_bundle_path)?;
    if let Some(rsc_path) = &build_result.manifest.rsc_server_bundle {
        let rsc_bundle = std::fs::read_to_string(rsc_path)?;
        bundle_js.push_str("\n;\n");
        bundle_js.push_str(&rsc_bundle);
    }
    if let Some(ssr_path) = &build_result.manifest.rsc_ssr_bundle {
        let ssr_bundle = std::fs::read_to_string(ssr_path)?;
        bundle_js.push_str("\n;\n");
        bundle_js.push_str(&ssr_bundle);
    }
    Ok(Arc::new(bundle_js))
}

/// Handle a file change event: rebuild, reload isolates, update state, notify HMR clients
pub async fn handle_file_event(
    event: FileEvent,
    config: &RexConfig,
    state: &Arc<AppState>,
    hmr: &HmrBroadcast,
    last_scan: &mut Option<ScanResult>,
) -> Result<()> {
    debug!(path = %event.path.display(), kind = ?event.kind, "Processing file change");

    match event.kind {
        FileEventKind::PageModified
        | FileEventKind::CssModified
        | FileEventKind::MiddlewareModified
        | FileEventKind::McpModified
        | FileEventKind::SourceModified => {
            let t0 = Instant::now();

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

            let build_result = build_bundles(config, &scan, &project_config).await?;
            let t_bundle = t0.elapsed();

            let bundle_arc = read_server_bundle(&build_result)?;

            // Lazy reload: mark isolates stale instead of synchronous reload_all
            state.isolate_pool.mark_stale(bundle_arc);
            let t_reload = t0.elapsed();

            info!(
                scan_ms = if scan_skipped {
                    0
                } else {
                    t_scan.as_millis() as u64
                },
                bundle_ms = (t_bundle - t_scan).as_millis(),
                v8_reload = "lazy",
                v8_mark_ms = (t_reload - t_bundle).as_millis(),
                total_ms = t_reload.as_millis(),
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

            // Update hot state atomically with new Arc
            let manifest_json =
                HotState::compute_manifest_json(&build_result.build_id, &build_result.manifest);
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
            });

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

            let build_result = build_bundles(config, &scan, &project_config).await?;

            let bundle_arc = read_server_bundle(&build_result)?;

            state.isolate_pool.reload_all(bundle_arc).await?;

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

            // Update all hot state atomically with new Arc
            let mut hot_guard = state
                .hot
                .write()
                .map_err(|e| anyhow::anyhow!("HotState lock poisoned: {e}"))?;
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
                // Dev mode: no pre-rendering
                prerendered: std::collections::HashMap::new(),
                prerendered_app: std::collections::HashMap::new(),
            });

            // Signal full reload to clients
            hmr.send_full_reload();

            debug!("Full rebuild complete (route added/removed)");
        }
    }

    Ok(())
}

/// Handle a file change in ESM dev mode (unbundled fast path).
///
/// Instead of running rolldown to rebuild all bundles, this:
/// 1. OXC-transforms only the changed file (~3ms)
/// 2. Invalidates the module in all V8 isolates (~3ms)
/// 3. Sends an HMR update with a dev URL for the browser to reimport
///
/// For structural changes (page added/removed), falls back to a full reload.
pub async fn handle_esm_file_event(
    event: FileEvent,
    config: &RexConfig,
    state: &Arc<AppState>,
    hmr: &HmrBroadcast,
    transform_cache: &Arc<TransformCache>,
    page_sources: &Arc<Vec<(String, PathBuf)>>,
) -> Result<()> {
    debug!(path = %event.path.display(), kind = ?event.kind, "ESM file change");

    match event.kind {
        FileEventKind::PageModified | FileEventKind::SourceModified => {
            let t0 = Instant::now();

            // Read and OXC-transform the changed file
            let source = std::fs::read_to_string(&event.path)?;
            let transformed = transform_cache.transform(&event.path, &source)?;
            let t_transform = t0.elapsed();

            // Invalidate the module in all V8 isolates
            state
                .isolate_pool
                .invalidate_module(event.path.clone(), transformed, page_sources.clone())
                .await?;
            let t_invalidate = t0.elapsed();

            info!(
                transform_ms = t_transform.as_millis(),
                v8_ms = (t_invalidate - t_transform).as_millis(),
                total_ms = t_invalidate.as_millis(),
                "ESM hot update"
            );

            // Notify HMR client with dev URL
            let rel_path = event
                .path
                .strip_prefix(&config.project_root)
                .unwrap_or(&event.path);
            hmr.send_dev_esm_update(&rel_path.to_string_lossy());
        }
        FileEventKind::CssModified => {
            // CSS changes don't need V8 invalidation — just notify browser to reload
            hmr.send_full_reload();
        }
        FileEventKind::PageRemoved
        | FileEventKind::MiddlewareModified
        | FileEventKind::McpModified => {
            // Structural changes require full reload
            hmr.send_full_reload();
        }
    }

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rex_core::{McpToolRoute, PageType, Route};
    use std::path::PathBuf;

    fn make_route(abs: &str) -> Route {
        Route {
            pattern: String::new(),
            file_path: PathBuf::from(abs),
            abs_path: PathBuf::from(abs),
            dynamic_segments: vec![],
            page_type: PageType::Regular,
            specificity: 0,
        }
    }

    fn empty_scan() -> ScanResult {
        ScanResult {
            routes: vec![],
            api_routes: vec![],
            app: None,
            document: None,
            error: None,
            not_found: None,
            middleware: None,
            app_scan: None,
            mcp_tools: vec![],
        }
    }

    #[test]
    fn scan_contains_path_matches_page_route() {
        let scan = ScanResult {
            routes: vec![make_route("/pages/index.tsx")],
            ..empty_scan()
        };
        assert!(scan_contains_path(&scan, Path::new("/pages/index.tsx")));
        assert!(!scan_contains_path(&scan, Path::new("/pages/about.tsx")));
    }

    #[test]
    fn scan_contains_path_matches_api_route() {
        let scan = ScanResult {
            api_routes: vec![make_route("/pages/api/hello.ts")],
            ..empty_scan()
        };
        assert!(scan_contains_path(&scan, Path::new("/pages/api/hello.ts")));
    }

    #[test]
    fn scan_contains_path_matches_special_pages() {
        let scan = ScanResult {
            app: Some(make_route("/pages/_app.tsx")),
            document: Some(make_route("/pages/_document.tsx")),
            error: Some(make_route("/pages/_error.tsx")),
            not_found: Some(make_route("/pages/404.tsx")),
            ..empty_scan()
        };
        assert!(scan_contains_path(&scan, Path::new("/pages/_app.tsx")));
        assert!(scan_contains_path(&scan, Path::new("/pages/_document.tsx")));
        assert!(scan_contains_path(&scan, Path::new("/pages/_error.tsx")));
        assert!(scan_contains_path(&scan, Path::new("/pages/404.tsx")));
    }

    #[test]
    fn scan_contains_path_matches_middleware() {
        let scan = ScanResult {
            middleware: Some(PathBuf::from("/project/middleware.ts")),
            ..empty_scan()
        };
        assert!(scan_contains_path(
            &scan,
            Path::new("/project/middleware.ts")
        ));
        assert!(!scan_contains_path(
            &scan,
            Path::new("/project/middleware.js")
        ));
    }

    #[test]
    fn scan_contains_path_matches_mcp_tools() {
        let scan = ScanResult {
            mcp_tools: vec![McpToolRoute {
                name: "search".into(),
                abs_path: PathBuf::from("/project/mcp/search.ts"),
                file_path: PathBuf::from("search.ts"),
            }],
            ..empty_scan()
        };
        assert!(scan_contains_path(
            &scan,
            Path::new("/project/mcp/search.ts")
        ));
    }

    #[test]
    fn scan_contains_path_empty_scan_returns_false() {
        let scan = empty_scan();
        assert!(!scan_contains_path(&scan, Path::new("/pages/index.tsx")));
    }

    #[test]
    fn scan_contains_path_matches_app_router_page() {
        use rex_core::app_route::{AppApiRoute, AppRoute, AppScanResult, AppSegment};
        let scan = ScanResult {
            app_scan: Some(AppScanResult {
                root: AppSegment {
                    segment: "app".into(),
                    page: None,
                    layout: None,
                    route: None,
                    loading: None,
                    error_boundary: None,
                    not_found: None,
                    children: vec![],
                },
                routes: vec![AppRoute {
                    pattern: "/".into(),
                    page_path: PathBuf::from("/app/page.tsx"),
                    layout_chain: vec![PathBuf::from("/app/layout.tsx")],
                    loading_chain: vec![Some(PathBuf::from("/app/loading.tsx"))],
                    error_chain: vec![Some(PathBuf::from("/app/error.tsx"))],
                    dynamic_segments: vec![],
                    specificity: 0,
                    route_group: None,
                }],
                api_routes: vec![AppApiRoute {
                    pattern: "/api/test".into(),
                    handler_path: PathBuf::from("/app/api/test/route.ts"),
                    dynamic_segments: vec![],
                    specificity: 0,
                }],
                root_layout: Some(PathBuf::from("/app/layout.tsx")),
            }),
            ..empty_scan()
        };
        // page_path
        assert!(scan_contains_path(&scan, Path::new("/app/page.tsx")));
        // layout_chain
        assert!(scan_contains_path(&scan, Path::new("/app/layout.tsx")));
        // loading_chain
        assert!(scan_contains_path(&scan, Path::new("/app/loading.tsx")));
        // error_chain
        assert!(scan_contains_path(&scan, Path::new("/app/error.tsx")));
        // api_routes handler_path
        assert!(scan_contains_path(
            &scan,
            Path::new("/app/api/test/route.ts")
        ));
        // root_layout
        assert!(scan_contains_path(&scan, Path::new("/app/layout.tsx")));
        // Unknown path
        assert!(!scan_contains_path(&scan, Path::new("/app/unknown.tsx")));
    }

    #[test]
    fn read_server_bundle_reads_basic_bundle() {
        use rex_core::AssetManifest;
        let dir = tempfile::tempdir().unwrap();
        let bundle_path = dir.path().join("server-bundle.js");
        std::fs::write(&bundle_path, "var x = 1;").unwrap();
        let build_result = BuildResult {
            build_id: "test".into(),
            manifest: AssetManifest::new("test".into()),
            server_bundle_path: bundle_path,
        };
        let result = read_server_bundle(&build_result).unwrap();
        assert_eq!(&*result, "var x = 1;");
    }

    #[test]
    fn read_server_bundle_appends_rsc_bundles() {
        use rex_core::AssetManifest;
        let dir = tempfile::tempdir().unwrap();
        let bundle_path = dir.path().join("server-bundle.js");
        let rsc_path = dir.path().join("rsc-server.js");
        let ssr_path = dir.path().join("rsc-ssr.js");
        std::fs::write(&bundle_path, "var x = 1;").unwrap();
        std::fs::write(&rsc_path, "var rsc = 2;").unwrap();
        std::fs::write(&ssr_path, "var ssr = 3;").unwrap();
        let mut manifest = AssetManifest::new("test".into());
        manifest.rsc_server_bundle = Some(rsc_path.to_string_lossy().into());
        manifest.rsc_ssr_bundle = Some(ssr_path.to_string_lossy().into());
        let build_result = BuildResult {
            build_id: "test".into(),
            manifest,
            server_bundle_path: bundle_path,
        };
        let result = read_server_bundle(&build_result).unwrap();
        assert!(result.contains("var x = 1;"));
        assert!(result.contains("var rsc = 2;"));
        assert!(result.contains("var ssr = 3;"));
    }
}
