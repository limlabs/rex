//! MDX helper functions: ESM extraction, YAML frontmatter conversion, and
//! mdx-components file discovery.

use oxc_allocator::Allocator;
use oxc_ast::ast::Statement;
use oxc_span::{GetSpan, SourceType};
use std::path::Path;

/// Use OXC's error-recovering parser to extract ESM import/export statements
/// from the top of an MDX file.
///
/// Returns: (esm_lines, user_default_export_expression, content_start_offset).
pub fn extract_esm(source: &str) -> (Vec<String>, Option<String>, usize) {
    let allocator = Allocator::default();
    let ret = oxc_parser::Parser::new(&allocator, source, SourceType::jsx()).parse();

    let mut esm_lines = Vec::new();
    let mut user_default: Option<String> = None;
    let mut max_end: usize = 0;

    for stmt in &ret.program.body {
        match stmt {
            Statement::ImportDeclaration(import) => {
                let span = import.span();
                let text = &source[span.start as usize..span.end as usize];
                esm_lines.push(text.to_string());
                if (span.end as usize) > max_end {
                    max_end = span.end as usize;
                }
            }
            Statement::ExportNamedDeclaration(export) => {
                let span = export.span();
                let text = &source[span.start as usize..span.end as usize];
                esm_lines.push(text.to_string());
                if (span.end as usize) > max_end {
                    max_end = span.end as usize;
                }
            }
            Statement::ExportDefaultDeclaration(export) => {
                // Capture user default export for layout wrapping.
                // Extract the expression/declaration text (skip "export default ").
                let span = export.span();
                let full = &source[span.start as usize..span.end as usize];
                let expr = full
                    .strip_prefix("export default ")
                    .unwrap_or(full)
                    .trim_end_matches(';')
                    .trim();
                if !expr.is_empty() {
                    user_default = Some(expr.to_string());
                }
                if (span.end as usize) > max_end {
                    max_end = span.end as usize;
                }
            }
            Statement::ExportAllDeclaration(export) => {
                let span = export.span();
                let text = &source[span.start as usize..span.end as usize];
                esm_lines.push(text.to_string());
                if (span.end as usize) > max_end {
                    max_end = span.end as usize;
                }
            }
            _ => break,
        }
    }

    // Skip trailing whitespace after the last ESM statement
    let bytes = source.as_bytes();
    let mut content_start = max_end;
    while content_start < bytes.len()
        && (bytes[content_start] == b'\n'
            || bytes[content_start] == b'\r'
            || bytes[content_start] == b' '
            || bytes[content_start] == b'\t')
    {
        content_start += 1;
    }

    (esm_lines, user_default, content_start)
}

/// Resolve an MDX JSX tag name to a createElement first argument.
pub(crate) fn jsx_tag(name: &Option<String>) -> String {
    match name {
        Some(name) if name.starts_with(|c: char| c.is_uppercase()) => name.clone(),
        Some(name) => format!("'{name}'"),
        None => "'div'".to_string(), // Fragment
    }
}

/// Convert MDX JSX attributes to a props object expression.
pub(crate) fn mdx_attrs_to_props(attrs: &[markdown::mdast::AttributeContent]) -> String {
    if attrs.is_empty() {
        return "null".to_string();
    }

    let mut pairs = Vec::new();
    for attr in attrs {
        match attr {
            markdown::mdast::AttributeContent::Property(prop) => {
                let key = &prop.name;
                let value = match &prop.value {
                    Some(markdown::mdast::AttributeValue::Literal(lit)) => jsx_string_literal(lit),
                    Some(markdown::mdast::AttributeValue::Expression(expr)) => expr.value.clone(),
                    None => "true".to_string(),
                };
                pairs.push(format!("{key}: {value}"));
            }
            markdown::mdast::AttributeContent::Expression(expr) => {
                pairs.push(format!("...{}", expr.value));
            }
        }
    }

    format!("{{ {} }}", pairs.join(", "))
}

/// Escape a string for use as a JavaScript string literal.
pub(crate) fn jsx_string_literal(s: &str) -> String {
    let escaped = s
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t");
    format!("'{escaped}'")
}

/// Convert simple YAML frontmatter to a JS object literal.
///
/// Handles flat key: value pairs (strings, numbers, booleans, arrays of scalars).
/// For complex nested YAML, falls back to a string representation.
pub fn yaml_to_js_object(yaml: &str) -> String {
    let mut pairs = Vec::new();

    for line in yaml.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            let value = value.trim();

            if key.is_empty() {
                continue;
            }

            let js_value = yaml_value_to_js(value);
            // Quote keys that aren't valid JS identifiers
            if is_valid_js_ident(key) {
                pairs.push(format!("{key}: {js_value}"));
            } else {
                pairs.push(format!("{}: {js_value}", jsx_string_literal(key)));
            }
        }
    }

    if pairs.is_empty() {
        "{}".to_string()
    } else {
        format!("{{ {} }}", pairs.join(", "))
    }
}

