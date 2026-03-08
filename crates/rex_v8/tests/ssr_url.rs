#![allow(clippy::unwrap_used)]

mod common;

use common::{make_server_bundle, MOCK_REACT_RUNTIME};
use rex_v8::SsrIsolate;

fn make_url_isolate(gssp_code: &str) -> SsrIsolate {
    rex_v8::init_v8();
    let bundle = format!(
        "{}\n{MOCK_REACT_RUNTIME}\n{}",
        rex_build::V8_POLYFILLS,
        make_server_bundle(&[(
            "page",
            "function Page(props) { return React.createElement('pre', null, JSON.stringify(props)); }",
            Some(&format!("function(ctx) {{ {gssp_code} }}"))
        )])
    );
    SsrIsolate::new(&bundle, None).expect("failed to create url isolate")
}

// ---------- URL ----------

#[test]
fn test_url_parse_full() {
    let mut iso = make_url_isolate(
        r#"var u = new URL('https://user:pass@example.com:8080/path?q=1#frag');
        return { props: {
            protocol: u.protocol,
            username: u.username,
            password: u.password,
            hostname: u.hostname,
            port: u.port,
            pathname: u.pathname,
            search: u.search,
            hash: u.hash,
            host: u.host,
            origin: u.origin,
            href: u.href
        }};"#,
    );
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    let p = &val["props"];
    assert_eq!(p["protocol"], "https:");
    assert_eq!(p["username"], "user");
    assert_eq!(p["password"], "pass");
    assert_eq!(p["hostname"], "example.com");
    assert_eq!(p["port"], "8080");
    assert_eq!(p["pathname"], "/path");
    assert_eq!(p["search"], "?q=1");
    assert_eq!(p["hash"], "#frag");
    assert_eq!(p["host"], "example.com:8080");
    assert_eq!(p["origin"], "https://example.com:8080");
    assert_eq!(
        p["href"],
        "https://user:pass@example.com:8080/path?q=1#frag"
    );
}

#[test]
fn test_url_simple() {
    let mut iso = make_url_isolate(
        r#"var u = new URL('https://example.com/foo');
        return { props: {
            hostname: u.hostname,
            pathname: u.pathname,
            port: u.port,
            search: u.search,
            hash: u.hash
        }};"#,
    );
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    let p = &val["props"];
    assert_eq!(p["hostname"], "example.com");
    assert_eq!(p["pathname"], "/foo");
    assert_eq!(p["port"], "");
    assert_eq!(p["search"], "");
    assert_eq!(p["hash"], "");
}

#[test]
fn test_url_with_base() {
    let mut iso = make_url_isolate(
        r#"var u = new URL('/api/data', 'https://example.com:3000');
        return { props: { href: u.href, pathname: u.pathname, origin: u.origin }};"#,
    );
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    let p = &val["props"];
    assert_eq!(p["href"], "https://example.com:3000/api/data");
    assert_eq!(p["pathname"], "/api/data");
    assert_eq!(p["origin"], "https://example.com:3000");
}

#[test]
fn test_url_relative_with_base() {
    let mut iso = make_url_isolate(
        r#"var u = new URL('bar', 'https://example.com/foo/');
        return { props: { href: u.href, pathname: u.pathname }};"#,
    );
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    let p = &val["props"];
    assert_eq!(p["href"], "https://example.com/foo/bar");
    assert_eq!(p["pathname"], "/foo/bar");
}

#[test]
fn test_url_absolute_ignores_base() {
    let mut iso = make_url_isolate(
        r#"var u = new URL('https://other.com/x', 'https://example.com');
        return { props: { hostname: u.hostname, pathname: u.pathname }};"#,
    );
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    let p = &val["props"];
    assert_eq!(p["hostname"], "other.com");
    assert_eq!(p["pathname"], "/x");
}

#[test]
fn test_url_to_string() {
    let mut iso = make_url_isolate(
        r#"var u = new URL('https://example.com/path?q=1');
        return { props: { str: u.toString(), json: u.toJSON() }};"#,
    );
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    let p = &val["props"];
    assert_eq!(p["str"], "https://example.com/path?q=1");
    assert_eq!(p["json"], "https://example.com/path?q=1");
}

#[test]
fn test_url_search_params_linked() {
    let mut iso = make_url_isolate(
        r#"var u = new URL('https://example.com/?a=1&b=2');
        return { props: {
            a: u.searchParams.get('a'),
            b: u.searchParams.get('b'),
            c: u.searchParams.get('c')
        }};"#,
    );
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    let p = &val["props"];
    assert_eq!(p["a"], "1");
    assert_eq!(p["b"], "2");
    assert!(p["c"].is_null());
}

// ---------- URLSearchParams ----------

#[test]
fn test_search_params_from_string() {
    let mut iso = make_url_isolate(
        r#"var sp = new URLSearchParams('foo=bar&baz=qux');
        return { props: { foo: sp.get('foo'), baz: sp.get('baz'), str: sp.toString() }};"#,
    );
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    let p = &val["props"];
    assert_eq!(p["foo"], "bar");
    assert_eq!(p["baz"], "qux");
    assert_eq!(p["str"], "foo=bar&baz=qux");
}

