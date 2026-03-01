//! App Router file scanner.
//!
//! Scans the `app/` directory to produce an `AppScanResult` with:
//! - A nested `AppSegment` tree
//! - Flattened `AppRoute` entries with layout/loading/error chains

use rex_core::app_route::{AppRoute, AppScanResult, AppSegment};
use rex_core::DynamicSegment;
use std::path::{Path, PathBuf};
use tracing::debug;

const PAGE_EXTENSIONS: &[&str] = &["tsx", "ts", "jsx", "js"];

/// Scan the `app/` directory and produce an `AppScanResult`.
///
/// Returns `None` if the directory doesn't exist or has no root layout.
pub fn scan_app(app_dir: &Path) -> anyhow::Result<Option<AppScanResult>> {
    if !app_dir.exists() {
        return Ok(None);
    }

    let root = scan_segment(app_dir, "")?;

    // Root layout is required for the app router.
    let root_layout = match &root.layout {
        Some(layout) => layout.clone(),
        None => {
            debug!("app/ directory found but no root layout.tsx — skipping app router");
            return Ok(None);
        }
    };

    let mut routes = Vec::new();
    flatten_routes(
        &root,
        &[], // pattern segments
        &[], // layout chain
        &[], // loading chain
        &[], // error chain
        &mut routes,
    );

    // Sort by specificity (highest first)
    routes.sort_by(|a, b| b.specificity.cmp(&a.specificity));

    debug!(routes = routes.len(), "app/ directory scanned");

    Ok(Some(AppScanResult {
        root,
        routes,
        root_layout,
    }))
}

/// Recursively scan a directory segment, building the segment tree.
fn scan_segment(dir: &Path, segment_name: &str) -> anyhow::Result<AppSegment> {
    let layout = find_component(dir, "layout");
    let page = find_component(dir, "page");
    let loading = find_component(dir, "loading");
    let error_boundary = find_component(dir, "error");
    let not_found = find_component(dir, "not-found");

    let mut children = Vec::new();

    if dir.exists() {
        let mut entries: Vec<_> = std::fs::read_dir(dir)?.filter_map(|e| e.ok()).collect();
        entries.sort_by_key(|e| e.file_name());

        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                let dir_name = match path.file_name() {
                    Some(n) => n.to_string_lossy().to_string(),
                    None => continue,
                };
                // Skip hidden dirs and node_modules
                if dir_name.starts_with('.') || dir_name == "node_modules" {
                    continue;
                }
                let child = scan_segment(&path, &dir_name)?;
                children.push(child);
            }
        }
    }

    Ok(AppSegment {
        segment: segment_name.to_string(),
        layout,
        page,
        loading,
        error_boundary,
        not_found,
        children,
    })
}

/// Find a component file by base name (e.g., "layout" → "layout.tsx").
fn find_component(dir: &Path, name: &str) -> Option<PathBuf> {
    for ext in PAGE_EXTENSIONS {
        let path = dir.join(format!("{name}.{ext}"));
        if path.exists() {
            return Some(path);
        }
    }
    None
}

/// Flatten the segment tree into a list of routes, accumulating chains.
fn flatten_routes(
    segment: &AppSegment,
    pattern_parts: &[String],
    layout_chain: &[PathBuf],
    loading_chain: &[Option<PathBuf>],
    error_chain: &[Option<PathBuf>],
    routes: &mut Vec<AppRoute>,
) {
    // Extend chains with this segment's components.
    let mut current_layouts = layout_chain.to_vec();
    let mut current_loadings = loading_chain.to_vec();
    let mut current_errors = error_chain.to_vec();

    if let Some(layout) = &segment.layout {
        current_layouts.push(layout.clone());
        current_loadings.push(segment.loading.clone());
        current_errors.push(segment.error_boundary.clone());
    }

    // Build pattern parts for this segment.
    let mut current_parts = pattern_parts.to_vec();
    let is_group = is_route_group(&segment.segment);

    if !segment.segment.is_empty() && !is_group {
        // Add this segment to the URL pattern.
        current_parts.push(segment.segment.clone());
    }

    // If this segment has a page, register it as a route.
    if let Some(page_path) = &segment.page {
        let (pattern, dynamic_segments, specificity) = build_pattern(&current_parts);
        debug!(pattern = %pattern, page = %page_path.display(), "app route");

        routes.push(AppRoute {
            pattern,
            page_path: page_path.clone(),
            layout_chain: current_layouts.clone(),
            loading_chain: current_loadings.clone(),
            error_chain: current_errors.clone(),
            dynamic_segments,
            specificity,
        });
    }

    // Recurse into children.
    for child in &segment.children {
        flatten_routes(
            child,
            &current_parts,
            &current_layouts,
            &current_loadings,
            &current_errors,
            routes,
        );
    }
}

