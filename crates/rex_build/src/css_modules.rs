use anyhow::Result;
use rex_router::ScanResult;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Result of CSS module pre-processing.
pub(crate) struct CssModuleProcessing {
    /// Map of original page abs_path → modified page path (with CSS module imports rewritten)
    pub page_overrides: HashMap<PathBuf, PathBuf>,
    /// Scoped CSS files per route pattern
    pub route_css: HashMap<String, Vec<String>>,
    /// Scoped CSS files from _app (global)
    pub global_css: Vec<String>,
}

/// Pre-process MDX pages and CSS modules before rolldown bundling.
///
/// First compiles any `.mdx` pages to `.jsx` (via `process_mdx_pages`).
/// Then, for each page that imports `.module.css` files:
/// 1. Parse the CSS to extract class names
/// 2. Generate scoped class names and write scoped CSS to output
/// 3. Generate a JS proxy that exports the class name mapping
/// 4. Create a modified page source with CSS module imports rewritten to proxy imports
pub(crate) fn process_css_modules(
    scan: &ScanResult,
    output_dir: &Path,
    build_id: &str,
    project_root: &Path,
) -> Result<CssModuleProcessing> {
    let hash_prefix = &build_id[..8];
    let temp_dir = output_dir.join("_css_modules");
    fs::create_dir_all(&temp_dir)?;

    // Pre-process MDX pages first — their compiled .jsx files become the
    // effective source for any subsequent CSS module scanning.
    let mdx = crate::mdx::process_mdx_pages(scan, output_dir, project_root)?;
    let mut page_overrides = mdx.page_overrides;
    let mut route_css: HashMap<String, Vec<String>> = HashMap::new();
    let mut global_css = Vec::new();

    // Track processed CSS module files to avoid duplicating work
    let mut processed_css: HashMap<PathBuf, (String, HashMap<String, String>)> = HashMap::new();

    // Collect all source files to scan: (original_abs_path, effective_path, route_pattern or None for _app)
    // Use the MDX-compiled path when available so CSS module imports in MDX pages are found.
    let mut sources: Vec<(&PathBuf, PathBuf, Option<&str>)> = Vec::new();
    for route in &scan.routes {
        let effective = page_overrides
            .get(&route.abs_path)
            .cloned()
            .unwrap_or_else(|| route.abs_path.clone());
        sources.push((&route.abs_path, effective, Some(&route.pattern)));
    }
    if let Some(app) = &scan.app {
        let effective = page_overrides
            .get(&app.abs_path)
            .cloned()
            .unwrap_or_else(|| app.abs_path.clone());
        sources.push((&app.abs_path, effective, None));
    }

    for (original_path, effective_path, route_pattern) in &sources {
        let css_module_imports = find_css_module_imports(effective_path)?;
        if css_module_imports.is_empty() {
            continue;
        }

        let source_dir = effective_path.parent().unwrap_or(Path::new("."));
        let mut source_content = fs::read_to_string(effective_path)?;
        let mut page_css_files = Vec::new();

        for (import_specifier, css_abs_path) in &css_module_imports {
            // Process each CSS module file (reuse if already processed)
            let (css_filename, class_map) = if let Some(cached) = processed_css.get(css_abs_path) {
                cached.clone()
            } else {
                let css_content = fs::read_to_string(css_abs_path)?;
                let classes = parse_css_classes(&css_content);
                let file_hash = css_module_hash(css_abs_path);
                let stem = css_module_stem(css_abs_path);

                let mut class_map = HashMap::new();
                for class in &classes {
                    let scoped = format!("{stem}_{class}_{file_hash}");
                    class_map.insert(class.clone(), scoped);
                }

                // Write scoped CSS to output
                let scoped_css = scope_css(&css_content, &class_map);
                let css_filename = format!("{stem}.module-{hash_prefix}.css");
                fs::write(output_dir.join(&css_filename), &scoped_css)?;

                processed_css.insert(
                    css_abs_path.clone(),
                    (css_filename.clone(), class_map.clone()),
                );
                (css_filename, class_map)
            };

            page_css_files.push(css_filename);

            // Generate proxy JS file
            let proxy_content = generate_css_module_proxy(&class_map);
            let proxy_name = format!(
                "{}.js",
                css_abs_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            );
            let proxy_path = temp_dir.join(&proxy_name);
            fs::write(&proxy_path, &proxy_content)?;

            // Replace the CSS module import specifier with the absolute proxy path
            let proxy_abs = proxy_path.to_string_lossy().replace('\\', "/");
            source_content = source_content.replace(import_specifier, &proxy_abs);
        }

        // Absolutize remaining relative imports so the file works from the temp dir
        source_content = absolutize_relative_imports(&source_content, source_dir);

        // Write modified page source to temp dir
        let modified_name = effective_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        // Use a unique name to avoid collisions between pages in different dirs
        let unique_name = format!("{}_{}", css_module_hash(effective_path), modified_name);
        let modified_path = temp_dir.join(&unique_name);
        fs::write(&modified_path, &source_content)?;

        page_overrides.insert((*original_path).clone(), modified_path);

        // Track CSS files
        if let Some(pattern) = route_pattern {
            route_css
                .entry(pattern.to_string())
                .or_default()
                .extend(page_css_files);
        } else {
            global_css.extend(page_css_files);
        }
    }

    Ok(CssModuleProcessing {
        page_overrides,
        route_css,
        global_css,
    })
}