#[test]
fn test_search_params_from_string_with_question_mark() {
    let mut iso = make_url_isolate(
        r#"var sp = new URLSearchParams('?key=val');
        return { props: { key: sp.get('key') }};"#,
    );
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(val["props"]["key"], "val");
}

#[test]
fn test_search_params_append_and_get_all() {
    let mut iso = make_url_isolate(
        r#"var sp = new URLSearchParams();
        sp.append('a', '1');
        sp.append('a', '2');
        sp.append('b', '3');
        return { props: { all_a: sp.getAll('a'), b: sp.get('b'), str: sp.toString() }};"#,
    );
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    let p = &val["props"];
    assert_eq!(p["all_a"], serde_json::json!(["1", "2"]));
    assert_eq!(p["b"], "3");
    assert_eq!(p["str"], "a=1&a=2&b=3");
}

#[test]
fn test_search_params_set() {
    let mut iso = make_url_isolate(
        r#"var sp = new URLSearchParams('a=1&a=2&b=3');
        sp.set('a', 'x');
        return { props: { a: sp.get('a'), all_a: sp.getAll('a'), str: sp.toString() }};"#,
    );
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    let p = &val["props"];
    assert_eq!(p["a"], "x");
    assert_eq!(p["all_a"], serde_json::json!(["x"]));
    assert_eq!(p["str"], "a=x&b=3");
}

#[test]
fn test_search_params_delete() {
    let mut iso = make_url_isolate(
        r#"var sp = new URLSearchParams('a=1&b=2&a=3');
        sp.delete('a');
        return { props: { has_a: sp.has('a'), str: sp.toString() }};"#,
    );
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    let p = &val["props"];
    assert_eq!(p["has_a"], false);
    assert_eq!(p["str"], "b=2");
}

#[test]
fn test_search_params_has() {
    let mut iso = make_url_isolate(
        r#"var sp = new URLSearchParams('x=1');
        return { props: { has_x: sp.has('x'), has_y: sp.has('y') }};"#,
    );
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(val["props"]["has_x"], true);
    assert_eq!(val["props"]["has_y"], false);
}

#[test]
fn test_search_params_sort() {
    let mut iso = make_url_isolate(
        r#"var sp = new URLSearchParams('c=3&a=1&b=2');
        sp.sort();
        return { props: { str: sp.toString() }};"#,
    );
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    assert_eq!(val(&json, "str"), "a=1&b=2&c=3");
}

fn val(json: &str, key: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(json).unwrap();
    v["props"][key].as_str().unwrap().to_string()
}

#[test]
fn test_search_params_size() {
    let mut iso = make_url_isolate(
        r#"var sp = new URLSearchParams('a=1&b=2&a=3');
        return { props: { size: sp.size }};"#,
    );
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(val["props"]["size"], 3);
}

#[test]
fn test_search_params_encoding() {
    let mut iso = make_url_isolate(
        r#"var sp = new URLSearchParams();
        sp.set('key', 'hello world');
        sp.set('special', 'a&b=c');
        return { props: { str: sp.toString(), key: sp.get('key'), special: sp.get('special') }};"#,
    );
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let p = &v["props"];
    assert_eq!(p["key"], "hello world");
    assert_eq!(p["special"], "a&b=c");
    // Encoded form should use + for spaces and %26/%3D for & and =
    let s = p["str"].as_str().unwrap();
    assert!(s.contains("hello+world"), "spaces should be + encoded: {s}");
    assert!(
        !s.contains("a&b"),
        "& should be percent-encoded in values: {s}"
    );
}

#[test]
fn test_search_params_for_each() {
    let mut iso = make_url_isolate(
        r#"var sp = new URLSearchParams('x=1&y=2');
        var keys = [];
        var vals = [];
        sp.forEach(function(v, k) { keys.push(k); vals.push(v); });
        return { props: { keys: keys, vals: vals }};"#,
    );
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["props"]["keys"], serde_json::json!(["x", "y"]));
    assert_eq!(v["props"]["vals"], serde_json::json!(["1", "2"]));
}

#[test]
fn test_search_params_iterator() {
    let mut iso = make_url_isolate(
        r#"var sp = new URLSearchParams('a=1&b=2');
        var result = [];
        for (var pair of sp) { result.push(pair[0] + '=' + pair[1]); }
        return { props: { result: result }};"#,
    );
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["props"]["result"], serde_json::json!(["a=1", "b=2"]));
}

#[test]
fn test_search_params_from_object() {
    let mut iso = make_url_isolate(
        r#"var sp = new URLSearchParams({foo: 'bar', num: '42'});
        return { props: { foo: sp.get('foo'), num: sp.get('num'), str: sp.toString() }};"#,
    );
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let p = &v["props"];
    assert_eq!(p["foo"], "bar");
    assert_eq!(p["num"], "42");
}
