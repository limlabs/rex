//! MDX pre-processing: compile `.mdx` files to JSX before rolldown bundling.
//!
//! The actual MDX-to-JSX compiler lives in the [`rex_mdx`] crate. This module
//! handles scanning for `.mdx` pages in both the pages router (`ScanResult`)
//! and app router (`AppScanResult`), compiling them, and providing override
//! maps so the bundler uses the compiled `.jsx` files.

use anyhow::Result;
use rex_core::app_route::AppScanResult;
use rex_mdx::{compile_mdx_with_options, find_mdx_components, MdxOptions};
use rex_router::ScanResult;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Result of MDX pre-processing.
pub(crate) struct MdxProcessing {
    /// Map of original page abs_path → compiled JSX path
    pub page_overrides: HashMap<PathBuf, PathBuf>,
}

/// Build `MdxOptions` by scanning the project root for `mdx-components.*`.
fn mdx_options_for_project(project_root: &Path) -> MdxOptions {
    MdxOptions {
        mdx_components_path: find_mdx_components(project_root),
    }
}

/// Pre-process all `.mdx` page files in the scan result.
///
/// For each `.mdx` page, compiles it to a `.jsx` file in a temp directory
/// and records the override so the bundler uses the compiled version.
pub(crate) fn process_mdx_pages(
    scan: &ScanResult,
    output_dir: &Path,
    project_root: &Path,
) -> Result<MdxProcessing> {
    let temp_dir = output_dir.join("_mdx");
    let mut page_overrides = HashMap::new();
    let options = mdx_options_for_project(project_root);

    // Collect all MDX pages: regular routes + _app + special pages
    let mut mdx_sources: Vec<&PathBuf> = Vec::new();
    for route in &scan.routes {
        if is_mdx(&route.abs_path) {
            mdx_sources.push(&route.abs_path);
        }
    }
    for route in [&scan.app, &scan.document, &scan.error, &scan.not_found]
        .into_iter()
        .flatten()
    {
        if is_mdx(&route.abs_path) {
            mdx_sources.push(&route.abs_path);
        }
    }

    if mdx_sources.is_empty() {
        return Ok(MdxProcessing { page_overrides });
    }

    fs::create_dir_all(&temp_dir)?;

    for source_path in mdx_sources {
        let source = fs::read_to_string(source_path)?;
        let compiled = compile_mdx_with_options(&source, &options)?;

        // Absolutize relative imports so they resolve from the temp directory
        let source_dir = source_path.parent().unwrap_or(Path::new("."));
        let compiled = crate::css_modules::absolutize_relative_imports(&compiled, source_dir);

        // Write compiled JSX to temp dir with a unique name
        let hash = path_hash(source_path);
        let stem = source_path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy();
        let compiled_path = temp_dir.join(format!("{hash}_{stem}.jsx"));
        fs::write(&compiled_path, &compiled)?;

        debug!(
            source = %source_path.display(),
            compiled = %compiled_path.display(),
            "Compiled MDX page"
        );
        page_overrides.insert(source_path.clone(), compiled_path);
    }

    Ok(MdxProcessing { page_overrides })
}

/// Pre-process `.mdx` files found in an app router scan result.
///
/// Returns a cloned `AppScanResult` with any `.mdx` page/layout paths replaced
/// by their compiled `.jsx` equivalents.
pub(crate) fn process_mdx_app_pages(
    app_scan: &AppScanResult,
    output_dir: &Path,
    project_root: &Path,
) -> Result<AppScanResult> {
    let temp_dir = output_dir.join("_mdx");
    let options = mdx_options_for_project(project_root);

    // Collect all unique MDX paths from routes and root layout
    let mut mdx_paths: Vec<PathBuf> = Vec::new();
    if is_mdx(&app_scan.root_layout) {
        mdx_paths.push(app_scan.root_layout.clone());
    }
    for route in &app_scan.routes {
        if is_mdx(&route.page_path) {
            mdx_paths.push(route.page_path.clone());
        }
        for layout in &route.layout_chain {
            if is_mdx(layout) {
                mdx_paths.push(layout.clone());
            }
        }
    }
    mdx_paths.sort();
    mdx_paths.dedup();

    if mdx_paths.is_empty() {
        return Ok(app_scan.clone());
    }

    fs::create_dir_all(&temp_dir)?;

    // Compile each MDX file and build the override map
    let mut overrides: HashMap<PathBuf, PathBuf> = HashMap::new();
    for source_path in &mdx_paths {
        let source = fs::read_to_string(source_path)?;
        let compiled = compile_mdx_with_options(&source, &options)?;

        // Absolutize relative imports so they resolve from the temp directory
        let source_dir = source_path.parent().unwrap_or(Path::new("."));
        let compiled = crate::css_modules::absolutize_relative_imports(&compiled, source_dir);

        let hash = path_hash(source_path);
        let stem = source_path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy();
        let compiled_path = temp_dir.join(format!("{hash}_{stem}.jsx"));
        fs::write(&compiled_path, &compiled)?;

        debug!(
            source = %source_path.display(),
            compiled = %compiled_path.display(),
            "Compiled MDX app page"
        );
        overrides.insert(source_path.clone(), compiled_path);
    }

    // Clone and patch the scan result
    let mut result = app_scan.clone();
    if let Some(replacement) = overrides.get(&result.root_layout) {
        result.root_layout = replacement.clone();
    }
    for route in &mut result.routes {
        if let Some(replacement) = overrides.get(&route.page_path) {
            route.page_path = replacement.clone();
        }
        for layout in &mut route.layout_chain {
            if let Some(replacement) = overrides.get(layout) {
                *layout = replacement.clone();
            }
        }
    }
    // Also patch the segment tree
    patch_segment_mdx(&mut result.root, &overrides);

    Ok(result)
}

