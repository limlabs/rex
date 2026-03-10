#![allow(clippy::unwrap_used)]

mod common;

use common::{build_and_load, setup_test_project};

#[tokio::test]
async fn test_integration_url_module_parse() {
    let (_tmp, config, scan) = setup_test_project(
        &[(
            "index.tsx",
            r#"
                import url from 'url';
                export default function Home(props) {
                    return <div>{JSON.stringify(props)}</div>;
                }
                export function getServerSideProps() {
                    var parsed = url.parse('https://example.com:8080/path?key=value#hash');
                    return { props: {
                        protocol: parsed.protocol,
                        hostname: parsed.hostname,
                        port: parsed.port,
                        pathname: parsed.pathname,
                        hash: parsed.hash,
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
    assert_eq!(gssp["props"]["protocol"], "https:", "protocol: {gssp_json}");
    assert_eq!(
        gssp["props"]["hostname"], "example.com",
        "hostname: {gssp_json}"
    );
    assert_eq!(gssp["props"]["port"], "8080", "port: {gssp_json}");
    assert_eq!(gssp["props"]["pathname"], "/path", "pathname: {gssp_json}");
    assert_eq!(gssp["props"]["hash"], "#hash", "hash: {gssp_json}");
}

#[tokio::test]
async fn test_integration_url_module_format() {
    let (_tmp, config, scan) = setup_test_project(
        &[(
            "index.tsx",
            r#"
                import { parse, format } from 'url';
                export default function Home(props) {
                    return <div>{props.result}</div>;
                }
                export function getServerSideProps() {
                    var parsed = parse('https://example.com/hello?q=1');
                    var formatted = format(parsed);
                    return { props: { result: formatted }};
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
        gssp["props"]["result"], "https://example.com/hello?q=1",
        "url.format roundtrip: {gssp_json}"
    );
}

#[tokio::test]
async fn test_integration_querystring_module() {
    let (_tmp, config, scan) = setup_test_project(
        &[(
            "index.tsx",
            r#"
                import qs from 'querystring';
                export default function Home(props) {
                    return <div>{JSON.stringify(props)}</div>;
                }
                export function getServerSideProps() {
                    var parsed = qs.parse('foo=bar&baz=qux&foo=second');
                    var stringified = qs.stringify({ a: '1', b: '2' });
                    return { props: {
                        parsedBaz: parsed.baz,
                        parsedFooArray: Array.isArray(parsed.foo),
                        stringified: stringified,
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
    assert_eq!(gssp["props"]["parsedBaz"], "qux", "qs.parse: {gssp_json}");
    assert!(
        gssp["props"]["parsedFooArray"].as_bool().unwrap(),
        "duplicate keys become array: {gssp_json}"
    );
    assert_eq!(
        gssp["props"]["stringified"], "a=1&b=2",
        "qs.stringify: {gssp_json}"
    );
}

#[tokio::test]
async fn test_integration_events_module() {
    let (_tmp, config, scan) = setup_test_project(
        &[(
            "index.tsx",
            r#"
                import { EventEmitter } from 'events';
                export default function Home(props) {
                    return <div>{props.result}</div>;
                }
                export function getServerSideProps() {
                    var emitter = new EventEmitter();
                    var received = [];
                    emitter.on('data', function(v) { received.push(v); });
                    emitter.emit('data', 'hello');
                    emitter.emit('data', 'world');
                    return { props: { result: received.join(' ') }};
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
        gssp["props"]["result"], "hello world",
        "EventEmitter: {gssp_json}"
    );
}

#[tokio::test]
async fn test_integration_http_module_resolves() {
    // Verify that importing http doesn't crash the bundle at eval time.
    // We can't actually call http.request() in tests (no network),
    // but we verify the module loads and exports are accessible.
    let (_tmp, config, scan) = setup_test_project(
        &[(
            "index.tsx",
            r#"
                import http from 'http';
                export default function Home(props) {
                    return <div>{JSON.stringify(props)}</div>;
                }
                export function getServerSideProps() {
                    return { props: {
                        hasRequest: typeof http.request === 'function',
                        hasGet: typeof http.get === 'function',
                        hasCreateServer: typeof http.createServer === 'function',
                        statusOk: http.STATUS_CODES[200],
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
    assert!(
        gssp["props"]["hasRequest"].as_bool().unwrap(),
        "http.request should be a function: {gssp_json}"
    );
    assert!(
        gssp["props"]["hasGet"].as_bool().unwrap(),
        "http.get should be a function: {gssp_json}"
    );
    assert!(
        gssp["props"]["hasCreateServer"].as_bool().unwrap(),
        "http.createServer should be a function: {gssp_json}"
    );
    assert_eq!(
        gssp["props"]["statusOk"], "OK",
        "STATUS_CODES[200]: {gssp_json}"
    );
}

#[tokio::test]
async fn test_integration_https_module_resolves() {
    let (_tmp, config, scan) = setup_test_project(
        &[(
            "index.tsx",
            r#"
                import https from 'https';
                export default function Home(props) {
                    return <div>{props.result}</div>;
                }
                export function getServerSideProps() {
                    return { props: {
                        hasRequest: typeof https.request === 'function',
                        statusNotFound: https.STATUS_CODES[404],
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
    assert!(
        gssp["props"]["hasRequest"].as_bool().unwrap(),
        "https.request should be a function: {gssp_json}"
    );
    assert_eq!(
        gssp["props"]["statusNotFound"], "Not Found",
        "STATUS_CODES[404]: {gssp_json}"
    );
}

#[tokio::test]
async fn test_integration_node_prefix_imports() {
    // Verify node: prefix works for the new modules
    let (_tmp, config, scan) = setup_test_project(
        &[(
            "index.tsx",
            r#"
                import { parse } from 'node:url';
                import qs from 'node:querystring';
                import { EventEmitter } from 'node:events';
                export default function Home(props) {
                    return <div>{JSON.stringify(props)}</div>;
                }
                export function getServerSideProps() {
                    var parsed = parse('http://localhost/test');
                    var str = qs.stringify({ x: '1' });
                    var ee = new EventEmitter();
                    return { props: {
                        pathname: parsed.pathname,
                        qs: str,
                        isEmitter: typeof ee.on === 'function',
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
    assert_eq!(
        gssp["props"]["pathname"], "/test",
        "node:url parse: {gssp_json}"
    );
    assert_eq!(gssp["props"]["qs"], "x=1", "node:querystring: {gssp_json}");
    assert!(
        gssp["props"]["isEmitter"].as_bool().unwrap(),
        "node:events: {gssp_json}"
    );
}
