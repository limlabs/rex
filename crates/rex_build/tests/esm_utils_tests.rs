//! Tests for ESM dev mode utilities (esm_utils.rs).
#![allow(clippy::unwrap_used)]

use rex_build::esm_utils;
use rex_core::{DataStrategy, PageType, Route};
use std::path::PathBuf;

#[test]
fn detect_data_strategy_gssp() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("page.tsx");
    std::fs::write(
        &path,
        r#"
export function getServerSideProps() { return { props: {} }; }
export default function Page() { return null; }
"#,
    )
    .unwrap();

    assert_eq!(
        esm_utils::detect_data_strategy(&path),
        DataStrategy::GetServerSideProps
    );
}

#[test]
fn detect_data_strategy_gsp() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("page.tsx");
    std::fs::write(
        &path,
        r#"
export function getStaticProps() { return { props: {} }; }
export default function Page() { return null; }
"#,
    )
    .unwrap();

    assert_eq!(
        esm_utils::detect_data_strategy(&path),
        DataStrategy::GetStaticProps
    );
}

#[test]
fn detect_data_strategy_none() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("page.tsx");
    std::fs::write(&path, "export default function Page() { return null; }").unwrap();

    assert_eq!(esm_utils::detect_data_strategy(&path), DataStrategy::None);
}

#[test]
fn detect_data_strategy_missing_file() {
    // Missing file should return None (no panic)
    assert_eq!(
        esm_utils::detect_data_strategy(&PathBuf::from("/nonexistent/page.tsx")),
        DataStrategy::None
    );
}

#[test]
fn esm_page_sources_builds_pairs() {
    let scan = rex_router::ScanResult {
        routes: vec![
            Route {
                pattern: "/".to_string(),
                file_path: PathBuf::from("pages/index.tsx"),
                abs_path: PathBuf::from("/project/pages/index.tsx"),
                dynamic_segments: vec![],
                page_type: PageType::Regular,
                specificity: 0,
            },
            Route {
                pattern: "/about".to_string(),
                file_path: PathBuf::from("pages/about.tsx"),
                abs_path: PathBuf::from("/project/pages/about.tsx"),
                dynamic_segments: vec![],
                page_type: PageType::Regular,
                specificity: 0,
            },
        ],
        api_routes: vec![],
        app: None,
        document: None,
        error: None,
        not_found: None,
        middleware: None,
        app_scan: None,
        mcp_tools: vec![],
    };

    let pairs = esm_utils::esm_page_sources(&scan);
    assert_eq!(pairs.len(), 2);
    // module_name() includes the directory path (e.g., "pages/index")
    assert!(pairs[0].0.contains("index"), "first page should be index");
    assert_eq!(pairs[0].1, PathBuf::from("/project/pages/index.tsx"));
    assert!(pairs[1].0.contains("about"), "second page should be about");
    assert_eq!(pairs[1].1, PathBuf::from("/project/pages/about.tsx"));
}
