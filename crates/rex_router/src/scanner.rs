use rex_core::app_route::AppScanResult;
use rex_core::{DynamicSegment, McpToolRoute, PageType, Route};
use std::path::Path;
use tracing::debug;

/// Result of scanning the pages directory
#[derive(Debug, Clone)]
pub struct ScanResult {
    pub routes: Vec<Route>,
    pub api_routes: Vec<Route>,
    pub app: Option<Route>,
    pub document: Option<Route>,
    pub error: Option<Route>,
    pub not_found: Option<Route>,
    /// Path to middleware file at project root (middleware.ts/js/tsx/jsx)
    pub middleware: Option<std::path::PathBuf>,
    /// App router scan result (if app/ directory exists)
    pub app_scan: Option<AppScanResult>,
    /// MCP tool files found in the mcp/ directory
    pub mcp_tools: Vec<McpToolRoute>,
}

/// Scan the pages/ directory and produce routes
pub fn scan_pages(pages_dir: &Path) -> anyhow::Result<ScanResult> {
    let mut routes = Vec::new();
    let mut api_routes = Vec::new();
    let mut app = None;
    let mut document = None;
    let mut error = None;
    let mut not_found = None;

    walk_dir(pages_dir, pages_dir, &mut |rel_path, abs_path| {
        let route = parse_route(rel_path, abs_path);
        debug!(pattern = %route.pattern, file = %route.file_path.display(), "scanned route");

        match route.page_type {
            PageType::App => app = Some(route),
            PageType::Document => document = Some(route),
            PageType::Error => error = Some(route),
            PageType::NotFound => not_found = Some(route),
            PageType::Api => api_routes.push(route),
            PageType::Regular => routes.push(route),
            PageType::AppApi => {} // app router route handlers are handled by app_scanner
        }
    })?;

    // Sort routes by specificity (highest first)
    routes.sort_by(|a, b| b.specificity.cmp(&a.specificity));
    api_routes.sort_by(|a, b| b.specificity.cmp(&a.specificity));

    Ok(ScanResult {
        routes,
        api_routes,
        app,
        document,
        error,
        not_found,
        middleware: None,
        app_scan: None,
        mcp_tools: Vec::new(),
    })
}

/// Find middleware file at the project root (next to `pages/`).
pub fn find_middleware(project_root: &Path) -> Option<std::path::PathBuf> {
    for ext in &["ts", "tsx", "js", "jsx"] {
        let path = project_root.join(format!("middleware.{ext}"));
        if path.exists() {
            return Some(path);
        }
    }
    None
}

/// Scan the mcp/ directory for tool handler files (flat, non-recursive).
pub fn scan_mcp_tools(project_root: &Path) -> Vec<McpToolRoute> {
    let mcp_dir = project_root.join("mcp");
    if !mcp_dir.exists() || !mcp_dir.is_dir() {
        return Vec::new();
    }

    let mut tools: Vec<McpToolRoute> = match std::fs::read_dir(&mcp_dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                let path = e.path();
                path.is_file() && is_page_file(&path)
            })
            .map(|e| {
                let abs_path = e.path();
                let file_path = abs_path
                    .strip_prefix(&mcp_dir)
                    .expect("mcp tool path must be under mcp_dir")
                    .to_path_buf();
                let name = file_path
                    .file_stem()
                    .expect("mcp tool file must have a stem")
                    .to_string_lossy()
                    .to_string();
                debug!(name = %name, path = %abs_path.display(), "scanned mcp tool");
                McpToolRoute {
                    name,
                    abs_path,
                    file_path,
                }
            })
            .collect(),
        Err(_) => Vec::new(),
    };

    tools.sort_by(|a, b| a.name.cmp(&b.name));
    tools
}

/// Scan pages and detect middleware. Convenience wrapper for callers that have the project root.
///
/// Also scans app/ directory if present for RSC/App Router support.
pub fn scan_project(
    project_root: &Path,
    pages_dir: &Path,
    app_dir: &Path,
) -> anyhow::Result<ScanResult> {
    let mut scan = if pages_dir.exists() {
        scan_pages(pages_dir)?
    } else {
        // No pages/ dir — return empty scan result (app-only project)
        ScanResult {
            routes: Vec::new(),
            api_routes: Vec::new(),
            app: None,
            document: None,
            error: None,
            not_found: None,
            middleware: None,
            app_scan: None,
            mcp_tools: Vec::new(),
        }
    };
    scan.middleware = find_middleware(project_root);

    // Scan app/ directory for RSC routes (check root then src/)
    let app_dir = if project_root.join("app").exists() {
        project_root.join("app")
    } else {
        project_root.join("src").join("app")
    };
    if let Some(app_scan) = crate::app_scanner::scan_app(&app_dir)? {
        debug!(routes = app_scan.routes.len(), "App router routes scanned");
        scan.app_scan = Some(app_scan);
    }

    scan.mcp_tools = scan_mcp_tools(project_root);
    Ok(scan)
}