/// Check if a segment name is a route group (parenthesized, e.g. "(marketing)").
fn is_route_group(segment: &str) -> bool {
    segment.starts_with('(') && segment.ends_with(')')
}

/// Build a URL pattern from path parts, parsing dynamic segments.
fn build_pattern(parts: &[String]) -> (String, Vec<DynamicSegment>, u32) {
    let mut segments = Vec::new();
    let mut dynamic_segments = Vec::new();
    let mut specificity: u32 = 0;

    for part in parts {
        if let Some(dyn_seg) = parse_dynamic_segment(part) {
            match &dyn_seg {
                DynamicSegment::Single(name) => {
                    segments.push(format!(":{name}"));
                    specificity += 5;
                }
                DynamicSegment::CatchAll(name) | DynamicSegment::OptionalCatchAll(name) => {
                    segments.push(format!("*{name}"));
                    specificity += 1;
                }
            }
            dynamic_segments.push(dyn_seg);
        } else {
            segments.push(part.clone());
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

/// Parse a directory name as a dynamic segment.
/// Reuses the same conventions as the pages router scanner.
fn parse_dynamic_segment(part: &str) -> Option<DynamicSegment> {
    if part.starts_with("[[...") && part.ends_with("]]") {
        let name = part[5..part.len() - 2].to_string();
        return Some(DynamicSegment::OptionalCatchAll(name));
    }
    if part.starts_with("[...") && part.ends_with(']') {
        let name = part[4..part.len() - 1].to_string();
        return Some(DynamicSegment::CatchAll(name));
    }
    if part.starts_with('[') && part.ends_with(']') {
        let name = part[1..part.len() - 1].to_string();
        return Some(DynamicSegment::Single(name));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup_fixture(dir: &Path) {
        // app/
        //   layout.tsx
        //   page.tsx
        //   loading.tsx
        //   about/
        //     page.tsx
        //   blog/
        //     page.tsx
        //     [slug]/
        //       page.tsx
        //       layout.tsx
        //   (marketing)/
        //     pricing/
        //       page.tsx
        //   dashboard/
        //     layout.tsx
        //     page.tsx
        //     settings/
        //       page.tsx

        let dirs = [
            "",
            "about",
            "blog",
            "blog/[slug]",
            "(marketing)",
            "(marketing)/pricing",
            "dashboard",
            "dashboard/settings",
        ];
        for d in &dirs {
            fs::create_dir_all(dir.join(d)).unwrap();
        }

        let files = [
            "layout.tsx",
            "page.tsx",
            "loading.tsx",
            "about/page.tsx",
            "blog/page.tsx",
            "blog/[slug]/page.tsx",
            "blog/[slug]/layout.tsx",
            "(marketing)/pricing/page.tsx",
            "dashboard/layout.tsx",
            "dashboard/page.tsx",
            "dashboard/settings/page.tsx",
        ];
        for f in &files {
            fs::write(dir.join(f), format!("// {f}")).unwrap();
        }
    }

    #[test]
    fn scans_app_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let app_dir = tmp.path().join("app");
        setup_fixture(&app_dir);

        let result = scan_app(&app_dir).unwrap().unwrap();

        assert!(result.root_layout.exists());
        assert!(!result.routes.is_empty());
    }

    #[test]
    fn correct_route_count() {
        let tmp = tempfile::tempdir().unwrap();
        let app_dir = tmp.path().join("app");
        setup_fixture(&app_dir);

        let result = scan_app(&app_dir).unwrap().unwrap();

        // Routes: /, /about, /blog, /blog/:slug, /pricing, /dashboard, /dashboard/settings
        assert_eq!(result.routes.len(), 7);
    }

    #[test]
    fn route_group_excluded_from_url() {
        let tmp = tempfile::tempdir().unwrap();
        let app_dir = tmp.path().join("app");
        setup_fixture(&app_dir);

        let result = scan_app(&app_dir).unwrap().unwrap();
        let patterns: Vec<&str> = result.routes.iter().map(|r| r.pattern.as_str()).collect();

        // (marketing)/pricing → /pricing (group not in URL)
        assert!(patterns.contains(&"/pricing"));
        assert!(!patterns.iter().any(|p| p.contains("marketing")));
    }

    #[test]
    fn dynamic_segment_parsed() {
        let tmp = tempfile::tempdir().unwrap();
        let app_dir = tmp.path().join("app");
        setup_fixture(&app_dir);

        let result = scan_app(&app_dir).unwrap().unwrap();
        let blog_slug = result
            .routes
            .iter()
            .find(|r| r.pattern == "/blog/:slug")
            .expect("should have /blog/:slug route");

        assert_eq!(blog_slug.dynamic_segments.len(), 1);
        assert!(matches!(
            &blog_slug.dynamic_segments[0],
            DynamicSegment::Single(n) if n == "slug"
        ));
    }

    #[test]
    fn layout_chain_ordering() {
        let tmp = tempfile::tempdir().unwrap();
        let app_dir = tmp.path().join("app");
        setup_fixture(&app_dir);

        let result = scan_app(&app_dir).unwrap().unwrap();

        // /blog/:slug has two layouts: root + blog/[slug]/layout.tsx
        let blog_slug = result
            .routes
            .iter()
            .find(|r| r.pattern == "/blog/:slug")
            .unwrap();
        assert_eq!(blog_slug.layout_chain.len(), 2);
        // First should be root layout
        assert!(blog_slug.layout_chain[0]
            .to_string_lossy()
            .ends_with("layout.tsx"));
        // Second should be blog/[slug]/layout.tsx
        assert!(blog_slug.layout_chain[1]
            .to_string_lossy()
            .contains("[slug]"));

        // /about has only root layout
        let about = result
            .routes
            .iter()
            .find(|r| r.pattern == "/about")
            .unwrap();
        assert_eq!(about.layout_chain.len(), 1);
    }

    #[test]
    fn nested_layout_chain() {
        let tmp = tempfile::tempdir().unwrap();
        let app_dir = tmp.path().join("app");
        setup_fixture(&app_dir);

        let result = scan_app(&app_dir).unwrap().unwrap();

        // /dashboard/settings has root layout + dashboard/layout.tsx
        let settings = result
            .routes
            .iter()
            .find(|r| r.pattern == "/dashboard/settings")
            .unwrap();
        assert_eq!(settings.layout_chain.len(), 2);
    }

    #[test]
    fn loading_chain_parallel() {
        let tmp = tempfile::tempdir().unwrap();
        let app_dir = tmp.path().join("app");
        setup_fixture(&app_dir);

        let result = scan_app(&app_dir).unwrap().unwrap();

        // Root has loading.tsx, so first entry is Some
        let root_route = result.routes.iter().find(|r| r.pattern == "/").unwrap();
        assert_eq!(root_route.loading_chain.len(), 1);
        assert!(root_route.loading_chain[0].is_some());
    }

    #[test]
    fn no_app_dir_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let result = scan_app(&tmp.path().join("nonexistent")).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn no_root_layout_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let app_dir = tmp.path().join("app");
        fs::create_dir_all(&app_dir).unwrap();
        fs::write(app_dir.join("page.tsx"), "// page").unwrap();
        // No layout.tsx

        let result = scan_app(&app_dir).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn specificity_ordering() {
        let tmp = tempfile::tempdir().unwrap();
        let app_dir = tmp.path().join("app");
        setup_fixture(&app_dir);

        let result = scan_app(&app_dir).unwrap().unwrap();

        // Routes should be sorted by specificity descending.
        // Static routes have higher specificity than dynamic ones.
        for window in result.routes.windows(2) {
            assert!(window[0].specificity >= window[1].specificity);
        }
    }
}
