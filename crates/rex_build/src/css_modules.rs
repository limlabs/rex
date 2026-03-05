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

/// Pre-process CSS modules before rolldown bundling.
///
/// For each page that imports `.module.css` files:
/// 1. Parse the CSS to extract class names
/// 2. Generate scoped class names and write scoped CSS to output
/// 3. Generate a JS proxy that exports the class name mapping
/// 4. Create a modified page source with CSS module imports rewritten to proxy imports
pub(crate) fn process_css_modules(
    scan: &ScanResult,
    output_dir: &Path,
    build_id: &str,
) -> Result<CssModuleProcessing> {
    let hash_prefix = &build_id[..8];
    let temp_dir = output_dir.join("_css_modules");
    fs::create_dir_all(&temp_dir)?;

    let mut page_overrides = HashMap::new();
    let mut route_css: HashMap<String, Vec<String>> = HashMap::new();
    let mut global_css = Vec::new();

    // Track processed CSS module files to avoid duplicating work
    let mut processed_css: HashMap<PathBuf, (String, HashMap<String, String>)> = HashMap::new();

    // Collect all source files to scan: (abs_path, route_pattern or None for _app)
    let mut sources: Vec<(&PathBuf, Option<&str>)> = Vec::new();
    for route in &scan.routes {
        sources.push((&route.abs_path, Some(&route.pattern)));
    }
    if let Some(app) = &scan.app {
        sources.push((&app.abs_path, None));
    }

    for (source_path, route_pattern) in &sources {
        let css_module_imports = find_css_module_imports(source_path)?;
        if css_module_imports.is_empty() {
            continue;
        }

        let source_dir = source_path.parent().unwrap_or(Path::new("."));
        let mut source_content = fs::read_to_string(source_path)?;
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
        let modified_name = source_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        // Use a unique name to avoid collisions between pages in different dirs
        let unique_name = format!("{}_{}", css_module_hash(source_path), modified_name);
        let modified_path = temp_dir.join(&unique_name);
        fs::write(&modified_path, &source_content)?;

        page_overrides.insert((*source_path).clone(), modified_path);

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

/// Find `.module.css` imports in a source file.
/// Returns: Vec of (import_specifier, resolved_absolute_path).
fn find_css_module_imports(source_path: &Path) -> Result<Vec<(String, PathBuf)>> {
    let source = fs::read_to_string(source_path)?;
    let parent = source_path.parent().unwrap_or(Path::new("."));
    let mut results = Vec::new();

    for line in source.lines() {
        let trimmed = line.trim();
        // Match: import X from './path.module.css'
        if trimmed.starts_with("import ") {
            if let Some(specifier) = extract_import_from_specifier(trimmed) {
                if specifier.ends_with(".module.css") {
                    let abs_path = parent.join(specifier);
                    if abs_path.exists() {
                        results.push((specifier.to_string(), abs_path));
                    }
                }
            }
        }
    }

    Ok(results)
}

/// Extract the `from` specifier from an import statement.
/// E.g. `import styles from './Button.module.css';` → `./Button.module.css`
fn extract_import_from_specifier(line: &str) -> Option<&str> {
    // Look for `from '...'` or `from "..."`
    let from_pos = line.find("from ")?;
    let after_from = &line[from_pos + 5..];
    let trimmed = after_from.trim();
    let quote_char = trimmed.chars().next()?;
    if quote_char != '\'' && quote_char != '"' {
        return None;
    }
    let inner = &trimmed[1..];
    let end = inner.find(quote_char)?;
    Some(&inner[..end])
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
pub(crate) fn absolutize_relative_imports(source: &str, source_dir: &Path) -> String {
    let mut result = String::new();

    for line in source.lines() {
        let trimmed = line.trim();

        // Handle: import X from './relative' or import X from '../relative'
        if trimmed.starts_with("import ") || trimmed.starts_with("export ") {
            if let Some(from_pos) = trimmed.find("from ") {
                let after_from = &trimmed[from_pos + 5..];
                let after_from_trimmed = after_from.trim();
                if let Some(quote_char) = after_from_trimmed.chars().next() {
                    if (quote_char == '\'' || quote_char == '"') && after_from_trimmed.len() > 1 {
                        let inner = &after_from_trimmed[1..];
                        if let Some(end) = inner.find(quote_char) {
                            let specifier = &inner[..end];
                            if specifier.starts_with("./") || specifier.starts_with("../") {
                                let abs = source_dir.join(specifier);
                                let abs_str = abs.to_string_lossy().replace('\\', "/");
                                let new_line = format!(
                                    "{}{}{}{}{}",
                                    &trimmed[..from_pos + 5],
                                    quote_char,
                                    abs_str,
                                    quote_char,
                                    &inner[end + 1..]
                                );
                                result.push_str(&new_line);
                                result.push('\n');
                                continue;
                            }
                        }
                    }
                }
            }
            // Handle side-effect imports: import './foo.css'
            if trimmed.starts_with("import '") || trimmed.starts_with("import \"") {
                let quote_char = if trimmed.starts_with("import '") {
                    '\''
                } else {
                    '"'
                };
                let after_quote = &trimmed[8..]; // after `import '` or `import "`
                if let Some(end) = after_quote.find(quote_char) {
                    let specifier = &after_quote[..end];
                    if specifier.starts_with("./") || specifier.starts_with("../") {
                        let abs = source_dir.join(specifier);
                        let abs_str = abs.to_string_lossy().replace('\\', "/");
                        let new_line = format!(
                            "import {quote_char}{abs_str}{quote_char}{}",
                            &after_quote[end + 1..]
                        );
                        result.push_str(&new_line);
                        result.push('\n');
                        continue;
                    }
                }
            }
        }

        result.push_str(line);
        result.push('\n');
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
    fn test_extract_import_specifier_single_quotes() {
        let line = "import styles from './Button.module.css';";
        assert_eq!(
            extract_import_from_specifier(line),
            Some("./Button.module.css")
        );
    }

    #[test]
    fn test_extract_import_specifier_double_quotes() {
        let line = r#"import styles from "./Button.module.css";"#;
        assert_eq!(
            extract_import_from_specifier(line),
            Some("./Button.module.css")
        );
    }

    #[test]
    fn test_extract_import_specifier_no_from() {
        let line = "import './globals.css';";
        assert_eq!(extract_import_from_specifier(line), None);
    }

    #[test]
    fn test_absolutize_relative_imports() {
        let source = "import Foo from './foo';\nimport React from 'react';\n";
        let dir = Path::new("/project/src");
        let result = absolutize_relative_imports(source, dir);
        // join("./foo") produces "/project/src/./foo" which is fine — rolldown resolves it
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
}
