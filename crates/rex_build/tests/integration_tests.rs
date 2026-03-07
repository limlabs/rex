#![allow(clippy::unwrap_used)]

mod common;

use common::{
    build_and_load, build_and_load_with_root, setup_mock_node_modules, setup_test_project,
};
use rex_core::{PageType, RexConfig, Route};
use rex_router::ScanResult;
use std::fs;
use std::path::PathBuf;

#[tokio::test]
async fn test_integration_basic_ssr() {
    let (_tmp, config, scan) = setup_test_project(
        &[(
            "index.tsx",
            r#"
                export default function Home() {
                    return <div><h1>Hello Rex</h1><p>Welcome</p></div>;
                }
                "#,
        )],
        None,
    );

    let (_result, pool) = build_and_load(&config, &scan).await;

    let render = pool
        .execute(|iso| iso.render_page("index", "{}"))
        .await
        .expect("pool execute")
        .expect("render_page");

    assert!(
        render.body.contains("Hello Rex"),
        "SSR should render heading: {}",
        render.body
    );
    assert!(
        render.body.contains("Welcome"),
        "SSR should render paragraph: {}",
        render.body
    );
    assert!(
        render.body.contains("<div>"),
        "SSR should produce HTML tags: {}",
        render.body
    );
}

#[tokio::test]
async fn test_integration_ssr_with_props() {
    let (_tmp, config, scan) = setup_test_project(
        &[(
            "index.tsx",
            r#"
                export default function Home({ message }) {
                    return <div><h1>{message}</h1></div>;
                }
                export function getServerSideProps() {
                    return { props: { message: "Dynamic content" } };
                }
                "#,
        )],
        None,
    );

    let (_result, pool) = build_and_load(&config, &scan).await;

    // Test GSSP
    let gssp_json = pool
        .execute(|iso| iso.get_server_side_props("index", "{\"params\":{},\"query\":{}}"))
        .await
        .expect("pool execute")
        .expect("gssp");

    let gssp: serde_json::Value = serde_json::from_str(&gssp_json).unwrap();
    assert_eq!(
        gssp["props"]["message"].as_str(),
        Some("Dynamic content"),
        "GSSP should return props"
    );

    // Test SSR with those props
    let render = pool
        .execute(|iso| iso.render_page("index", "{\"message\":\"Dynamic content\"}"))
        .await
        .expect("pool execute")
        .expect("render_page");

    assert!(
        render.body.contains("Dynamic content"),
        "SSR should render GSSP props: {}",
        render.body
    );
}

#[tokio::test]
async fn test_integration_multiple_pages() {
    let (_tmp, config, scan) = setup_test_project(
        &[
            (
                "index.tsx",
                r#"
                    export default function Home() {
                        return <div><h1>Home Page</h1></div>;
                    }
                    "#,
            ),
            (
                "about.tsx",
                r#"
                    export default function About() {
                        return <div><h1>About Page</h1></div>;
                    }
                    "#,
            ),
        ],
        None,
    );

    let (_result, pool) = build_and_load(&config, &scan).await;

    // Render home
    let home = pool
        .execute(|iso| iso.render_page("index", "{}"))
        .await
        .unwrap()
        .unwrap();
    assert!(home.body.contains("Home Page"), "home: {}", home.body);

    // Render about
    let about = pool
        .execute(|iso| iso.render_page("about", "{}"))
        .await
        .unwrap()
        .unwrap();
    assert!(about.body.contains("About Page"), "about: {}", about.body);
}

