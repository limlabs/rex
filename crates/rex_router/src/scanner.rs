use rex_core::app_route::AppScanResult;
use rex_core::{DynamicSegment, PageType, Route};
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

/// Scan pages and detect middleware. Convenience wrapper for callers that have the project root.
///
/// Also scans app/ directory if present for RSC/App Router support.
pub fn scan_project(project_root: &Path, pages_dir: &Path) -> anyhow::Result<ScanResult> {
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
        }
    };
    scan.middleware = find_middleware(project_root);

    // Scan app/ directory for RSC routes
    let app_dir = project_root.join("app");
    if let Some(app_scan) = crate::app_scanner::scan_app(&app_dir)? {
        debug!(routes = app_scan.routes.len(), "App router routes scanned");
        scan.app_scan = Some(app_scan);
    }

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
        Some("tsx" | "ts" | "jsx" | "js")
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
}
