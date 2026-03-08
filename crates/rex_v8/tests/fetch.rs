#![allow(clippy::unwrap_used)]

use rex_v8::fetch::{
    drain_fetch_queue, fetch_callback, is_private_ip, run_fetch_loop, validate_url_not_private,
};

#[test]
fn fetch_queue_starts_empty() {
    let pending = drain_fetch_queue();
    assert!(pending.is_empty());
}

#[test]
fn install_fetch_on_global() {
    rex_v8::init_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    v8::scope!(scope, &mut isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let global = context.global(scope);

    let t = v8::FunctionTemplate::new(scope, fetch_callback);
    let f = t.get_function(scope).expect("fetch function template");
    let k = v8::String::new(scope, "fetch").expect("fetch string");
    global.set(scope, k.into(), f.into());

    let k = v8::String::new(scope, "fetch").unwrap();
    let v = global.get(scope, k.into()).unwrap();
    assert!(v.is_function());
}

#[test]
fn fetch_returns_promise() {
    rex_v8::init_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());

    {
        v8::scope!(scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let global = context.global(scope);

        let t = v8::FunctionTemplate::new(scope, fetch_callback);
        let f = t.get_function(scope).expect("fetch fn");
        let k = v8::String::new(scope, "fetch").expect("fetch key");
        global.set(scope, k.into(), f.into());

        let code = "typeof fetch('http://example.com')";
        let source = v8::String::new(scope, code).unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        let result = script.run(scope).unwrap();
        assert_eq!(result.to_rust_string_lossy(scope), "object");
    }

    let pending = drain_fetch_queue();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].url, "http://example.com");
    assert_eq!(pending[0].method, "GET");
}

/// Integration test: verifies that `run_fetch_loop` can resolve a fetch promise
/// end-to-end. Requires a real HTTP server, so marked `#[ignore]` for CI.
#[test]
#[ignore]
fn test_run_fetch_loop_resolves_promise() {
    rex_v8::init_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());

    let context = {
        v8::scope!(scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let global = context.global(scope);

        let t = v8::FunctionTemplate::new(scope, fetch_callback);
        let f = t.get_function(scope).expect("fetch fn");
        let k = v8::String::new(scope, "fetch").expect("fetch key");
        global.set(scope, k.into(), f.into());

        let code = r#"
            var _result = null;
            fetch('https://httpbin.org/get')
                .then(function(r) { return r.json(); })
                .then(function(data) { _result = data.url; });
        "#;
        let source = v8::String::new(scope, code).unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        script.run(scope);

        v8::Global::new(scope, context)
    };

    run_fetch_loop(&mut isolate, &context);

    {
        v8::scope_with_context!(scope, &mut isolate, &context);
        let global = context.open(scope).global(scope);
        let key = v8::String::new(scope, "_result").unwrap();
        let val = global.get(scope, key.into()).unwrap();
        let result_str = val.to_rust_string_lossy(scope);
        assert!(
            result_str.contains("httpbin.org"),
            "Expected resolved URL to contain httpbin.org, got: {result_str}"
        );
    }
}

#[test]
fn fetch_parses_init_options() {
    rex_v8::init_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());

    {
        v8::scope!(scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let global = context.global(scope);

        let t = v8::FunctionTemplate::new(scope, fetch_callback);
        let f = t.get_function(scope).expect("fetch fn");
        let k = v8::String::new(scope, "fetch").expect("fetch key");
        global.set(scope, k.into(), f.into());

        let code = r#"fetch('http://example.com/api', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: '{"key": "value"}'
        })"#;
        let source = v8::String::new(scope, code).unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        script.run(scope);
    }

    let pending = drain_fetch_queue();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].method, "POST");
    assert_eq!(
        pending[0].headers.get("Content-Type").unwrap(),
        "application/json"
    );
    assert_eq!(pending[0].body.as_deref(), Some("{\"key\": \"value\"}"));
}

#[test]
fn ssrf_blocks_loopback_ipv4() {
    assert!(is_private_ip(&"127.0.0.1".parse().unwrap()));
    assert!(is_private_ip(&"127.0.0.2".parse().unwrap()));
}

#[test]
fn ssrf_blocks_private_ranges() {
    assert!(is_private_ip(&"10.0.0.1".parse().unwrap()));
    assert!(is_private_ip(&"172.16.0.1".parse().unwrap()));
    assert!(is_private_ip(&"192.168.1.1".parse().unwrap()));
}

#[test]
fn ssrf_blocks_link_local() {
    assert!(is_private_ip(&"169.254.169.254".parse().unwrap()));
}

#[test]
fn ssrf_blocks_ipv6_loopback() {
    assert!(is_private_ip(&"::1".parse().unwrap()));
}

#[test]
fn ssrf_allows_public_ip() {
    assert!(!is_private_ip(&"8.8.8.8".parse().unwrap()));
    assert!(!is_private_ip(&"93.184.216.34".parse().unwrap()));
}

#[tokio::test]
async fn ssrf_blocks_localhost_url() {
    let result = validate_url_not_private("http://127.0.0.1/secret").await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("private address"));
}

#[tokio::test]
async fn ssrf_blocks_metadata_url() {
    let result = validate_url_not_private("http://169.254.169.254/latest/meta-data/").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn ssrf_allowlist_permits_internal_origin() {
    std::env::set_var("REX_INTERNAL_ORIGIN", "http://127.0.0.1:8000");
    let result = validate_url_not_private("http://127.0.0.1:8000/api/data").await;
    assert!(result.is_ok());
    let result2 = validate_url_not_private("http://127.0.0.1:9999/secret").await;
    assert!(result2.is_err());
    std::env::remove_var("REX_INTERNAL_ORIGIN");
}
