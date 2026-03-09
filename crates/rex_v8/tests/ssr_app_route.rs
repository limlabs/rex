#![allow(clippy::unwrap_used)]

mod common;

use common::{make_server_bundle, MOCK_REACT_RUNTIME};
use rex_v8::SsrIsolate;

const APP_ROUTE_HANDLER_RUNTIME: &str = r#"
    globalThis.__rex_app_route_handlers = {};
    globalThis.__rex_app_route_handlers['/api/hello'] = {
        GET: function(req) {
            return { statusCode: 200, headers: { 'content-type': 'application/json' }, body: JSON.stringify({ message: 'hello' }) };
        },
        POST: function(req) {
            return { statusCode: 201, headers: { 'content-type': 'application/json' }, body: JSON.stringify({ created: true }) };
        }
    };

    globalThis.__rex_call_app_route_handler = function(routePattern, reqJson) {
        var handlers = globalThis.__rex_app_route_handlers;
        if (!handlers) throw new Error('No app route handlers registered');
        var routeModule = handlers[routePattern];
        if (!routeModule) throw new Error('App route handler not found: ' + routePattern);
        var reqData = JSON.parse(reqJson);
        var method = (reqData.method || 'GET').toUpperCase();
        var handlerFn = routeModule[method];
        if (!handlerFn) {
            var allowed = ['GET','HEAD','POST','PUT','DELETE','PATCH','OPTIONS'].filter(function(m) { return typeof routeModule[m] === 'function'; });
            return JSON.stringify({ statusCode: 405, headers: { allow: allowed.join(', ') }, body: 'Method Not Allowed' });
        }
        var result = handlerFn(reqData);
        return JSON.stringify(result);
    };
"#;

const ASYNC_APP_ROUTE_HANDLER_RUNTIME: &str = r#"
    globalThis.__rex_app_route_handlers = {};
    globalThis.__rex_app_route_handlers['/api/async'] = {
        GET: function(req) {
            return new Promise(function(resolve) {
                resolve({ statusCode: 200, headers: {}, body: 'async result' });
            });
        }
    };

    globalThis.__rex_app_route_resolved = null;
    globalThis.__rex_app_route_rejected = null;

    globalThis.__rex_call_app_route_handler = function(routePattern, reqJson) {
        var handlers = globalThis.__rex_app_route_handlers;
        if (!handlers) throw new Error('No app route handlers registered');
        var routeModule = handlers[routePattern];
        if (!routeModule) throw new Error('App route handler not found: ' + routePattern);
        var reqData = JSON.parse(reqJson);
        var method = (reqData.method || 'GET').toUpperCase();
        var handlerFn = routeModule[method];
        if (!handlerFn) {
            return JSON.stringify({ statusCode: 405, headers: {}, body: 'Method Not Allowed' });
        }
        var result = handlerFn(reqData);
        if (result && typeof result.then === 'function') {
            globalThis.__rex_app_route_resolved = null;
            globalThis.__rex_app_route_rejected = null;
            result.then(
                function(v) { globalThis.__rex_app_route_resolved = v; },
                function(e) { globalThis.__rex_app_route_rejected = e; }
            );
            return '__REX_APP_ROUTE_ASYNC__';
        }
        return JSON.stringify(result);
    };

    globalThis.__rex_resolve_app_route = function() {
        if (globalThis.__rex_app_route_rejected) throw globalThis.__rex_app_route_rejected;
        if (globalThis.__rex_app_route_resolved !== null) return JSON.stringify(globalThis.__rex_app_route_resolved);
        throw new Error('App route promise did not resolve');
    };
"#;

fn make_app_route_isolate(extra_js: &str) -> SsrIsolate {
    rex_v8::init_v8();
    let bundle = format!(
        "{}\n{}\n{}",
        MOCK_REACT_RUNTIME,
        make_server_bundle(&[(
            "index",
            "function I() { return React.createElement('div'); }",
            None,
        )]),
        extra_js
    );
    SsrIsolate::new(&bundle, None).expect("failed to create isolate")
}

#[test]
fn test_call_app_route_handler_sync_get() {
    let mut iso = make_app_route_isolate(APP_ROUTE_HANDLER_RUNTIME);
    let req =
        r#"{"method":"GET","url":"/api/hello","headers":{},"query":{},"body":null,"params":{}}"#;
    let result = iso.call_app_route_handler("/api/hello", req).unwrap();
    let json: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(json["statusCode"], 200);
    let body: serde_json::Value = serde_json::from_str(json["body"].as_str().unwrap()).unwrap();
    assert_eq!(body["message"], "hello");
}

#[test]
fn test_call_app_route_handler_sync_post() {
    let mut iso = make_app_route_isolate(APP_ROUTE_HANDLER_RUNTIME);
    let req =
        r#"{"method":"POST","url":"/api/hello","headers":{},"query":{},"body":null,"params":{}}"#;
    let result = iso.call_app_route_handler("/api/hello", req).unwrap();
    let json: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(json["statusCode"], 201);
}

#[test]
fn test_call_app_route_handler_method_not_allowed() {
    let mut iso = make_app_route_isolate(APP_ROUTE_HANDLER_RUNTIME);
    let req =
        r#"{"method":"DELETE","url":"/api/hello","headers":{},"query":{},"body":null,"params":{}}"#;
    let result = iso.call_app_route_handler("/api/hello", req).unwrap();
    let json: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(json["statusCode"], 405);
    assert!(json["headers"]["allow"].as_str().unwrap().contains("GET"));
}

#[test]
fn test_call_app_route_handler_not_loaded() {
    let mut iso = common::make_isolate(&[(
        "index",
        "function I() { return React.createElement('div'); }",
        None,
    )]);
    let result = iso.call_app_route_handler("/api/hello", "{}");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not loaded"));
}

#[test]
fn test_call_app_route_handler_async() {
    let mut iso = make_app_route_isolate(ASYNC_APP_ROUTE_HANDLER_RUNTIME);
    let req =
        r#"{"method":"GET","url":"/api/async","headers":{},"query":{},"body":null,"params":{}}"#;
    let result = iso.call_app_route_handler("/api/async", req).unwrap();
    let json: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(json["statusCode"], 200);
    assert_eq!(json["body"], "async result");
}