#[tokio::test]
async fn test_integration_css_module_in_ssr() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();

    setup_mock_node_modules(&root);

    let pages_dir = root.join("pages");
    let styles_dir = root.join("styles");
    fs::create_dir_all(&pages_dir).unwrap();
    fs::create_dir_all(&styles_dir).unwrap();

    fs::write(
        styles_dir.join("Home.module.css"),
        ".wrapper { padding: 20px; }\n.heading { color: blue; }\n",
    )
    .unwrap();

    let index_path = pages_dir.join("index.tsx");
    fs::write(
        &index_path,
        r#"import styles from '../styles/Home.module.css';
export default function Home() {
    return <div className={styles.wrapper}><h1 className={styles.heading}>Styled</h1></div>;
}
"#,
    )
    .unwrap();

    let config = RexConfig::new(root).with_dev(true);
    let scan = ScanResult {
        routes: vec![Route {
            pattern: "/".to_string(),
            file_path: PathBuf::from("index.tsx"),
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
        render.body.contains("Styled"),
        "should render page content: {}",
        render.body
    );
    // Scoped class names should appear in the HTML
    assert!(
        render.body.contains("Home_wrapper_"),
        "should have scoped class name for wrapper: {}",
        render.body
    );
    assert!(
        render.body.contains("Home_heading_"),
        "should have scoped class name for heading: {}",
        render.body
    );
}

#[tokio::test]
async fn test_integration_suspense_ssr() {
    let (_tmp, config, scan) = setup_test_project(
        &[(
            "index.tsx",
            r#"
                import { Suspense } from 'react';
                export default function Home() {
                    return (
                        <Suspense fallback={<div>Loading...</div>}>
                            <div><h1>Suspense Content</h1></div>
                        </Suspense>
                    );
                }
                "#,
        )],
        None,
    );

    let (_result, pool) = build_and_load(&config, &scan).await;

    let render = pool
        .execute(|iso| iso.render_page("index", "{}"))
        .await
        .unwrap()
        .unwrap();

    assert!(
        render.body.contains("Suspense Content"),
        "SSR should render Suspense children: {}",
        render.body
    );
    assert!(
        !render.body.contains("Loading..."),
        "SSR should NOT render fallback when children render normally: {}",
        render.body
    );
}

#[tokio::test]
async fn test_integration_fs_polyfill() {
    // Create a page that imports fs and uses readFileSync in GSSP
    let (_tmp, config, scan) = setup_test_project(
        &[(
            "index.tsx",
            r#"
                import fs from 'fs';
                export default function Home({ content }) {
                    return <div><h1>{content}</h1></div>;
                }
                export function getServerSideProps() {
                    const content = fs.readFileSync('data/message.txt', 'utf8');
                    return { props: { content } };
                }
                "#,
        )],
        None,
    );

    // Write the data file the page will read
    let data_dir = config.project_root.join("data");
    fs::create_dir_all(&data_dir).unwrap();
    fs::write(data_dir.join("message.txt"), "hello from file").unwrap();

    let (_result, pool) = build_and_load_with_root(&config, &scan).await;

    // Test GSSP reads the file
    let gssp_json = pool
        .execute(|iso| iso.get_server_side_props("index", "{\"params\":{},\"query\":{}}"))
        .await
        .expect("pool execute")
        .expect("gssp");

    let gssp: serde_json::Value = serde_json::from_str(&gssp_json).unwrap();
    assert_eq!(
        gssp["props"]["content"].as_str(),
        Some("hello from file"),
        "GSSP should read file content via fs polyfill: {gssp_json}"
    );

    // Test SSR renders the file content
    let render = pool
        .execute(|iso| iso.render_page("index", "{\"content\":\"hello from file\"}"))
        .await
        .expect("pool execute")
        .expect("render_page");

    assert!(
        render.body.contains("hello from file"),
        "SSR should render file content: {}",
        render.body
    );
}

#[tokio::test]
async fn test_integration_fs_promises_polyfill() {
    // Test the fs/promises shim with async getServerSideProps
    let (_tmp, config, scan) = setup_test_project(
        &[(
            "index.tsx",
            r#"
                import { readFile } from 'fs/promises';
                export default function Home({ content }) {
                    return <div><h1>{content}</h1></div>;
                }
                export async function getServerSideProps() {
                    const content = await readFile('data/async.txt', 'utf8');
                    return { props: { content } };
                }
                "#,
        )],
        None,
    );

    // Write the data file
    let data_dir = config.project_root.join("data");
    fs::create_dir_all(&data_dir).unwrap();
    fs::write(data_dir.join("async.txt"), "async file content").unwrap();

    let (_result, pool) = build_and_load_with_root(&config, &scan).await;

    // Test async GSSP reads via fs/promises
    let gssp_json = pool
        .execute(|iso| iso.get_server_side_props("index", "{\"params\":{},\"query\":{}}"))
        .await
        .expect("pool execute")
        .expect("gssp");

    let gssp: serde_json::Value = serde_json::from_str(&gssp_json).unwrap();
    assert_eq!(
        gssp["props"]["content"].as_str(),
        Some("async file content"),
        "Async GSSP should read file content via fs/promises polyfill: {gssp_json}"
    );
}

#[tokio::test]
async fn test_integration_fs_path_traversal_blocked() {
    // Verify that path traversal is blocked through the full pipeline
    let (_tmp, config, scan) = setup_test_project(
        &[(
            "index.tsx",
            r#"
                import fs from 'fs';
                export default function Home({ error }) {
                    return <div><h1>{error}</h1></div>;
                }
                export function getServerSideProps() {
                    try {
                        fs.readFileSync('../../etc/passwd', 'utf8');
                        return { props: { error: 'should have thrown' } };
                    } catch (e) {
                        return { props: { error: e.code || e.message } };
                    }
                }
                "#,
        )],
        None,
    );

    let (_result, pool) = build_and_load_with_root(&config, &scan).await;

    let gssp_json = pool
        .execute(|iso| iso.get_server_side_props("index", "{\"params\":{},\"query\":{}}"))
        .await
        .expect("pool execute")
        .expect("gssp");

    let gssp: serde_json::Value = serde_json::from_str(&gssp_json).unwrap();
    assert_eq!(
        gssp["props"]["error"].as_str(),
        Some("EACCES"),
        "Path traversal should be blocked: {gssp_json}"
    );
}

// -- path polyfill integration tests --

#[tokio::test]
async fn test_integration_path_polyfill() {
    // Test path.join, path.basename, path.dirname, path.extname via GSSP
    let (_tmp, config, scan) = setup_test_project(
        &[(
            "index.tsx",
            r#"
                import path from 'path';
                export default function Home(props) {
                    return <div>{JSON.stringify(props)}</div>;
                }
                export function getServerSideProps() {
                    return { props: {
                        joined: path.join('a', 'b', 'c'),
                        base: path.basename('/foo/bar.txt'),
                        baseExt: path.basename('/foo/bar.txt', '.txt'),
                        dir: path.dirname('/foo/bar.txt'),
                        ext: path.extname('/foo/bar.txt'),
                        normalized: path.join('a', '..', 'b', '.', 'c'),
                    }};
                }
                "#,
        )],
        None,
    );

    let (_result, pool) = build_and_load(&config, &scan).await;

    let gssp_json = pool
        .execute(|iso| iso.get_server_side_props("index", "{\"params\":{},\"query\":{}}"))
        .await
        .expect("pool execute")
        .expect("gssp");

    let gssp: serde_json::Value = serde_json::from_str(&gssp_json).unwrap();
    assert_eq!(gssp["props"]["joined"], "a/b/c", "path.join: {gssp_json}");
    assert_eq!(
        gssp["props"]["base"], "bar.txt",
        "path.basename: {gssp_json}"
    );
    assert_eq!(
        gssp["props"]["baseExt"], "bar",
        "path.basename with ext: {gssp_json}"
    );
    assert_eq!(gssp["props"]["dir"], "/foo", "path.dirname: {gssp_json}");
    assert_eq!(gssp["props"]["ext"], ".txt", "path.extname: {gssp_json}");
    assert_eq!(
        gssp["props"]["normalized"], "b/c",
        "path.join normalize: {gssp_json}"
    );
}

#[tokio::test]
async fn test_integration_path_node_prefix() {
    // Verify `import path from 'node:path'` also resolves
    let (_tmp, config, scan) = setup_test_project(
        &[(
            "index.tsx",
            r#"
                import path from 'node:path';
                export default function Home(props) {
                    return <div>{props.joined}</div>;
                }
                export function getServerSideProps() {
                    return { props: { joined: path.join('x', 'y') }};
                }
                "#,
        )],
        None,
    );

    let (_result, pool) = build_and_load(&config, &scan).await;

    let gssp_json = pool
        .execute(|iso| iso.get_server_side_props("index", "{\"params\":{},\"query\":{}}"))
        .await
        .expect("pool execute")
        .expect("gssp");

    let gssp: serde_json::Value = serde_json::from_str(&gssp_json).unwrap();
    assert_eq!(
        gssp["props"]["joined"], "x/y",
        "node:path should work: {gssp_json}"
    );
}

#[tokio::test]
async fn test_integration_path_named_imports() {
    // Verify named imports work: import { join, resolve } from 'path'
    let (_tmp, config, scan) = setup_test_project(
        &[(
            "index.tsx",
            r#"
                import { join, basename, dirname, extname, isAbsolute } from 'path';
                export default function Home(props) {
                    return <div>{JSON.stringify(props)}</div>;
                }
                export function getServerSideProps() {
                    return { props: {
                        joined: join('a', 'b'),
                        base: basename('/x/y.js'),
                        dir: dirname('/x/y.js'),
                        ext: extname('file.tar.gz'),
                        abs: isAbsolute('/foo'),
                        rel: isAbsolute('foo'),
                    }};
                }
                "#,
        )],
        None,
    );

    let (_result, pool) = build_and_load(&config, &scan).await;

    let gssp_json = pool
        .execute(|iso| iso.get_server_side_props("index", "{\"params\":{},\"query\":{}}"))
        .await
        .expect("pool execute")
        .expect("gssp");

    let gssp: serde_json::Value = serde_json::from_str(&gssp_json).unwrap();
    assert_eq!(gssp["props"]["joined"], "a/b");
    assert_eq!(gssp["props"]["base"], "y.js");
    assert_eq!(gssp["props"]["dir"], "/x");
    assert_eq!(gssp["props"]["ext"], ".gz");
    assert_eq!(gssp["props"]["abs"], true);
    assert_eq!(gssp["props"]["rel"], false);
}

// -- buffer polyfill integration tests --

#[tokio::test]
async fn test_integration_buffer_polyfill_global() {
    // Test Buffer global: from/toString with utf8, base64, hex
    let (_tmp, config, scan) = setup_test_project(
        &[(
            "index.tsx",
            r#"
                export default function Home(props) {
                    return <div>{JSON.stringify(props)}</div>;
                }
                export function getServerSideProps() {
                    var buf = Buffer.from('hello world');
                    return { props: {
                        utf8: buf.toString('utf8'),
                        base64: buf.toString('base64'),
                        hex: buf.toString('hex'),
                        roundtrip: Buffer.from(buf.toString('base64'), 'base64').toString('utf8'),
                        isBuffer: Buffer.isBuffer(buf),
                        notBuffer: Buffer.isBuffer('nope'),
                        len: buf.length,
                    }};
                }
                "#,
        )],
        None,
    );

    let (_result, pool) = build_and_load(&config, &scan).await;

    let gssp_json = pool
        .execute(|iso| iso.get_server_side_props("index", "{\"params\":{},\"query\":{}}"))
        .await
        .expect("pool execute")
        .expect("gssp");

    let gssp: serde_json::Value = serde_json::from_str(&gssp_json).unwrap();
    assert_eq!(gssp["props"]["utf8"], "hello world", "utf8: {gssp_json}");
    assert_eq!(
        gssp["props"]["base64"], "aGVsbG8gd29ybGQ=",
        "base64: {gssp_json}"
    );
    assert_eq!(
        gssp["props"]["hex"], "68656c6c6f20776f726c64",
        "hex: {gssp_json}"
    );
    assert_eq!(
        gssp["props"]["roundtrip"], "hello world",
        "base64 roundtrip: {gssp_json}"
    );
    assert_eq!(gssp["props"]["isBuffer"], true, "isBuffer: {gssp_json}");
    assert_eq!(gssp["props"]["notBuffer"], false, "notBuffer: {gssp_json}");
    assert_eq!(gssp["props"]["len"], 11, "length: {gssp_json}");
}

#[tokio::test]
async fn test_integration_buffer_polyfill_import() {
    // Test importing Buffer from 'buffer' module
    let (_tmp, config, scan) = setup_test_project(
        &[(
            "index.tsx",
            r#"
                import { Buffer } from 'buffer';
                export default function Home(props) {
                    return <div>{props.result}</div>;
                }
                export function getServerSideProps() {
                    var buf = Buffer.from('SGVsbG8=', 'base64');
                    return { props: { result: buf.toString('utf8') }};
                }
                "#,
        )],
        None,
    );

    let (_result, pool) = build_and_load(&config, &scan).await;

    let gssp_json = pool
        .execute(|iso| iso.get_server_side_props("index", "{\"params\":{},\"query\":{}}"))
        .await
        .expect("pool execute")
        .expect("gssp");

    let gssp: serde_json::Value = serde_json::from_str(&gssp_json).unwrap();
    assert_eq!(
        gssp["props"]["result"], "Hello",
        "import from 'buffer' should work: {gssp_json}"
    );
}

#[tokio::test]
async fn test_integration_buffer_alloc_concat() {
    // Test Buffer.alloc, Buffer.concat, and integer read/write methods
    let (_tmp, config, scan) = setup_test_project(
        &[(
            "index.tsx",
            r#"
                export default function Home(props) {
                    return <div>{JSON.stringify(props)}</div>;
                }
                export function getServerSideProps() {
                    var a = Buffer.from('foo');
                    var b = Buffer.from('bar');
                    var c = Buffer.concat([a, b]);

                    var alloc = Buffer.alloc(4);
                    alloc.writeUInt32BE(0xDEADBEEF, 0);
                    var readBack = alloc.readUInt32BE(0);

                    return { props: {
                        concat: c.toString('utf8'),
                        allocLen: alloc.length,
                        readBack: readBack,
                        byteLen: Buffer.byteLength('hello', 'utf8'),
                    }};
                }
                "#,
        )],
        None,
    );

    let (_result, pool) = build_and_load(&config, &scan).await;

    let gssp_json = pool
        .execute(|iso| iso.get_server_side_props("index", "{\"params\":{},\"query\":{}}"))
        .await
        .expect("pool execute")
        .expect("gssp");

    let gssp: serde_json::Value = serde_json::from_str(&gssp_json).unwrap();
    assert_eq!(gssp["props"]["concat"], "foobar", "concat: {gssp_json}");
    assert_eq!(gssp["props"]["allocLen"], 4, "alloc length: {gssp_json}");
    assert_eq!(
        gssp["props"]["readBack"], 0xDEADBEEFu64,
        "readBack: {gssp_json}"
    );
    assert_eq!(gssp["props"]["byteLen"], 5, "byteLength: {gssp_json}");
}
