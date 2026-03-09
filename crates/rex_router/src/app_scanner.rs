//! App Router file scanner.
//!
//! Scans the `app/` directory to produce an `AppScanResult` with:
//! - A nested `AppSegment` tree
//! - Flattened `AppRoute` entries with layout/loading/error chains

use rex_core::app_route::{AppApiRoute, AppRoute, AppScanResult, AppSegment};
use rex_core::DynamicSegment;
use std::path::{Path, PathBuf};
use tracing::debug;

const PAGE_EXTENSIONS: &[&str] = &["tsx", "ts", "jsx", "js", "mdx"];

/// Scan the `app/` directory and produce an `AppScanResult`.
///
/// Returns `None` if the directory doesn't exist or has no routes.
/// Supports both a shared root `layout.tsx` and the route-group-only pattern
/// where each `(group)/` provides its own layout (Next.js convention).
pub fn scan_app(app_dir: &Path) -> anyhow::Result<Option<AppScanResult>> {
    if !app_dir.exists() {
        return Ok(None);
    }

    let root = scan_segment(app_dir, "")?;

    let root_layout = root.layout.clone();

    // Flatten API routes (route.ts files) — these don't need layouts
    let mut api_routes = Vec::new();
    flatten_api_routes(&root, &[], &mut api_routes);
    api_routes.sort_by(|a, b| b.specificity.cmp(&a.specificity));

    // If there is no root layout, check that at least one route group child
    // has a layout — otherwise the app directory has no usable page routes.
    // API routes (route.ts) are still valid without a layout.
    if root_layout.is_none() {
        let has_group_layout = root
            .children
            .iter()
            .any(|child| is_route_group(&child.segment) && child.layout.is_some());
        if !has_group_layout && api_routes.is_empty() {
            debug!("app/ directory found but no root layout, no route group layouts, and no route handlers — skipping app router");
            return Ok(None);
        }
        if !has_group_layout {
            debug!("app/ directory has no root layout but has route handlers");
        } else {
            debug!("app/ directory has no root layout but route groups provide layouts");
        }
    }

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

    debug!(
        routes = routes.len(),
        api_routes = api_routes.len(),
        "app/ directory scanned"
    );

    Ok(Some(AppScanResult {
        root,
        routes,
        api_routes,
        root_layout,
    }))
}

/// Recursively scan a directory segment, building the segment tree.
fn scan_segment(dir: &Path, segment_name: &str) -> anyhow::Result<AppSegment> {
    let layout = find_component(dir, "layout");
    let page = find_component(dir, "page");
    let route = find_component(dir, "route");
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

    // Per Next.js convention: route.ts and page.tsx are mutually exclusive.
    // If both exist, page takes priority and route is ignored.
    let route = if page.is_some() { None } else { route };

    Ok(AppSegment {
        segment: segment_name.to_string(),
        layout,
        page,
        route,
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

/// Flatten the segment tree into a list of API routes (route.ts handlers).
fn flatten_api_routes(
    segment: &AppSegment,
    pattern_parts: &[String],
    api_routes: &mut Vec<AppApiRoute>,
) {
    let mut current_parts = pattern_parts.to_vec();
    let is_group = is_route_group(&segment.segment);

    if !segment.segment.is_empty() && !is_group {
        current_parts.push(segment.segment.clone());
    }

    // If this segment has a route handler, register it as an API route.
    if let Some(handler_path) = &segment.route {
        let (pattern, dynamic_segments, specificity) = build_pattern(&current_parts);
        debug!(pattern = %pattern, handler = %handler_path.display(), "app api route");

        api_routes.push(AppApiRoute {
            pattern,
            handler_path: handler_path.clone(),
            dynamic_segments,
            specificity,
        });
    }

    for child in &segment.children {
        flatten_api_routes(child, &current_parts, api_routes);
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
