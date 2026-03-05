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
) -> Result<()> {
    let hash = &build_id[..8];

    // Collect CSS from _app (global styles)
    if let Some(app) = &scan.app {
        let css_paths = extract_css_imports(&app.abs_path)?;
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
        let css_paths = extract_css_imports(&route.abs_path)?;
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
            manifest.add_page_with_css(&route.pattern, &js_filename, &page_css, strategy);
        }
    }

    Ok(())
}

/// Parse a source file and extract CSS import paths (resolved relative to the file).
pub(crate) fn extract_css_imports(source_path: &Path) -> Result<Vec<PathBuf>> {
    let source = fs::read_to_string(source_path)?;
    let parent = source_path.parent().unwrap_or(Path::new("."));
    let mut css_paths = Vec::new();

    for line in source.lines() {
        let trimmed = line.trim();
        // Match: import 'path.css' or import "path.css"
        if trimmed.starts_with("import ")
            || trimmed.starts_with("import'")
            || trimmed.starts_with("import\"")
        {
            if let Some(path) = extract_string_literal(trimmed) {
                // Skip .module.css — handled separately by process_css_modules
                if path.ends_with(".css") && !path.ends_with(".module.css") {
                    css_paths.push(parent.join(path));
                }
            }
        }
    }

    Ok(css_paths)
}

/// Extract the string literal from an import statement.
/// E.g. `import '../styles/globals.css';` → `../styles/globals.css`
pub(crate) fn extract_string_literal(line: &str) -> Option<&str> {
    // Find first quote character
    let single = line.find('\'');
    let double = line.find('"');
    let (quote_char, start) = match (single, double) {
        (Some(s), Some(d)) => {
            if s < d {
                ('\'', s)
            } else {
                ('"', d)
            }
        }
        (Some(s), None) => ('\'', s),
        (None, Some(d)) => ('"', d),
        (None, None) => return None,
    };
    let rest = &line[start + 1..];
    let end = rest.find(quote_char)?;
    Some(&rest[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_string_literal_single_quotes() {
        assert_eq!(
            extract_string_literal("import './foo.css';"),
            Some("./foo.css")
        );
    }

    #[test]
    fn test_extract_string_literal_double_quotes() {
        assert_eq!(
            extract_string_literal(r#"import "./foo.css";"#),
            Some("./foo.css")
        );
    }

    #[test]
    fn test_extract_string_literal_from_syntax() {
        assert_eq!(
            extract_string_literal("import x from './foo';"),
            Some("./foo")
        );
    }

    #[test]
    fn test_extract_string_literal_no_quotes() {
        assert_eq!(extract_string_literal("import foo"), None);
    }
}