/// Recursively patch MDX paths in the segment tree.
fn patch_segment_mdx(
    segment: &mut rex_core::app_route::AppSegment,
    overrides: &HashMap<PathBuf, PathBuf>,
) {
    if let Some(ref p) = segment.layout {
        if let Some(replacement) = overrides.get(p) {
            segment.layout = Some(replacement.clone());
        }
    }
    if let Some(ref p) = segment.page {
        if let Some(replacement) = overrides.get(p) {
            segment.page = Some(replacement.clone());
        }
    }
    for child in &mut segment.children {
        patch_segment_mdx(child, overrides);
    }
}

fn is_mdx(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some("mdx")
}

fn path_hash(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    hex::encode(&hasher.finalize()[..4])
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn process_mdx_no_mdx_files() {
        let scan = ScanResult {
            routes: vec![],
            api_routes: vec![],
            app: None,
            document: None,
            error: None,
            not_found: None,
            middleware: None,
            app_scan: None,
            mcp_tools: vec![],
        };
        let tmp = std::env::temp_dir().join("rex_test_mdx_empty");
        let _ = fs::create_dir_all(&tmp);
        let result = process_mdx_pages(&scan, &tmp, &tmp).unwrap();
        assert!(result.page_overrides.is_empty());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn process_mdx_app_pages_no_mdx() {
        use rex_core::app_route::{AppRoute, AppSegment};

        let tmp = std::env::temp_dir().join("rex_test_mdx_app_no_mdx");
        let _ = fs::create_dir_all(&tmp);

        let layout_path = tmp.join("layout.tsx");
        fs::write(
            &layout_path,
            "export default function Layout({children}) { return children; }",
        )
        .unwrap();
        let page_path = tmp.join("page.tsx");
        fs::write(
            &page_path,
            "export default function Page() { return null; }",
        )
        .unwrap();

        let app_scan = AppScanResult {
            root: AppSegment {
                segment: String::new(),
                layout: Some(layout_path.clone()),
                page: Some(page_path.clone()),
                loading: None,
                error_boundary: None,
                not_found: None,
                children: vec![],
            },
            routes: vec![AppRoute {
                pattern: "/".to_string(),
                page_path: page_path.clone(),
                layout_chain: vec![layout_path.clone()],
                loading_chain: vec![],
                error_chain: vec![],
                dynamic_segments: vec![],
                specificity: 1,
            }],
            root_layout: layout_path,
        };

        let result = process_mdx_app_pages(&app_scan, &tmp, &tmp).unwrap();
        assert_eq!(result.root_layout, app_scan.root_layout);
        assert_eq!(result.routes[0].page_path, app_scan.routes[0].page_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn process_mdx_app_pages_with_mdx() {
        use rex_core::app_route::{AppRoute, AppSegment};

        let tmp = std::env::temp_dir().join("rex_test_mdx_app_with_mdx");
        let _ = fs::create_dir_all(&tmp);

        let layout_path = tmp.join("layout.tsx");
        fs::write(
            &layout_path,
            "export default function Layout({children}) { return children; }",
        )
        .unwrap();
        let page_path = tmp.join("page.mdx");
        fs::write(&page_path, "# Hello World\n\nSome content.\n").unwrap();

        let app_scan = AppScanResult {
            root: AppSegment {
                segment: String::new(),
                layout: Some(layout_path.clone()),
                page: Some(page_path.clone()),
                loading: None,
                error_boundary: None,
                not_found: None,
                children: vec![],
            },
            routes: vec![AppRoute {
                pattern: "/".to_string(),
                page_path: page_path.clone(),
                layout_chain: vec![layout_path.clone()],
                loading_chain: vec![],
                error_chain: vec![],
                dynamic_segments: vec![],
                specificity: 1,
            }],
            root_layout: layout_path.clone(),
        };

        let result = process_mdx_app_pages(&app_scan, &tmp, &tmp).unwrap();
        assert_ne!(result.routes[0].page_path, page_path);
        assert!(result.routes[0].page_path.extension().unwrap() == "jsx");
        assert_eq!(result.root_layout, layout_path);
        assert_ne!(result.root.page.as_ref().unwrap(), &page_path);
        let compiled = fs::read_to_string(&result.routes[0].page_path).unwrap();
        assert!(compiled.contains("createElement(_components.h1"));
        let _ = fs::remove_dir_all(&tmp);
    }
}
