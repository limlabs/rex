#![allow(clippy::unwrap_used)]

mod common;

use common::{make_server_bundle, MOCK_REACT_RUNTIME};
use rex_v8::SsrIsolate;
use std::io::{Read, Write};
use std::net::TcpListener;

/// Create an isolate with V8 polyfills (needed for async/IO loop support).
fn make_tcp_isolate(gssp_code: &str) -> SsrIsolate {
    rex_v8::init_v8();
    let bundle = format!(
        "{}\n{MOCK_REACT_RUNTIME}\n{}",
        rex_build::V8_POLYFILLS,
        make_server_bundle(&[(
            "page",
            "function Page(props) { return React.createElement('pre', null, JSON.stringify(props)); }",
            Some(gssp_code),
        )])
    );
    SsrIsolate::new(&bundle, None).expect("failed to create tcp isolate")
}

/// Start a TCP echo server on a random port. Returns the port and join handle.
fn start_echo_server() -> (u16, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 1024];
            if let Ok(n) = stream.read(&mut buf) {
                let _ = stream.write_all(&buf[..n]);
                let _ = stream.flush();
            }
            // Small delay so the client can read before we drop the stream
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    });
    (port, handle)
}

#[test]
fn test_tcp_connect_write_push_read_close() {
    let (port, server) = start_echo_server();

    // Uses push-based IO: poll_tcp_sockets calls __rex_tcp_push_data
    let gssp_code = format!(
        r#"async function(ctx) {{
            var resolve;
            var p = new Promise(function(r) {{ resolve = r; }});

            globalThis.__rex_tcp_push_data = function(connId, data) {{
                var text = '';
                for (var i = 0; i < data.length; i++) {{
                    text += String.fromCharCode(data[i]);
                }}
                resolve(text);
            }};
            globalThis.__rex_tcp_push_eof = function() {{}};
            globalThis.__rex_tcp_push_error = function() {{}};

            var connId = globalThis.__rex_tcp_connect('127.0.0.1', {port});
            var arr = new Uint8Array([104, 101, 108, 108, 111]);
            globalThis.__rex_tcp_write(connId, arr);
            globalThis.__rex_tcp_enable_polling(connId);

            var text = await p;
            globalThis.__rex_tcp_close(connId);
            return {{ props: {{ data: text }} }};
        }}"#
    );

    let mut iso = make_tcp_isolate(&gssp_code);
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(val["props"]["data"].as_str().unwrap(), "hello");

    server.join().unwrap();
}

#[test]
fn test_tcp_write_string_data() {
    let (port, server) = start_echo_server();

    let gssp_code = format!(
        r#"async function(ctx) {{
            var resolve;
            var p = new Promise(function(r) {{ resolve = r; }});

            globalThis.__rex_tcp_push_data = function(connId, data) {{
                var text = '';
                for (var i = 0; i < data.length; i++) {{
                    text += String.fromCharCode(data[i]);
                }}
                resolve(text);
            }};
            globalThis.__rex_tcp_push_eof = function() {{}};
            globalThis.__rex_tcp_push_error = function() {{}};

            var connId = globalThis.__rex_tcp_connect('127.0.0.1', {port});
            globalThis.__rex_tcp_write(connId, 'world');
            globalThis.__rex_tcp_enable_polling(connId);

            var text = await p;
            globalThis.__rex_tcp_close(connId);
            return {{ props: {{ data: text }} }};
        }}"#
    );

    let mut iso = make_tcp_isolate(&gssp_code);
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(val["props"]["data"].as_str().unwrap(), "world");

    server.join().unwrap();
}

#[test]
fn test_tcp_connect_invalid_address() {
    // Connect to a port that's (almost certainly) not listening
    let gssp_code = r#"function(ctx) {
        try {
            globalThis.__rex_tcp_connect('127.0.0.1', 1);
            return { props: { error: 'none' } };
        } catch(e) {
            return { props: { error: e.message } };
        }
    }"#;

    let mut iso = make_tcp_isolate(gssp_code);
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    let error = val["props"]["error"].as_str().unwrap();
    assert!(
        error.contains("TCP connect failed"),
        "Expected connection error, got: {error}"
    );
}

#[test]
fn test_tcp_write_invalid_conn_id() {
    let gssp_code = r#"function(ctx) {
        try {
            globalThis.__rex_tcp_write(99999, 'data');
            return { props: { error: 'none' } };
        } catch(e) {
            return { props: { error: e.message } };
        }
    }"#;

    let mut iso = make_tcp_isolate(gssp_code);
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    let error = val["props"]["error"].as_str().unwrap();
    assert!(
        error.contains("not found"),
        "Expected not found error, got: {error}"
    );
}

#[test]
fn test_tcp_close_invalid_conn_id() {
    // Closing a non-existent connection should be a no-op (no error)
    let gssp_code = r#"function(ctx) {
        globalThis.__rex_tcp_close(99999);
        return { props: { ok: true } };
    }"#;

    let mut iso = make_tcp_isolate(gssp_code);
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(val["props"]["ok"].as_bool().unwrap());
}

#[test]
fn test_tcp_start_tls_invalid_conn_id() {
    let gssp_code = r#"function(ctx) {
        try {
            globalThis.__rex_tcp_start_tls(99999, 'example.com');
            return { props: { error: 'none' } };
        } catch(e) {
            return { props: { error: e.message } };
        }
    }"#;

    let mut iso = make_tcp_isolate(gssp_code);
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    let error = val["props"]["error"].as_str().unwrap();
    assert!(
        error.contains("not found"),
        "Expected not found error, got: {error}"
    );
}

#[test]
fn test_tcp_connect_missing_args() {
    let gssp_code = r#"function(ctx) {
        try {
            globalThis.__rex_tcp_connect();
            return { props: { error: 'none' } };
        } catch(e) {
            return { props: { error: e.message } };
        }
    }"#;

    let mut iso = make_tcp_isolate(gssp_code);
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    let error = val["props"]["error"].as_str().unwrap();
    assert!(
        error.contains("requires host and port"),
        "Expected missing args error, got: {error}"
    );
}

#[test]
fn test_tcp_write_missing_args() {
    let gssp_code = r#"function(ctx) {
        try {
            globalThis.__rex_tcp_write();
            return { props: { error: 'none' } };
        } catch(e) {
            return { props: { error: e.message } };
        }
    }"#;

    let mut iso = make_tcp_isolate(gssp_code);
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    let error = val["props"]["error"].as_str().unwrap();
    assert!(
        error.contains("requires connId and data"),
        "Expected missing args error, got: {error}"
    );
}

#[test]
fn test_tcp_start_tls_missing_args() {
    let gssp_code = r#"function(ctx) {
        try {
            globalThis.__rex_tcp_start_tls();
            return { props: { error: 'none' } };
        } catch(e) {
            return { props: { error: e.message } };
        }
    }"#;

    let mut iso = make_tcp_isolate(gssp_code);
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    let error = val["props"]["error"].as_str().unwrap();
    assert!(
        error.contains("requires connId and hostname"),
        "Expected missing args error, got: {error}"
    );
}