/// Find `.module.css` imports in a source file using the OXC parser.
/// Returns: Vec of (import_specifier, resolved_absolute_path).
fn find_css_module_imports(source_path: &Path) -> Result<Vec<(String, PathBuf)>> {
    let source = fs::read_to_string(source_path)?;
    let parent = source_path.parent().unwrap_or(Path::new("."));

    let source_type = crate::css_collect::source_type_for_path(source_path);
    let allocator = oxc_allocator::Allocator::default();
    let parsed = oxc_parser::Parser::new(&allocator, &source, source_type).parse();

    let mut results = Vec::new();
    for stmt in &parsed.program.body {
        if let oxc_ast::ast::Statement::ImportDeclaration(import) = stmt {
            let specifier = import.source.value.as_str();
            if specifier.ends_with(".module.css") {
                let abs_path = parent.join(specifier);
                if abs_path.exists() {
                    results.push((specifier.to_string(), abs_path));
                }
            }
        }
    }

    Ok(results)
}

/// Parse CSS source to extract class names from selectors.
pub(crate) fn parse_css_classes(css: &str) -> Vec<String> {
    let mut classes = Vec::new();
    let bytes = css.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        // Skip CSS comments
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            if let Some(end) = css[i + 2..].find("*/") {
                i += end + 4;
                continue;
            }
        }

        if bytes[i] == b'.' {
            let start = i + 1;
            if start < bytes.len() && (bytes[start].is_ascii_alphabetic() || bytes[start] == b'_') {
                let mut end = start;
                while end < bytes.len()
                    && (bytes[end].is_ascii_alphanumeric()
                        || bytes[end] == b'_'
                        || bytes[end] == b'-')
                {
                    end += 1;
                }
                let class = &css[start..end];
                if !classes.contains(&class.to_string()) {
                    classes.push(class.to_string());
                }
                i = end;
                continue;
            }
        }
        i += 1;
    }

    classes
}

/// Generate a short hash for CSS module scoping based on the file path.
fn css_module_hash(file_path: &Path) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(file_path.to_string_lossy().as_bytes());
    hex::encode(&hasher.finalize()[..4])
}

/// Extract the stem from a CSS module filename (e.g., `Button.module.css` → `Button`).
fn css_module_stem(file_path: &Path) -> String {
    file_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("module")
        .trim_end_matches(".module")
        .to_string()
}

/// Rewrite CSS with scoped class names.
pub(crate) fn scope_css(css: &str, class_map: &HashMap<String, String>) -> String {
    let mut result = css.to_string();
    // Sort by length descending to avoid partial replacements (e.g., `.btn` before `.btn-primary`)
    let mut entries: Vec<_> = class_map.iter().collect();
    entries.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    for (original, scoped) in entries {
        result = result.replace(&format!(".{original}"), &format!(".{scoped}"));
    }
    result
}

/// Generate JS proxy file content for a CSS module.
pub(crate) fn generate_css_module_proxy(class_map: &HashMap<String, String>) -> String {
    let mut entries: Vec<_> = class_map.iter().collect();
    entries.sort_by_key(|(k, _)| (*k).clone());

    let pairs: Vec<String> = entries
        .iter()
        .map(|(orig, scoped)| format!("  \"{orig}\": \"{scoped}\""))
        .collect();

    format!(
        "var __css_module = {{\n{}\n}};\nexport default __css_module;\n",
        pairs.join(",\n")
    )
}

