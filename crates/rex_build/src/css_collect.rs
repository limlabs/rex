use crate::build_utils::{detect_data_strategy, route_to_chunk_name};
use crate::manifest::AssetManifest;
use anyhow::Result;
use rex_router::ScanResult;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Scan source files for CSS imports and copy them to the output directory.
/// Registers global CSS (from _app) and per-page CSS in the manifest.
/// When a CSS file has been pre-processed by Tailwind (present in `tailwind_outputs`),
/// the processed output is used instead of the raw source.
pub(crate) fn collect_css_files(
    scan: &ScanResult,
    output_dir: &Path,
    build_id: &str,
    manifest: &mut AssetManifest,
    tailwind_outputs: &HashMap<PathBuf, PathBuf>,
    page_overrides: &HashMap<PathBuf, PathBuf>,
) -> Result<()> {
    let hash = &build_id[..8];

    // Collect CSS from _app (global styles)
    if let Some(app) = &scan.app {
        let effective = page_overrides.get(&app.abs_path).unwrap_or(&app.abs_path);
        let css_paths = extract_css_imports(effective)?;
        for css_path in css_paths {
            if css_path.exists() {
                let stem = css_path.file_stem().unwrap_or_default().to_string_lossy();
                let filename = format!("{stem}-{hash}.css");
                let dest = output_dir.join(&filename);
                // Use Tailwind-processed output if available, otherwise raw source
                if let Some(tw_output) = tailwind_outputs.get(&css_path) {
                    let content = fs::read_to_string(tw_output)?;
                    fs::write(&dest, &content)?;
                    manifest.css_contents.insert(filename.clone(), content);
                } else {
                    let content = fs::read_to_string(&css_path)?;
                    fs::copy(&css_path, &dest)?;
                    manifest.css_contents.insert(filename.clone(), content);
                }
                manifest.global_css.push(filename);
            }
        }
    }

    // Collect CSS from individual pages
    for route in &scan.routes {
        let effective = page_overrides
            .get(&route.abs_path)
            .unwrap_or(&route.abs_path);
        let css_paths = extract_css_imports(effective)?;
        if css_paths.is_empty() {
            continue;
        }
        let mut page_css = Vec::new();
        for css_path in css_paths {
            if css_path.exists() {
                let stem = css_path.file_stem().unwrap_or_default().to_string_lossy();
                let filename = format!("{stem}-{hash}.css");
                let dest = output_dir.join(&filename);
                if let Some(tw_output) = tailwind_outputs.get(&css_path) {
                    let content = fs::read_to_string(tw_output)?;
                    fs::write(&dest, &content)?;
                    manifest.css_contents.insert(filename.clone(), content);
                } else {
                    let content = fs::read_to_string(&css_path)?;
                    fs::copy(&css_path, &dest)?;
                    manifest.css_contents.insert(filename.clone(), content);
                }
                page_css.push(filename);
            }
        }
        if !page_css.is_empty() {
            let chunk_name = route_to_chunk_name(route);
            let js_filename = format!("{chunk_name}-{hash}.js");
            let strategy = detect_data_strategy(&route.abs_path)?;
            let has_dynamic = !route.dynamic_segments.is_empty();
            manifest.add_page_with_css(
                &route.pattern,
                &js_filename,
                &page_css,
                strategy,
                has_dynamic,
            );
        }
    }

    Ok(())
}

/// Parse a source file and extract CSS import paths (resolved relative to the file).
///
/// Uses the OXC parser to find `ImportDeclaration` nodes whose source ends with
/// `.css` (excluding `.module.css`, which is handled separately).
pub(crate) fn extract_css_imports(source_path: &Path) -> Result<Vec<PathBuf>> {
    let source = fs::read_to_string(source_path)?;
    let parent = source_path.parent().unwrap_or(Path::new("."));

    let source_type = source_type_for_path(source_path);
    let allocator = oxc_allocator::Allocator::default();
    let parsed = oxc_parser::Parser::new(&allocator, &source, source_type).parse();

    let mut css_paths = Vec::new();
    for stmt in &parsed.program.body {
        if let oxc_ast::ast::Statement::ImportDeclaration(import) = stmt {
            let specifier = import.source.value.as_str();
            if specifier.ends_with(".css") && !specifier.ends_with(".module.css") {
                css_paths.push(parent.join(specifier));
            }
        }
    }

    Ok(css_paths)
}