fn walk_dir(base: &Path, dir: &Path, callback: &mut dyn FnMut(&Path, &Path)) -> anyhow::Result<()> {
    if !dir.exists() {
        return Ok(());
    }

    let mut entries: Vec<_> = std::fs::read_dir(dir)?.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            // Skip directories starting with _ (except pages themselves can have _app etc.)
            let dir_name = match path.file_name() {
                Some(n) => n.to_string_lossy(),
                None => continue,
            };
            if dir_name.starts_with('.') || dir_name == "node_modules" {
                continue;
            }
            walk_dir(base, &path, callback)?;
        } else if is_page_file(&path) {
            let rel_path = path
                .strip_prefix(base)
                .map_err(|e| anyhow::anyhow!("Failed to strip prefix {}: {e}", base.display()))?;
            callback(rel_path, &path);
        }
    }

    Ok(())
}

fn is_page_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("tsx" | "ts" | "jsx" | "js" | "mdx")
    )
}

fn parse_route(rel_path: &Path, abs_path: &Path) -> Route {
    let stem = rel_path.with_extension("");
    let stem_str = stem.to_string_lossy().replace('\\', "/");

    // Detect special pages and API routes
    let page_type = if stem_str.starts_with("api/") || stem_str == "api" {
        PageType::Api
    } else {
        match stem_str.as_str() {
            "_app" => PageType::App,
            "_document" => PageType::Document,
            "_error" => PageType::Error,
            "404" => PageType::NotFound,
            _ => PageType::Regular,
        }
    };

    // Convert file path to URL pattern
    let (pattern, dynamic_segments, specificity) = file_path_to_pattern(&stem_str);

    Route {
        pattern,
        file_path: rel_path.to_path_buf(),
        abs_path: abs_path.to_path_buf(),
        dynamic_segments,
        page_type,
        specificity,
    }
}

fn file_path_to_pattern(stem: &str) -> (String, Vec<DynamicSegment>, u32) {
    let mut segments = Vec::new();
    let mut dynamic_segments = Vec::new();
    let mut specificity: u32 = 0;

    // Handle index files
    let parts: Vec<&str> = stem.split('/').collect();
    for (i, part) in parts.iter().enumerate() {
        // "index" at the end maps to "/"
        if *part == "index" && i == parts.len() - 1 {
            continue;
        }

        if let Some(segment) = parse_dynamic_segment(part) {
            match &segment {
                DynamicSegment::Single(name) => {
                    segments.push(format!(":{name}"));
                    specificity += 5;
                }
                DynamicSegment::CatchAll(name) => {
                    segments.push(format!("*{name}"));
                    specificity += 1;
                }
                DynamicSegment::OptionalCatchAll(name) => {
                    segments.push(format!("*{name}"));
                    specificity += 1;
                }
            }
            dynamic_segments.push(segment);
        } else {
            segments.push(part.to_string());
            specificity += 10;
        }
    }

    let pattern = if segments.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", segments.join("/"))
    };

    (pattern, dynamic_segments, specificity)
}

