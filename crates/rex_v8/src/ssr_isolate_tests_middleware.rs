use super::*;

#[test]
fn test_run_middleware_no_middleware() {
    let mut iso = make_isolate(&[(
        "index",
        "function Index() { return React.createElement('h1', null, 'Hello'); }",
        None,
    )]);
    let result = iso.run_middleware(r#"{"method":"GET","url":"/"}"#).unwrap();
    assert!(result.is_none(), "should return None when no middleware");
}

#[test]
fn test_run_middleware_next() {
    crate::init_v8();
    let mut bundle = format!(
        "{}\n{}",
        MOCK_REACT_RUNTIME,
        make_server_bundle(&[(
            "index",
            "function Index() { return React.createElement('h1', null, 'Hello'); }",
            None,
        )])
    );
    bundle.push_str(r#"
        globalThis.__rex_middleware = {
            middleware: function(req) {
                return { _action: 'next', _url: null, _status: 307, _requestHeaders: {}, _responseHeaders: {} };
            }
        };
        globalThis.__rex_run_middleware = function(reqJson) {
            var mw = globalThis.__rex_middleware;
            var result = mw.middleware(JSON.parse(reqJson));
            return JSON.stringify({
                action: result._action,
                url: result._url || null,
                status: result._status || 307,
                request_headers: result._requestHeaders || {},
                response_headers: result._responseHeaders || {}
            });
        };
    "#);
    let mut iso = SsrIsolate::new(&bundle, None).unwrap();
    let result = iso.run_middleware(r#"{"method":"GET","url":"/"}"#).unwrap();
    assert!(result.is_some());
    let json: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
    assert_eq!(json["action"], "next");
}

#[test]
fn test_run_middleware_redirect() {
    crate::init_v8();
    let mut bundle = format!(
        "{}\n{}",
        MOCK_REACT_RUNTIME,
        make_server_bundle(&[(
            "index",
            "function Index() { return React.createElement('h1', null, 'Hello'); }",
            None,
        )])
    );
    bundle.push_str(r#"
        globalThis.__rex_middleware = {
            middleware: function(req) {
                return { _action: 'redirect', _url: '/login', _status: 302, _requestHeaders: {}, _responseHeaders: {} };
            }
        };
        globalThis.__rex_run_middleware = function(reqJson) {
            var mw = globalThis.__rex_middleware;
            var result = mw.middleware(JSON.parse(reqJson));
            return JSON.stringify({
                action: result._action,
                url: result._url || null,
                status: result._status || 307,
                request_headers: result._requestHeaders || {},
                response_headers: result._responseHeaders || {}
            });
        };
    "#);
    let mut iso = SsrIsolate::new(&bundle, None).unwrap();
    let result = iso
        .run_middleware(r#"{"method":"GET","url":"/dashboard"}"#)
        .unwrap();
    assert!(result.is_some());
    let json: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
    assert_eq!(json["action"], "redirect");
    assert_eq!(json["url"], "/login");
    assert_eq!(json["status"], 302);
}

#[test]
fn test_list_mcp_tools_none() {
    let mut iso = make_isolate(&[("index", "function() { return 'hi'; }", None)]);
    // No MCP tools registered, should return None
    let result = iso.list_mcp_tools().unwrap();
    assert!(result.is_none());
}

#[test]
fn test_list_mcp_tools() {
    crate::init_v8();
    let mut bundle = format!(
        "{}\n{}",
        MOCK_REACT_RUNTIME,
        make_server_bundle(&[("index", "function() { return 'hi'; }", None)])
    );
    bundle.push_str(
        r#"
        globalThis.__rex_mcp_tools = {
            'search': {
                description: 'Search items',
                parameters: { type: 'object', properties: { query: { type: 'string' } } },
                default: function(params) { return { results: [] }; }
            }
        };
        globalThis.__rex_list_mcp_tools = function() {
            var tools = globalThis.__rex_mcp_tools;
            var result = [];
            var names = Object.keys(tools);
            for (var i = 0; i < names.length; i++) {
                var name = names[i];
                var mod = tools[name];
                result.push({ name: name, description: mod.description || '', parameters: mod.parameters || {} });
            }
            return JSON.stringify(result);
        };
    "#,
    );
    let mut iso = SsrIsolate::new(&bundle, None).unwrap();
    let result = iso.list_mcp_tools().unwrap();
    assert!(result.is_some());
    let tools: Vec<serde_json::Value> = serde_json::from_str(&result.unwrap()).unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "search");
    assert_eq!(tools[0]["description"], "Search items");
}

#[test]
fn test_call_mcp_tool_sync() {
    crate::init_v8();
    let mut bundle = format!(
        "{}\n{}",
        MOCK_REACT_RUNTIME,
        make_server_bundle(&[("index", "function() { return 'hi'; }", None)])
    );
    bundle.push_str(
        r#"
        globalThis.__rex_mcp_tools = {
            'echo': {
                description: 'Echo input',
                parameters: { type: 'object', properties: { msg: { type: 'string' } } },
                default: function(params) { return { echo: params.msg }; }
            }
        };
        globalThis.__rex_call_mcp_tool = function(name, paramsJson) {
            var tools = globalThis.__rex_mcp_tools;
            var mod = tools[name];
            if (!mod) throw new Error('MCP tool not found: ' + name);
            var params = JSON.parse(paramsJson);
            var result = mod.default(params);
            return JSON.stringify(result);
        };
    "#,
    );
    let mut iso = SsrIsolate::new(&bundle, None).unwrap();
    let result = iso.call_mcp_tool("echo", r#"{"msg":"hello"}"#).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["echo"], "hello");
}

#[test]
fn test_call_mcp_tool_async() {
    crate::init_v8();
    let mut bundle = format!(
        "{}\n{}",
        MOCK_REACT_RUNTIME,
        make_server_bundle(&[("index", "function() { return 'hi'; }", None)])
    );
    bundle.push_str(
        r#"
        globalThis.__rex_mcp_tools = {
            'async_tool': {
                description: 'Async tool',
                parameters: {},
                default: function(params) { return Promise.resolve({ async: true }); }
            }
        };
        globalThis.__rex_mcp_resolved = null;
        globalThis.__rex_mcp_rejected = null;
        globalThis.__rex_call_mcp_tool = function(name, paramsJson) {
            var tools = globalThis.__rex_mcp_tools;
            var mod = tools[name];
            if (!mod) throw new Error('MCP tool not found: ' + name);
            var params = JSON.parse(paramsJson);
            var result = mod.default(params);
            if (result && typeof result.then === 'function') {
                globalThis.__rex_mcp_resolved = null;
                globalThis.__rex_mcp_rejected = null;
                result.then(
                    function(v) { globalThis.__rex_mcp_resolved = v; },
                    function(e) { globalThis.__rex_mcp_rejected = e; }
                );
                return '__REX_MCP_ASYNC__';
            }
            return JSON.stringify(result);
        };
        globalThis.__rex_resolve_mcp = function() {
            if (globalThis.__rex_mcp_rejected) throw globalThis.__rex_mcp_rejected;
            if (globalThis.__rex_mcp_resolved !== null) return JSON.stringify(globalThis.__rex_mcp_resolved);
            throw new Error('MCP tool promise did not resolve');
        };
    "#,
    );
    let mut iso = SsrIsolate::new(&bundle, None).unwrap();
    let result = iso.call_mcp_tool("async_tool", "{}").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["async"], true);
}
