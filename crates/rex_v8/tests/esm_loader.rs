//! Tests for the ESM module registry (esm_loader.rs).
#![allow(clippy::unwrap_used)]

use rex_v8::EsmModuleRegistry;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

fn make_registry(aliases: Vec<(&str, &str)>, sources: Vec<(&str, &str)>) -> EsmModuleRegistry {
    let alias_map: HashMap<String, PathBuf> = aliases
        .into_iter()
        .map(|(k, v)| (k.to_string(), PathBuf::from(v)))
        .collect();
    let source_map: HashMap<PathBuf, String> = sources
        .into_iter()
        .map(|(k, v)| (PathBuf::from(k), v.to_string()))
        .collect();
    EsmModuleRegistry::new(
        Arc::new("// dep iife".to_string()),
        source_map,
        alias_map,
        PathBuf::from("/project"),
    )
}

#[test]
fn registry_resolve_alias() {
    let reg = make_registry(
        vec![
            ("react", "/vendor/react.js"),
            ("rex/head", "/runtime/server/head.ts"),
        ],
        vec![],
    );
    assert_eq!(
        reg.resolve("react", &PathBuf::from("/project/pages/index.tsx")),
        Some(PathBuf::from("/vendor/react.js"))
    );
    assert_eq!(
        reg.resolve("rex/head", &PathBuf::from("/project/pages/index.tsx")),
        Some(PathBuf::from("/runtime/server/head.ts"))
    );
}

#[test]
fn registry_resolve_relative() {
    // Relative imports resolve based on referrer's parent directory.
    // Since the files don't exist on disk, resolve returns None for relative paths.
    // This test verifies the code path is exercised (it tries the file system).
    let reg = make_registry(vec![], vec![]);
    let result = reg.resolve("./utils", &PathBuf::from("/project/pages/index.tsx"));
    // File doesn't exist on disk, so resolve returns None
    assert_eq!(result, None);
}

#[test]
fn registry_resolve_css_sentinel() {
    let reg = make_registry(vec![], vec![]);
    let referrer = PathBuf::from("/project/pages/index.tsx");

    assert_eq!(
        reg.resolve("styles.css", &referrer),
        Some(PathBuf::from("__empty_css__"))
    );
    assert_eq!(
        reg.resolve("theme.scss", &referrer),
        Some(PathBuf::from("__empty_css__"))
    );
    assert_eq!(
        reg.resolve("vars.sass", &referrer),
        Some(PathBuf::from("__empty_css__"))
    );
    assert_eq!(
        reg.resolve("mixins.less", &referrer),
        Some(PathBuf::from("__empty_css__"))
    );
}

#[test]
fn registry_build_entry_source() {
    let reg = make_registry(vec![], vec![]);
    let page_sources = vec![
        (
            "index".to_string(),
            PathBuf::from("/project/pages/index.tsx"),
        ),
        (
            "about".to_string(),
            PathBuf::from("/project/pages/about.tsx"),
        ),
    ];
    let entry = reg.build_entry_source(&page_sources);

    assert!(
        entry.contains("globalThis.__rex_pages = {};"),
        "should initialize page registry"
    );
    assert!(
        entry.contains("import * as __page0 from '/project/pages/index.tsx'"),
        "should import page 0"
    );
    assert!(
        entry.contains("globalThis.__rex_pages['index'] = __page0"),
        "should register page 0 as 'index'"
    );
    assert!(
        entry.contains("import * as __page1 from '/project/pages/about.tsx'"),
        "should import page 1"
    );
    assert!(
        entry.contains("globalThis.__rex_pages['about'] = __page1"),
        "should register page 1 as 'about'"
    );
    assert!(
        entry.contains("__rex_createElement"),
        "should reference createElement from globals"
    );
}

#[test]
fn registry_update_source() {
    let mut reg = make_registry(vec![], vec![("/project/pages/index.tsx", "var x = 1;")]);
    assert_eq!(
        reg.get_source(&PathBuf::from("/project/pages/index.tsx")),
        Some("var x = 1;")
    );

    reg.update_source(
        PathBuf::from("/project/pages/index.tsx"),
        "var x = 2;".to_string(),
    );
    assert_eq!(
        reg.get_source(&PathBuf::from("/project/pages/index.tsx")),
        Some("var x = 2;")
    );

    // Update also works for new paths
    reg.update_source(
        PathBuf::from("/project/pages/new.tsx"),
        "var y = 3;".to_string(),
    );
    assert_eq!(
        reg.get_source(&PathBuf::from("/project/pages/new.tsx")),
        Some("var y = 3;")
    );
}

#[test]
fn registry_resolve_unknown_bare_specifier() {
    // Bare specifiers not in aliases should return None
    let reg = make_registry(vec![("react", "/vendor/react.js")], vec![]);
    let result = reg.resolve("lodash", &PathBuf::from("/project/pages/index.tsx"));
    assert_eq!(result, None, "unknown bare specifier should return None");
}

#[test]
fn registry_resolve_module_css_sentinel() {
    let reg = make_registry(vec![], vec![]);
    let referrer = PathBuf::from("/project/pages/index.tsx");

    assert_eq!(
        reg.resolve("styles.module.css", &referrer),
        Some(PathBuf::from("__empty_css__")),
        ".module.css should also map to sentinel"
    );
}

#[test]
fn registry_build_entry_includes_ssr_runtime() {
    let reg = make_registry(vec![], vec![]);
    let page_sources = vec![(
        "index".to_string(),
        PathBuf::from("/project/pages/index.tsx"),
    )];
    let entry = reg.build_entry_source(&page_sources);

    // The entry should contain the SSR runtime setup
    assert!(
        entry.contains("__rex_pages"),
        "entry should set up page registry"
    );
    assert!(
        entry.contains("import * as __page0"),
        "entry should import first page"
    );
}

#[test]
fn registry_build_entry_empty_pages() {
    let reg = make_registry(vec![], vec![]);
    let page_sources: Vec<(String, PathBuf)> = vec![];
    let entry = reg.build_entry_source(&page_sources);

    // Even with no pages, entry should set up globals
    assert!(
        entry.contains("globalThis.__rex_pages = {}"),
        "entry should initialize page registry even with no pages"
    );
}

#[test]
fn registry_get_source_nonexistent() {
    let reg = make_registry(vec![], vec![]);
    assert_eq!(
        reg.get_source(&PathBuf::from("/project/pages/missing.tsx")),
        None,
        "nonexistent path should return None"
    );
}

#[test]
fn registry_dep_iife() {
    let dep_iife = "globalThis.__rex_React = {};".to_string();
    let reg = EsmModuleRegistry::new(
        Arc::new(dep_iife.clone()),
        HashMap::new(),
        HashMap::new(),
        PathBuf::from("/project"),
    );
    assert_eq!(reg.dep_iife(), &dep_iife, "dep IIFE should be accessible");
}