fn parse_dynamic_segment(part: &str) -> Option<DynamicSegment> {
    // [[...slug]] - optional catch-all
    if part.starts_with("[[...") && part.ends_with("]]") {
        let name = part[5..part.len() - 2].to_string();
        return Some(DynamicSegment::OptionalCatchAll(name));
    }

    // [...slug] - catch-all
    if part.starts_with("[...") && part.ends_with(']') {
        let name = part[4..part.len() - 1].to_string();
        return Some(DynamicSegment::CatchAll(name));
    }

    // [slug] - single dynamic
    if part.starts_with('[') && part.ends_with(']') {
        let name = part[1..part.len() - 1].to_string();
        return Some(DynamicSegment::Single(name));
    }

    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_file_path_to_pattern() {
        let (p, _, _) = file_path_to_pattern("index");
        assert_eq!(p, "/");

        let (p, _, _) = file_path_to_pattern("about");
        assert_eq!(p, "/about");

        let (p, segs, _) = file_path_to_pattern("blog/[slug]");
        assert_eq!(p, "/blog/:slug");
        assert_eq!(segs.len(), 1);

        let (p, segs, _) = file_path_to_pattern("blog/[...slug]");
        assert_eq!(p, "/blog/*slug");
        assert_eq!(segs.len(), 1);

        let (p, _, _) = file_path_to_pattern("blog/index");
        assert_eq!(p, "/blog");
    }

    #[test]
    fn test_specificity() {
        let (_, _, s1) = file_path_to_pattern("blog/post");
        let (_, _, s2) = file_path_to_pattern("blog/[slug]");
        let (_, _, s3) = file_path_to_pattern("blog/[...slug]");
        assert!(s1 > s2);
        assert!(s2 > s3);
    }

    #[test]
    fn test_parse_dynamic_segment() {
        assert_eq!(
            parse_dynamic_segment("[slug]"),
            Some(DynamicSegment::Single("slug".to_string()))
        );
        assert_eq!(
            parse_dynamic_segment("[...slug]"),
            Some(DynamicSegment::CatchAll("slug".to_string()))
        );
        assert_eq!(
            parse_dynamic_segment("[[...slug]]"),
            Some(DynamicSegment::OptionalCatchAll("slug".to_string()))
        );
        assert_eq!(parse_dynamic_segment("about"), None);
    }

    #[test]
    fn test_parse_route_api() {
        let route = parse_route(
            Path::new("api/hello.ts"),
            Path::new("/tmp/pages/api/hello.ts"),
        );
        assert_eq!(route.page_type, PageType::Api);
        assert_eq!(route.pattern, "/api/hello");
        assert_eq!(route.module_name(), "api/hello");

        // Nested API route
        let route = parse_route(
            Path::new("api/users/[id].ts"),
            Path::new("/tmp/pages/api/users/[id].ts"),
        );
        assert_eq!(route.page_type, PageType::Api);
        assert_eq!(route.pattern, "/api/users/:id");
        assert_eq!(route.module_name(), "api/users/[id]");

        // Non-API route should be Regular
        let route = parse_route(Path::new("about.tsx"), Path::new("/tmp/pages/about.tsx"));
        assert_eq!(route.page_type, PageType::Regular);
    }

    #[test]
    fn test_find_middleware_ts() {
        let tmp = std::env::temp_dir().join("rex_test_find_mw");
        let _ = std::fs::create_dir_all(&tmp);
        std::fs::write(tmp.join("middleware.ts"), "export function middleware() {}").unwrap();
        let result = find_middleware(&tmp);
        assert!(result.is_some());
        assert!(result.unwrap().ends_with("middleware.ts"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_find_middleware_none() {
        let tmp = std::env::temp_dir().join("rex_test_find_mw_none");
        let _ = std::fs::create_dir_all(&tmp);
        let result = find_middleware(&tmp);
        assert!(result.is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_scan_mcp_tools_discovery() {
        let tmp = std::env::temp_dir().join("rex_test_mcp_scan");
        let mcp_dir = tmp.join("mcp");
        let _ = std::fs::create_dir_all(&mcp_dir);
        std::fs::write(mcp_dir.join("search.ts"), "export default function() {}").unwrap();
        std::fs::write(mcp_dir.join("weather.ts"), "export default function() {}").unwrap();
        // Non-tool files should be ignored
        std::fs::write(mcp_dir.join("README.md"), "# MCP Tools").unwrap();

        let tools = scan_mcp_tools(&tmp);
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "search");
        assert_eq!(tools[1].name, "weather");
        assert!(tools[0].abs_path.ends_with("search.ts"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_scan_mcp_tools_empty_dir() {
        let tmp = std::env::temp_dir().join("rex_test_mcp_empty");
        let mcp_dir = tmp.join("mcp");
        let _ = std::fs::create_dir_all(&mcp_dir);

        let tools = scan_mcp_tools(&tmp);
        assert!(tools.is_empty());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_scan_mcp_tools_no_dir() {
        let tmp = std::env::temp_dir().join("rex_test_mcp_nodir");
        let _ = std::fs::create_dir_all(&tmp);

        let tools = scan_mcp_tools(&tmp);
        assert!(tools.is_empty());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_scan_pages_basic() {
        let tmp = std::env::temp_dir().join("rex_test_scan_pages");
        let _ = std::fs::remove_dir_all(&tmp);
        let pages = tmp.join("pages");
        std::fs::create_dir_all(pages.join("blog")).unwrap();
        std::fs::write(pages.join("index.tsx"), "export default function(){}").unwrap();
        std::fs::write(pages.join("about.tsx"), "export default function(){}").unwrap();
        std::fs::write(pages.join("blog/[slug].tsx"), "export default function(){}").unwrap();
        std::fs::write(pages.join("_app.tsx"), "export default function(){}").unwrap();
        std::fs::write(pages.join("404.tsx"), "export default function(){}").unwrap();

        let scan = scan_pages(&pages).unwrap();
        assert_eq!(scan.routes.len(), 3); // index, about, blog/[slug]
        assert!(scan.app.is_some());
        assert!(scan.not_found.is_some());
        assert!(scan.document.is_none());
        assert!(scan.error.is_none());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_scan_pages_api_routes() {
        let tmp = std::env::temp_dir().join("rex_test_scan_api");
        let _ = std::fs::remove_dir_all(&tmp);
        let pages = tmp.join("pages");
        std::fs::create_dir_all(pages.join("api")).unwrap();
        std::fs::write(pages.join("index.tsx"), "export default function(){}").unwrap();
        std::fs::write(pages.join("api/hello.ts"), "export default function(){}").unwrap();

        let scan = scan_pages(&pages).unwrap();
        assert_eq!(scan.routes.len(), 1);
        assert_eq!(scan.api_routes.len(), 1);
        assert_eq!(scan.api_routes[0].pattern, "/api/hello");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_scan_project_with_middleware_and_mcp() {
        let tmp = std::env::temp_dir().join("rex_test_scan_project");
        let _ = std::fs::remove_dir_all(&tmp);
        let pages = tmp.join("pages");
        std::fs::create_dir_all(&pages).unwrap();
        std::fs::write(pages.join("index.tsx"), "export default function(){}").unwrap();
        std::fs::write(tmp.join("middleware.ts"), "export function middleware(){}").unwrap();
        let mcp = tmp.join("mcp");
        std::fs::create_dir_all(&mcp).unwrap();
        std::fs::write(mcp.join("search.ts"), "export default function(){}").unwrap();

        let app = tmp.join("app");
        let scan = scan_project(&tmp, &pages, &app).unwrap();
        assert_eq!(scan.routes.len(), 1);
        assert!(scan.middleware.is_some());
        assert_eq!(scan.mcp_tools.len(), 1);
        assert_eq!(scan.mcp_tools[0].name, "search");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_scan_project_no_pages_dir() {
        let tmp = std::env::temp_dir().join("rex_test_scan_no_pages");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let pages = tmp.join("pages"); // doesn't exist
        let app = tmp.join("app"); // doesn't exist

        let scan = scan_project(&tmp, &pages, &app).unwrap();
        assert!(scan.routes.is_empty());
        assert!(scan.api_routes.is_empty());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_scan_project_src_app_dir() {
        let tmp = std::env::temp_dir().join("rex_test_scan_src_app");
        let _ = std::fs::remove_dir_all(&tmp);

        // Simulate a src/app/ project with route groups (no root layout)
        let app_dir = tmp.join("src/app");
        let frontend = app_dir.join("(frontend)");
        std::fs::create_dir_all(frontend.join("about")).unwrap();
        std::fs::write(
            frontend.join("layout.tsx"),
            "export default function Layout({children}) { return children }",
        )
        .unwrap();
        std::fs::write(
            frontend.join("page.tsx"),
            "export default function Home() { return 'home' }",
        )
        .unwrap();
        std::fs::write(
            frontend.join("about/page.tsx"),
            "export default function About() { return 'about' }",
        )
        .unwrap();

        let pages = tmp.join("src/pages"); // doesn't exist
        let scan = scan_project(&tmp, &pages, &app_dir).unwrap();

        // Should find app routes via the app_dir parameter
        assert!(scan.app_scan.is_some());
        let app_scan = scan.app_scan.unwrap();
        assert_eq!(app_scan.routes.len(), 2);
        assert!(app_scan.root_layout.is_none()); // route-group-only

        let patterns: Vec<&str> = app_scan.routes.iter().map(|r| r.pattern.as_str()).collect();
        assert!(patterns.contains(&"/"));
        assert!(patterns.contains(&"/about"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_scan_pages_special_files() {
        let tmp = std::env::temp_dir().join("rex_test_scan_special");
        let _ = std::fs::remove_dir_all(&tmp);
        let pages = tmp.join("pages");
        std::fs::create_dir_all(&pages).unwrap();
        std::fs::write(pages.join("index.tsx"), "export default function(){}").unwrap();
        std::fs::write(pages.join("_document.tsx"), "export default function(){}").unwrap();
        std::fs::write(pages.join("_error.tsx"), "export default function(){}").unwrap();

        let scan = scan_pages(&pages).unwrap();
        assert_eq!(scan.routes.len(), 1);
        assert!(scan.document.is_some());
        assert!(scan.error.is_some());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_scan_pages_ignores_non_page_files() {
        let tmp = std::env::temp_dir().join("rex_test_scan_ignore");
        let _ = std::fs::remove_dir_all(&tmp);
        let pages = tmp.join("pages");
        std::fs::create_dir_all(&pages).unwrap();
        std::fs::write(pages.join("index.tsx"), "export default function(){}").unwrap();
        std::fs::write(pages.join("styles.css"), "body{}").unwrap();
        std::fs::write(pages.join("README.md"), "# docs").unwrap();

        let scan = scan_pages(&pages).unwrap();
        assert_eq!(scan.routes.len(), 1); // only index.tsx

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