/// Convert a YAML scalar value to a JS expression.
fn yaml_value_to_js(value: &str) -> String {
    // Empty value → null
    if value.is_empty() {
        return "null".to_string();
    }

    // Boolean
    match value {
        "true" | "True" | "TRUE" | "yes" | "Yes" | "YES" | "on" | "On" | "ON" => {
            return "true".to_string()
        }
        "false" | "False" | "FALSE" | "no" | "No" | "NO" | "off" | "Off" | "OFF" => {
            return "false".to_string()
        }
        "null" | "Null" | "NULL" | "~" => return "null".to_string(),
        _ => {}
    }

    // Number
    if value.parse::<f64>().is_ok() && !value.starts_with('0') || value == "0" {
        return value.to_string();
    }

    // Inline array: [a, b, c]
    if value.starts_with('[') && value.ends_with(']') {
        let inner = &value[1..value.len() - 1];
        let items: Vec<String> = inner
            .split(',')
            .map(|s| yaml_value_to_js(s.trim()))
            .collect();
        return format!("[{}]", items.join(", "));
    }

    // Quoted string
    if (value.starts_with('"') && value.ends_with('"'))
        || (value.starts_with('\'') && value.ends_with('\''))
    {
        let inner = &value[1..value.len() - 1];
        return jsx_string_literal(inner);
    }

    // Bare string
    jsx_string_literal(value)
}

/// Check if a string is a valid JS identifier (simple check).
fn is_valid_js_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' || c == '$' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// Find `mdx-components.{tsx,ts,jsx,js}` in the project root.
pub fn find_mdx_components(project_root: &Path) -> Option<String> {
    for ext in &["tsx", "ts", "jsx", "js"] {
        let path = project_root.join(format!("mdx-components.{ext}"));
        if path.exists() {
            return Some(path.to_string_lossy().to_string());
        }
    }
    // Also check src/ directory
    let src = project_root.join("src");
    if src.exists() {
        for ext in &["tsx", "ts", "jsx", "js"] {
            let path = src.join(format!("mdx-components.{ext}"));
            if path.exists() {
                return Some(path.to_string_lossy().to_string());
            }
        }
    }
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn extract_esm_imports() {
        let source = "import Foo from './Foo'\nimport Bar from './Bar'\n\n# Hello\n";
        let (esm, default_export, offset) = extract_esm(source);
        assert_eq!(esm.len(), 2);
        assert!(esm[0].contains("Foo"));
        assert!(esm[1].contains("Bar"));
        assert!(default_export.is_none());
        assert!(source[offset..].starts_with('#'));
    }

    #[test]
    fn extract_esm_none() {
        let source = "# Just markdown\n\nNo imports here.\n";
        let (esm, default_export, offset) = extract_esm(source);
        assert!(esm.is_empty());
        assert!(default_export.is_none());
        assert_eq!(offset, 0);
    }

    #[test]
    fn extract_esm_stops_at_non_esm() {
        let source = "import Foo from './Foo'\nconst x = 1\nexport const y = 2\n";
        let (esm, _default, _offset) = extract_esm(source);
        assert_eq!(esm.len(), 1, "Should stop at first non-ESM: {esm:?}");
        assert!(esm[0].contains("Foo"));
    }

    #[test]
    fn yaml_to_js_basic() {
        let js = yaml_to_js_object("title: Hello\ncount: 5\ndraft: true\n");
        assert!(js.contains("title: 'Hello'"));
        assert!(js.contains("count: 5"));
        assert!(js.contains("draft: true"));
    }

    #[test]
    fn yaml_to_js_empty() {
        assert_eq!(yaml_to_js_object(""), "{}");
        assert_eq!(yaml_to_js_object("# just a comment"), "{}");
    }

    #[test]
    fn yaml_to_js_arrays() {
        let js = yaml_to_js_object("tags: [react, mdx, next]");
        assert!(js.contains("tags: ['react', 'mdx', 'next']"));
    }

    #[test]
    fn yaml_to_js_quoted_strings() {
        let js = yaml_to_js_object("title: \"Hello World\"\nsubtitle: 'Sub'");
        assert!(js.contains("title: 'Hello World'"));
        assert!(js.contains("subtitle: 'Sub'"));
    }

    #[test]
    fn yaml_to_js_null_values() {
        let js = yaml_to_js_object("empty:\nnull_val: null\ntilde: ~");
        assert!(js.contains("empty: null"));
        assert!(js.contains("null_val: null"));
        assert!(js.contains("tilde: null"));
    }

    #[test]
    fn find_mdx_components_not_found() {
        let tmp = std::env::temp_dir().join("rex_test_no_mdx_components");
        let _ = std::fs::create_dir_all(&tmp);
        assert!(find_mdx_components(&tmp).is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn find_mdx_components_found() {
        let tmp = std::env::temp_dir().join("rex_test_mdx_components_found");
        let _ = std::fs::create_dir_all(&tmp);
        let file = tmp.join("mdx-components.tsx");
        std::fs::write(&file, "export function useMDXComponents(c) { return c; }").unwrap();
        let result = find_mdx_components(&tmp);
        assert!(result.is_some());
        assert!(result.unwrap().contains("mdx-components.tsx"));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
