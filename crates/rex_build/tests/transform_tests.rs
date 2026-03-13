//! Tests for OXC runtime transform (transform.rs).
#![allow(clippy::unwrap_used)]

use rex_build::transform::{transform_to_esm, TransformCache};
use std::path::PathBuf;

#[test]
fn transform_tsx_strips_types() {
    let source = r#"
interface Props { name: string; count: number; }
const greet = (p: Props): string => p.name;
export default greet;
"#;
    let result = transform_to_esm(source, "test.tsx").unwrap();
    assert!(
        !result.contains("interface"),
        "interface should be stripped"
    );
    assert!(
        !result.contains(": string"),
        "type annotations should be stripped"
    );
    assert!(
        !result.contains(": Props"),
        "type annotations should be stripped"
    );
    assert!(result.contains("greet"), "identifiers should be preserved");
}

#[test]
fn transform_jsx_to_create_element() {
    let source = r#"
import React from 'react';
export default function Page() { return <div><h1>Hello</h1></div>; }
"#;
    let result = transform_to_esm(source, "test.jsx").unwrap();
    assert!(
        result.contains("createElement") || result.contains("_jsx"),
        "JSX should be transformed, got: {result}"
    );
    assert!(!result.contains("<div>"), "raw JSX tags should be removed");
}

#[test]
fn transform_css_imports_stripped() {
    let source = r#"
import './styles.css';
import styles from './foo.module.css';
import './theme.scss';
const x = 1;
"#;
    let result = transform_to_esm(source, "test.ts").unwrap();
    assert!(
        !result.contains("styles.css"),
        "CSS import should be stripped"
    );
    assert!(
        !result.contains("foo.module.css"),
        "CSS module import should be stripped"
    );
    assert!(
        !result.contains("theme.scss"),
        "SCSS import should be stripped"
    );
    assert!(result.contains("const x = 1"), "non-CSS code should remain");
}

#[test]
fn transform_cache_hit() {
    let cache = TransformCache::new();
    let path = PathBuf::from("/tmp/test_cache.tsx");
    let source = "const x: number = 1;";

    let first = cache.transform(&path, source).unwrap();
    let second = cache.transform(&path, source).unwrap();
    assert_eq!(first, second, "same source should produce identical output");
    // Verify the cached value is accessible
    assert!(
        cache.get_cached(&path).is_some(),
        "cache entry should exist"
    );
}

#[test]
fn transform_cache_invalidate() {
    let cache = TransformCache::new();
    let path = PathBuf::from("/tmp/test_cache_inv.tsx");

    let source_v1 = "const x: number = 1;";
    let source_v2 = "const y: string = 'hello';";

    let out_v1 = cache.transform(&path, source_v1).unwrap();
    assert!(out_v1.contains("x"), "v1 output should contain x");

    // Same path, different source triggers re-transform
    let out_v2 = cache.transform(&path, source_v2).unwrap();
    assert!(out_v2.contains("y"), "v2 output should contain y");
    assert!(
        !out_v2.contains(": string"),
        "types should be stripped in v2"
    );

    // Explicit invalidate then re-transform
    cache.invalidate(&path);
    assert!(
        cache.get_cached(&path).is_none(),
        "cache should be empty after invalidate"
    );
}

#[test]
fn transform_preserves_esm_exports() {
    let source = r#"
export const name = 'test';
export function handler() { return 42; }
export default function Page() { return null; }
"#;
    let result = transform_to_esm(source, "test.ts").unwrap();
    assert!(result.contains("export"), "ESM exports should be preserved");
    assert!(
        result.contains("handler"),
        "named export should be preserved"
    );
}

#[test]
fn transform_unknown_extension_fails() {
    let result = transform_to_esm("const x = 1;", "test.py");
    assert!(result.is_err(), "unknown extension should fail");
}

#[test]
fn transform_plain_js_passthrough() {
    let source = "const x = 1;\nexport default x;\n";
    let result = transform_to_esm(source, "test.js").unwrap();
    assert!(
        result.contains("const x = 1"),
        "plain JS should pass through"
    );
    assert!(
        result.contains("export default"),
        "exports should be preserved"
    );
}

#[test]
fn transform_less_import_stripped() {
    let source = "import './variables.less';\nconst y = 2;\n";
    let result = transform_to_esm(source, "test.ts").unwrap();
    assert!(
        !result.contains("variables.less"),
        ".less import should be stripped"
    );
    assert!(result.contains("const y = 2"), "non-CSS code should remain");
}

#[test]
fn transform_sass_import_stripped() {
    let source = "import './mixins.sass';\nconst z = 3;\n";
    let result = transform_to_esm(source, "test.ts").unwrap();
    assert!(
        !result.contains("mixins.sass"),
        ".sass import should be stripped"
    );
}
