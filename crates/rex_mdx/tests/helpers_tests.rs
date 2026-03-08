//! Integration tests for MDX helper functions (public API).
#![allow(clippy::unwrap_used)]

use rex_mdx::{extract_esm, find_mdx_components, yaml_to_js_object};

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
fn extract_esm_collects_past_non_esm() {
    let source = "import Foo from './Foo'\nconst x = 1\nexport const y = 2\n";
    let (esm, _default, offset) = extract_esm(source);
    assert_eq!(esm.len(), 2, "Should collect ESM past non-ESM gap: {esm:?}");
    assert!(esm[0].contains("Foo"));
    assert!(esm[1].contains("y = 2"));
    // content_start should be after the initial contiguous ESM (just the import)
    assert!(
        source[offset..].starts_with("const"),
        "content_start should be at first non-ESM: '{}'",
        &source[offset..]
    );
}

#[test]
fn yaml_to_js_basic() {
    let js = yaml_to_js_object("title: Hello\ncount: 5\ndraft: true\n");
    assert!(js.contains("title: 'Hello'"));
    assert!(js.contains("count: 5"));
    assert!(js.contains("draft: true"));
}

#[test]
fn yaml_to_js_decimal_numbers() {
    let js = yaml_to_js_object("rating: 0.5\nzero: 0\npi: 3.14\noctal_like: 0123");
    assert!(js.contains("rating: 0.5"), "0.5 should be a number: {js}");
    assert!(js.contains("zero: 0"), "0 should be a number: {js}");
    assert!(js.contains("pi: 3.14"), "3.14 should be a number: {js}");
    assert!(
        js.contains("octal_like: '0123'"),
        "0123 should be a string: {js}"
    );
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
