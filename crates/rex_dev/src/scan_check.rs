use rex_router::ScanResult;
use std::path::Path;

/// Check if a file path is already known in a `ScanResult` (i.e. it's an existing file,
/// not a newly created one).
pub fn scan_contains_path(scan: &ScanResult, path: &Path) -> bool {
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
}