/// Infer OXC source type from file extension.
pub(crate) fn source_type_for_path(path: &Path) -> oxc_span::SourceType {
    match path.extension().and_then(|e| e.to_str()) {
        Some("tsx") => oxc_span::SourceType::tsx(),
        Some("ts") => oxc_span::SourceType::ts(),
        Some("jsx") => oxc_span::SourceType::jsx(),
        _ => oxc_span::SourceType::mjs(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::TempDir;

    #[test]
    fn test_extract_css_imports_finds_css() {
        let tmp = TempDir::new().unwrap();
        let page = tmp.path().join("page.tsx");
        fs::write(
            &page,
            "import './styles.css';\nimport React from 'react';\nexport default function Page() {}\n",
        )
        .unwrap();
        let css_file = tmp.path().join("styles.css");
        fs::write(&css_file, "body {}").unwrap();

        let result = extract_css_imports(&page).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result[0].ends_with("styles.css"));
    }

    #[test]
    fn test_extract_css_imports_skips_module_css() {
        let tmp = TempDir::new().unwrap();
        let page = tmp.path().join("page.tsx");
        fs::write(
            &page,
            "import './styles.module.css';\nimport './global.css';\n",
        )
        .unwrap();

        let result = extract_css_imports(&page).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result[0].ends_with("global.css"));
    }

    #[test]
    fn test_extract_css_imports_double_quotes() {
        let tmp = TempDir::new().unwrap();
        let page = tmp.path().join("page.tsx");
        fs::write(&page, "import \"./theme.css\";\n").unwrap();

        let result = extract_css_imports(&page).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result[0].ends_with("theme.css"));
    }

    #[test]
    fn test_extract_css_imports_no_css() {
        let tmp = TempDir::new().unwrap();
        let page = tmp.path().join("page.tsx");
        fs::write(
            &page,
            "import React from 'react';\nexport default function Page() {}\n",
        )
        .unwrap();

        let result = extract_css_imports(&page).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_collect_css_files_global_from_app() {
        let tmp = TempDir::new().unwrap();
        let pages_dir = tmp.path().join("pages");
        fs::create_dir_all(&pages_dir).unwrap();

        let css_file = tmp.path().join("globals.css");
        fs::write(&css_file, "body { margin: 0; }").unwrap();

        let app_file = pages_dir.join("_app.tsx");
        fs::write(
            &app_file,
            format!(
                "import '{}';\nexport default function App() {{}}\n",
                css_file.display()
            ),
        )
        .unwrap();

        let output_dir = tmp.path().join("output");
        fs::create_dir_all(&output_dir).unwrap();

        let scan = rex_router::ScanResult {
            app: Some(rex_core::Route {
                pattern: "/_app".to_string(),
                file_path: app_file.clone(),
                abs_path: app_file,
                dynamic_segments: vec![],
                page_type: rex_core::PageType::Regular,
                specificity: 0,
            }),
            routes: vec![],
            not_found: None,
            error: None,
            document: None,
            middleware: None,
            mcp_tools: vec![],
            api_routes: vec![],
            app_scan: None,
        };

        let mut manifest = crate::manifest::AssetManifest::new("abc12345def67890".to_string());
        collect_css_files(
            &scan,
            &output_dir,
            "abc12345def67890",
            &mut manifest,
            &HashMap::new(),
            &HashMap::new(),
        )
        .unwrap();

        assert_eq!(manifest.global_css.len(), 1);
        assert!(manifest.global_css[0].contains("globals"));
        assert!(manifest.global_css[0].ends_with(".css"));
        assert!(manifest.css_contents.contains_key(&manifest.global_css[0]));
    }

    #[test]
    fn test_collect_css_files_per_page() {
        let tmp = TempDir::new().unwrap();
        let pages_dir = tmp.path().join("pages");
        fs::create_dir_all(&pages_dir).unwrap();

        let css_file = pages_dir.join("about.css");
        fs::write(&css_file, ".about { color: red; }").unwrap();

        let page_file = pages_dir.join("about.tsx");
        fs::write(
            &page_file,
            format!(
                "import '{}';\nexport default function About() {{}}\n",
                css_file.display()
            ),
        )
        .unwrap();

        let output_dir = tmp.path().join("output");
        fs::create_dir_all(&output_dir).unwrap();

        let scan = rex_router::ScanResult {
            app: None,
            routes: vec![rex_core::Route {
                pattern: "/about".to_string(),
                file_path: page_file.clone(),
                abs_path: page_file,
                dynamic_segments: vec![],
                page_type: rex_core::PageType::Regular,
                specificity: 0,
            }],
            not_found: None,
            error: None,
            document: None,
            middleware: None,
            mcp_tools: vec![],
            api_routes: vec![],
            app_scan: None,
        };

        let mut manifest = crate::manifest::AssetManifest::new("abc12345def67890".to_string());
        collect_css_files(
            &scan,
            &output_dir,
            "abc12345def67890",
            &mut manifest,
            &HashMap::new(),
            &HashMap::new(),
        )
        .unwrap();

        assert!(manifest.pages.contains_key("/about"));
        let page = &manifest.pages["/about"];
        assert!(!page.css.is_empty());
    }

    #[test]
    fn test_collect_css_files_with_tailwind_override() {
        let tmp = TempDir::new().unwrap();
        let pages_dir = tmp.path().join("pages");
        fs::create_dir_all(&pages_dir).unwrap();

        let raw_css = tmp.path().join("globals.css");
        fs::write(&raw_css, "@import \"tailwindcss\";").unwrap();

        let tw_output = tmp.path().join("globals.tailwind.css");
        fs::write(&tw_output, ".processed { color: blue; }").unwrap();

        let app_file = pages_dir.join("_app.tsx");
        fs::write(
            &app_file,
            format!(
                "import '{}';\nexport default function App() {{}}\n",
                raw_css.display()
            ),
        )
        .unwrap();

        let output_dir = tmp.path().join("output");
        fs::create_dir_all(&output_dir).unwrap();

        let scan = rex_router::ScanResult {
            app: Some(rex_core::Route {
                pattern: "/_app".to_string(),
                file_path: app_file.clone(),
                abs_path: app_file,
                dynamic_segments: vec![],
                page_type: rex_core::PageType::Regular,
                specificity: 0,
            }),
            routes: vec![],
            not_found: None,
            error: None,
            document: None,
            middleware: None,
            mcp_tools: vec![],
            api_routes: vec![],
            app_scan: None,
        };

        let mut tw_map = HashMap::new();
        tw_map.insert(raw_css.clone(), tw_output);

        let mut manifest = crate::manifest::AssetManifest::new("abc12345def67890".to_string());
        collect_css_files(
            &scan,
            &output_dir,
            "abc12345def67890",
            &mut manifest,
            &tw_map,
            &HashMap::new(),
        )
        .unwrap();

        assert_eq!(manifest.global_css.len(), 1);
        let content = &manifest.css_contents[&manifest.global_css[0]];
        assert!(content.contains("processed"), "should use tailwind output");
    }
}