/// Absolutize relative imports in a source file so it can be moved to a temp directory.
///
/// Uses the OXC parser to find import/export declarations with relative source
/// specifiers, then performs span-based replacements on the source string value.
pub(crate) fn absolutize_relative_imports(source: &str, source_dir: &Path) -> String {
    use oxc_ast::ast::Statement;
    use oxc_span::GetSpan;

    let allocator = oxc_allocator::Allocator::default();
    let source_type = oxc_span::SourceType::tsx();
    let parsed = oxc_parser::Parser::new(&allocator, source, source_type).parse();

    // Collect (span_start, span_end, replacement) for source string literals.
    // The span covers the full StringLiteral including quotes.
    let mut replacements: Vec<(u32, u32, String)> = Vec::new();

    for stmt in &parsed.program.body {
        let source_lit = match stmt {
            Statement::ImportDeclaration(import) => Some(&import.source),
            Statement::ExportNamedDeclaration(export) => export.source.as_ref(),
            Statement::ExportAllDeclaration(export) => Some(&export.source),
            _ => None,
        };
        if let Some(lit) = source_lit {
            let specifier = lit.value.as_str();
            if specifier.starts_with("./") || specifier.starts_with("../") {
                let abs = source_dir.join(specifier);
                let abs_str = abs.to_string_lossy().replace('\\', "/");
                let span = lit.span();
                // Reconstruct the string literal with its original quote style.
                // The first char of the span in source is the quote character.
                let quote = &source[span.start as usize..span.start as usize + 1];
                let replacement = format!("{quote}{abs_str}{quote}");
                replacements.push((span.start, span.end, replacement));
            }
        }
    }

    if replacements.is_empty() {
        return source.to_string();
    }

    // Apply replacements from back to front to preserve positions.
    replacements.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
    let mut result = source.to_string();
    for (start, end, replacement) in replacements {
        result.replace_range(start as usize..end as usize, &replacement);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_css_module_hash_deterministic() {
        let path = Path::new("/app/Button.module.css");
        assert_eq!(css_module_hash(path), css_module_hash(path));
    }

    #[test]
    fn test_css_module_hash_differs() {
        let a = css_module_hash(Path::new("/app/Button.module.css"));
        let b = css_module_hash(Path::new("/app/Card.module.css"));
        assert_ne!(a, b);
    }

    #[test]
    fn test_css_module_stem_basic() {
        assert_eq!(css_module_stem(Path::new("Button.module.css")), "Button");
    }

    #[test]
    fn test_css_module_stem_fallback() {
        // Path with no file stem returns "module"
        assert_eq!(css_module_stem(Path::new("")), "module");
    }

    #[test]
    fn test_parse_css_classes_from_selectors() {
        let css = r#"
.container { padding: 20px; }
.title { font-size: 24px; }
.btn-primary { background: blue; }
.btn-primary:hover { background: darkblue; }
/* .commented { display: none; } */
"#;
        let classes = parse_css_classes(css);
        assert!(classes.contains(&"container".to_string()));
        assert!(classes.contains(&"title".to_string()));
        assert!(classes.contains(&"btn-primary".to_string()));
    }

    #[test]
    fn test_scope_css_rewrites_classes() {
        let css = ".container { padding: 20px; }\n.title { font-size: 24px; }\n";
        let mut class_map = HashMap::new();
        class_map.insert("container".to_string(), "Home_container_abc".to_string());
        class_map.insert("title".to_string(), "Home_title_abc".to_string());

        let scoped = scope_css(css, &class_map);
        assert!(scoped.contains(".Home_container_abc"));
        assert!(scoped.contains(".Home_title_abc"));
        assert!(!scoped.contains(".container"));
        assert!(!scoped.contains(".title"));
    }

    #[test]
    fn test_generate_css_module_proxy_content() {
        let mut class_map = HashMap::new();
        class_map.insert("container".to_string(), "Home_container_abc".to_string());
        class_map.insert("title".to_string(), "Home_title_abc".to_string());

        let proxy = generate_css_module_proxy(&class_map);
        assert!(proxy.contains("\"container\": \"Home_container_abc\""));
        assert!(proxy.contains("\"title\": \"Home_title_abc\""));
        assert!(proxy.contains("export default"));
    }

    #[test]
    fn test_absolutize_relative_imports() {
        let source = "import Foo from './foo';\nimport React from 'react';\n";
        let dir = Path::new("/project/src");
        let result = absolutize_relative_imports(source, dir);
        assert!(
            result.contains("/project/src/"),
            "relative import should be absolutized: {result}"
        );
        assert!(
            result.contains("foo"),
            "specifier name should be preserved: {result}"
        );
        assert!(result.contains("from 'react'"), "bare specifiers unchanged");
    }

    #[test]
    fn test_absolutize_side_effect_import() {
        let source = "import './styles.css';\nimport React from 'react';\n";
        let dir = Path::new("/project/src");
        let result = absolutize_relative_imports(source, dir);
        assert!(
            result.contains("/project/src/"),
            "side-effect import should be absolutized: {result}"
        );
        assert!(
            result.contains("styles.css"),
            "filename preserved: {result}"
        );
        assert!(result.contains("from 'react'"), "bare specifiers unchanged");
    }

    #[test]
    fn test_absolutize_export_from() {
        let source = "export { Foo } from './foo';\n";
        let dir = Path::new("/project/src");
        let result = absolutize_relative_imports(source, dir);
        assert!(
            result.contains("/project/src/"),
            "export from should be absolutized: {result}"
        );
    }
}
