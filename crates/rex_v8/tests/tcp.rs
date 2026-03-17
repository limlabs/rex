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

/// Simulates the postgres.js connection pattern: multiple round-trips with
/// async/await delays between them. The server responds with deliberate delays
/// (>1ms) to test that the fetch loop doesn't exit prematurely.
///
/// This is the pattern that broke with the old fetch loop which would exit after
/// a single 1ms retry when TCP sockets had no data.
#[test]
fn test_tcp_multi_round_trip_with_delayed_server() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    // Server: simulates a multi-step protocol with delays
    let server = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 1024];

            // Round 1: Read startup message, delay, send auth challenge
            if let Ok(n) = stream.read(&mut buf) {
                assert!(n > 0);
                std::thread::sleep(std::time::Duration::from_millis(5));
                let _ = stream.write_all(b"AUTH_CHALLENGE");
                let _ = stream.flush();
            }

            // Round 2: Read auth response, delay, send auth ok
            if let Ok(n) = stream.read(&mut buf) {
                assert!(n > 0);
                std::thread::sleep(std::time::Duration::from_millis(5));
                let _ = stream.write_all(b"AUTH_OK");
                let _ = stream.flush();
            }

            // Round 3: Read query, delay, send result
            if let Ok(n) = stream.read(&mut buf) {
                assert!(n > 0);
                std::thread::sleep(std::time::Duration::from_millis(5));
                let _ = stream.write_all(b"RESULT:42");
                let _ = stream.flush();
            }

            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    });

    // JS code that simulates postgres.js pattern:
    // - async/await between each round-trip
    // - Multiple send→wait→receive cycles
    let gssp_code = format!(
        r#"async function(ctx) {{
            var dataResolve;

            globalThis.__rex_tcp_push_data = function(connId, data) {{
                var text = '';
                for (var i = 0; i < data.length; i++) {{
                    text += String.fromCharCode(data[i]);
                }}
                if (dataResolve) dataResolve(text);
            }};
            globalThis.__rex_tcp_push_eof = function() {{}};
            globalThis.__rex_tcp_push_error = function(connId, msg) {{
                if (dataResolve) dataResolve('ERROR:' + msg);
            }};

            function waitForData() {{
                return new Promise(function(r) {{ dataResolve = r; }});
            }}

            // Connect
            var connId = globalThis.__rex_tcp_connect('127.0.0.1', {port});
            globalThis.__rex_tcp_enable_polling(connId);

            // Round 1: send startup, await challenge
            globalThis.__rex_tcp_write(connId, 'STARTUP');
            var challenge = await waitForData();

            // Simulate async crypto (like SCRAM-SHA-256)
            var authResp = await Promise.resolve('SCRAM_PROOF_' + challenge);

            // Round 2: send auth response, await ok
            globalThis.__rex_tcp_write(connId, authResp);
            var authOk = await waitForData();

            // Round 3: send query, await result
            globalThis.__rex_tcp_write(connId, 'SELECT 42');
            var result = await waitForData();

            globalThis.__rex_tcp_close(connId);
            return {{ props: {{ challenge: challenge, authOk: authOk, result: result }} }};
        }}"#
    );

    let mut iso = make_tcp_isolate(&gssp_code);
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(
        val["props"]["challenge"].as_str().unwrap(),
        "AUTH_CHALLENGE"
    );
    assert_eq!(val["props"]["authOk"].as_str().unwrap(), "AUTH_OK");
    assert_eq!(val["props"]["result"].as_str().unwrap(), "RESULT:42");

    server.join().unwrap();
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
