#![allow(clippy::unwrap_used)]

mod common;

use common::{build_and_load, setup_mock_node_modules};
use rex_core::{PageType, RexConfig, Route};
use rex_router::ScanResult;
use std::fs;
use std::path::PathBuf;

#[tokio::test]
async fn test_integration_mdx_basic_ssr() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();

    setup_mock_node_modules(&root);

    let pages_dir = root.join("pages");
    fs::create_dir_all(&pages_dir).unwrap();

    let index_path = pages_dir.join("index.mdx");
    fs::write(
        &index_path,
        "# Hello MDX\n\nThis is **bold** and *italic* content.\n",
    )
    .unwrap();

    let config = RexConfig::new(root).with_dev(true);
    let scan = ScanResult {
        routes: vec![Route {
            pattern: "/".to_string(),
            file_path: PathBuf::from("index.mdx"),
            abs_path: index_path,
            dynamic_segments: vec![],
            page_type: PageType::Regular,
            specificity: 10,
        }],
        api_routes: vec![],
        app: None,
        document: None,
        error: None,
        not_found: None,
        middleware: None,
        app_scan: None,
        mcp_tools: vec![],
    };

    let (_result, pool) = build_and_load(&config, &scan).await;

    let render = pool
        .execute(|iso| iso.render_page("index", "{}"))
        .await
        .unwrap()
        .unwrap();

    assert!(
        render.body.contains("Hello MDX"),
        "MDX heading should render: {}",
        render.body
    );
    assert!(
        render.body.contains("<strong>bold</strong>"),
        "MDX bold should render as <strong>: {}",
        render.body
    );
    assert!(
        render.body.contains("<em>italic</em>"),
        "MDX italic should render as <em>: {}",
        render.body
    );
}

#[tokio::test]
async fn test_integration_mdx_with_frontmatter() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();

    setup_mock_node_modules(&root);

    let pages_dir = root.join("pages");
    fs::create_dir_all(&pages_dir).unwrap();

    let index_path = pages_dir.join("index.mdx");
    fs::write(
        &index_path,
        "---\ntitle: My Page\ncount: 42\nrating: 0.5\n---\n\n# {frontmatter.title}\n\nCount: {frontmatter.count}, Rating: {frontmatter.rating}\n",
    )
    .unwrap();

    let config = RexConfig::new(root).with_dev(true);
    let scan = ScanResult {
        routes: vec![Route {
            pattern: "/".to_string(),
            file_path: PathBuf::from("index.mdx"),
            abs_path: index_path,
            dynamic_segments: vec![],
            page_type: PageType::Regular,
            specificity: 10,
        }],
        api_routes: vec![],
        app: None,
        document: None,
        error: None,
        not_found: None,
        middleware: None,
        app_scan: None,
        mcp_tools: vec![],
    };

    let (_result, pool) = build_and_load(&config, &scan).await;

    let render = pool
        .execute(|iso| iso.render_page("index", "{}"))
        .await
        .unwrap()
        .unwrap();

    assert!(
        render.body.contains("My Page"),
        "MDX frontmatter title should render: {}",
        render.body
    );
    assert!(
        render.body.contains("42"),
        "MDX frontmatter count should render: {}",
        render.body
    );
    assert!(
        render.body.contains("0.5"),
        "MDX frontmatter decimal should render: {}",
        render.body
    );
}

#[tokio::test]
async fn test_integration_mdx_with_jsx_component() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();

    setup_mock_node_modules(&root);

    let pages_dir = root.join("pages");
    let components_dir = root.join("components");
    fs::create_dir_all(&pages_dir).unwrap();
    fs::create_dir_all(&components_dir).unwrap();

    // Create a React component the MDX file will import
    fs::write(
        components_dir.join("Alert.tsx"),
        r#"export default function Alert({ children }) {
    return <div className="alert">{children}</div>;
}
"#,
    )
    .unwrap();

    let index_path = pages_dir.join("index.mdx");
    fs::write(
        &index_path,
        "import Alert from '../components/Alert'\n\n# MDX with Components\n\n<Alert>This is an alert!</Alert>\n",
    )
    .unwrap();

    let config = RexConfig::new(root).with_dev(true);
    let scan = ScanResult {
        routes: vec![Route {
            pattern: "/".to_string(),
            file_path: PathBuf::from("index.mdx"),
            abs_path: index_path,
            dynamic_segments: vec![],
            page_type: PageType::Regular,
            specificity: 10,
        }],
        api_routes: vec![],
        app: None,
        document: None,
        error: None,
        not_found: None,
        middleware: None,
        app_scan: None,
        mcp_tools: vec![],
    };

    let (_result, pool) = build_and_load(&config, &scan).await;

    let render = pool
        .execute(|iso| iso.render_page("index", "{}"))
        .await
        .unwrap()
        .unwrap();

    assert!(
        render.body.contains("MDX with Components"),
        "MDX heading should render: {}",
        render.body
    );
    assert!(
        render.body.contains("This is an alert!"),
        "Imported JSX component should render: {}",
        render.body
    );
    assert!(
        render.body.contains("alert"),
        "Component className should appear: {}",
        render.body
    );
}
