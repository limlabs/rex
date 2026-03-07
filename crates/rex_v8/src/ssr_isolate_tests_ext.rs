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

// --- fs polyfill integration tests ---

/// Create an isolate with fs callbacks enabled and a GSSP that exercises them.
fn make_fs_isolate(project_root: &std::path::Path, gssp_code: &str) -> SsrIsolate {
    crate::init_v8();
    let pages = &[(
        "index",
        "function Index(props) { return React.createElement('div', null, JSON.stringify(props)); }",
        Some(gssp_code),
    )];
    let bundle = format!("{}\n{}", MOCK_REACT_RUNTIME, make_server_bundle(pages));
    let root_str = project_root.to_string_lossy().to_string();
    SsrIsolate::new(&bundle, Some(&root_str)).expect("failed to create fs isolate")
}

#[test]
fn test_fs_read_file_sync_utf8() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    std::fs::write(root.join("data.txt"), "hello from file").unwrap();

    let gssp = r#"function gssp(ctx) {
        var content = globalThis.__rex_fs_read_file_sync(globalThis.__rex_project_root, 'data.txt', 'utf8');
        return { props: { content: content } };
    }"#;
    let mut iso = make_fs_isolate(&root, gssp);

    let result = iso.get_server_side_props("index", "{}").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(
        parsed["props"]["content"].as_str(),
        Some("hello from file"),
        "Should read file content: {result}"
    );
}

#[test]
fn test_fs_path_traversal_blocked() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();

    let gssp = r#"function gssp(ctx) {
        var result = globalThis.__rex_fs_read_file_sync(globalThis.__rex_project_root, '../../etc/passwd', 'utf8');
        if (typeof result === 'string' && result.indexOf('__REX_FS_ERR__') === 0) {
            var err = JSON.parse(result.slice(14));
            return { props: { error: err.code } };
        }
        return { props: { content: result } };
    }"#;
    let mut iso = make_fs_isolate(&root, gssp);

    let result = iso.get_server_side_props("index", "{}").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(
        parsed["props"]["error"].as_str(),
        Some("EACCES"),
        "Should block traversal: {result}"
    );
}

#[test]
fn test_fs_write_and_read_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();

    let gssp = r#"function gssp(ctx) {
        globalThis.__rex_fs_write_file_sync(globalThis.__rex_project_root, 'out.txt', 'round trip data');
        var content = globalThis.__rex_fs_read_file_sync(globalThis.__rex_project_root, 'out.txt', 'utf8');
        return { props: { content: content } };
    }"#;
    let mut iso = make_fs_isolate(&root, gssp);

    let result = iso.get_server_side_props("index", "{}").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(
        parsed["props"]["content"].as_str(),
        Some("round trip data"),
        "Should write and read back: {result}"
    );
}

#[test]
fn test_fs_exists_sync() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    std::fs::write(root.join("exists.txt"), "yes").unwrap();

    let gssp = r#"function gssp(ctx) {
        var yes = globalThis.__rex_fs_exists_sync(globalThis.__rex_project_root, 'exists.txt');
        var no = globalThis.__rex_fs_exists_sync(globalThis.__rex_project_root, 'nope.txt');
        return { props: { exists: yes, missing: no } };
    }"#;
    let mut iso = make_fs_isolate(&root, gssp);

    let result = iso.get_server_side_props("index", "{}").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["props"]["exists"], true);
    assert_eq!(parsed["props"]["missing"], false);
}

#[test]
fn test_fs_readdir_sync() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    std::fs::write(root.join("a.txt"), "").unwrap();
    std::fs::write(root.join("b.txt"), "").unwrap();

    let gssp = r#"function gssp(ctx) {
        var json = globalThis.__rex_fs_readdir_sync(globalThis.__rex_project_root, '.');
        var entries = JSON.parse(json);
        entries.sort();
        return { props: { entries: entries } };
    }"#;
    let mut iso = make_fs_isolate(&root, gssp);

    let result = iso.get_server_side_props("index", "{}").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let entries = parsed["props"]["entries"].as_array().unwrap();
    assert!(
        entries.iter().any(|e| e.as_str() == Some("a.txt")),
        "Should list a.txt: {result}"
    );
    assert!(
        entries.iter().any(|e| e.as_str() == Some("b.txt")),
        "Should list b.txt: {result}"
    );
}

#[test]
fn test_fs_stat_sync() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    std::fs::write(root.join("stat_test.txt"), "hello world").unwrap();
    std::fs::create_dir(root.join("subdir")).unwrap();

    let gssp = r#"function gssp(ctx) {
        var fileJson = globalThis.__rex_fs_stat_sync(globalThis.__rex_project_root, 'stat_test.txt');
        var fileStat = JSON.parse(fileJson);
        var dirJson = globalThis.__rex_fs_stat_sync(globalThis.__rex_project_root, 'subdir');
        var dirStat = JSON.parse(dirJson);
        return { props: { fileIsFile: fileStat.isFile, fileSize: fileStat.size, dirIsDir: dirStat.isDirectory } };
    }"#;
    let mut iso = make_fs_isolate(&root, gssp);

    let result = iso.get_server_side_props("index", "{}").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["props"]["fileIsFile"], true);
    assert_eq!(parsed["props"]["fileSize"], 11); // "hello world"
    assert_eq!(parsed["props"]["dirIsDir"], true);
}

#[test]
fn test_fs_mkdir_recursive() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();

    let gssp = r#"function gssp(ctx) {
        globalThis.__rex_fs_mkdir_sync(globalThis.__rex_project_root, 'a/b/c', { recursive: true });
        var exists = globalThis.__rex_fs_exists_sync(globalThis.__rex_project_root, 'a/b/c');
        return { props: { created: exists } };
    }"#;
    let mut iso = make_fs_isolate(&root, gssp);

    let result = iso.get_server_side_props("index", "{}").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["props"]["created"], true);
}

#[test]
fn test_fs_rm_sync() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    std::fs::write(root.join("to_delete.txt"), "bye").unwrap();
    std::fs::create_dir_all(root.join("rmdir/sub")).unwrap();
    std::fs::write(root.join("rmdir/sub/file.txt"), "nested").unwrap();

    let gssp = r#"function gssp(ctx) {
        globalThis.__rex_fs_unlink_sync(globalThis.__rex_project_root, 'to_delete.txt');
        var fileGone = !globalThis.__rex_fs_exists_sync(globalThis.__rex_project_root, 'to_delete.txt');
        globalThis.__rex_fs_rm_sync(globalThis.__rex_project_root, 'rmdir', { recursive: true });
        var dirGone = !globalThis.__rex_fs_exists_sync(globalThis.__rex_project_root, 'rmdir');
        return { props: { fileGone: fileGone, dirGone: dirGone } };
    }"#;
    let mut iso = make_fs_isolate(&root, gssp);

    let result = iso.get_server_side_props("index", "{}").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["props"]["fileGone"], true);
    assert_eq!(parsed["props"]["dirGone"], true);
}

#[test]
fn test_process_env_from_rust() {
    // Set a known env var so we can verify it appears in V8
    std::env::set_var("REX_TEST_POLYFILL", "hello_from_rust");

    let mut iso = make_isolate(&[(
        "envtest",
        "function EnvTest() { return React.createElement('p', null, process.env.REX_TEST_POLYFILL || 'MISSING'); }",
        Some("function(ctx) { return { props: { val: process.env.REX_TEST_POLYFILL } }; }"),
    )]);

    // Verify GSSP can read process.env
    let gssp_result = iso.get_server_side_props("envtest", "{}").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&gssp_result).unwrap();
    assert_eq!(parsed["props"]["val"], "hello_from_rust");

    // Verify SSR render can read process.env
    let render = iso.render_page("envtest", "{}").unwrap();
    assert!(
        render.body.contains("hello_from_rust"),
        "SSR body should contain env var value, got: {}",
        render.body
    );

    // Clean up
    std::env::remove_var("REX_TEST_POLYFILL");
}

#[test]
fn test_process_env_is_writable() {
    // Node.js allows assigning to process.env; verify we match that behavior
    let mut iso = make_isolate(&[(
        "writetest",
        "function WriteTest() { process.env.DYNAMIC = 'set_at_runtime'; return React.createElement('p', null, process.env.DYNAMIC); }",
        None,
    )]);
    let render = iso.render_page("writetest", "{}").unwrap();
    assert!(
        render.body.contains("set_at_runtime"),
        "process.env should be writable, got: {}",
        render.body
    );
}

#[test]
fn test_console_log_emits_tracing_event() {
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::layer::SubscriberExt;

    /// Minimal tracing layer that captures log messages.
    struct CaptureLayer {
        messages: Arc<Mutex<Vec<(tracing::Level, String)>>>,
    }

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CaptureLayer {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            struct Visitor(String);
            impl tracing::field::Visit for Visitor {
                fn record_debug(
                    &mut self,
                    field: &tracing::field::Field,
                    value: &dyn std::fmt::Debug,
                ) {
                    if field.name() == "message" {
                        self.0 = format!("{value:?}");
                    }
                }
            }
            let mut visitor = Visitor(String::new());
            event.record(&mut visitor);
            self.messages
                .lock()
                .unwrap()
                .push((*event.metadata().level(), visitor.0));
        }
    }

    let messages = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new("v8::console=info"))
        .with(CaptureLayer {
            messages: messages.clone(),
        });

    let _guard = tracing::subscriber::set_default(subscriber);

    let mut iso = make_isolate(&[(
        "logpage",
        r#"function LogPage() {
            console.log("hello from ssr");
            console.warn("warning from ssr");
            console.error("error from ssr");
            return React.createElement('p', null, 'logged');
        }"#,
        None,
    )]);
    let render = iso.render_page("logpage", "{}").unwrap();
    assert!(render.body.contains("logged"), "page should render");

    let captured = messages.lock().unwrap();
    assert!(
        captured
            .iter()
            .any(|(_, msg)| msg.contains("hello from ssr")),
        "console.log should emit tracing event, captured: {captured:?}"
    );
    assert!(
        captured
            .iter()
            .any(|(level, msg)| *level == tracing::Level::WARN && msg.contains("warning from ssr")),
        "console.warn should emit WARN-level event, captured: {captured:?}"
    );
    assert!(
        captured
            .iter()
            .any(|(level, msg)| *level == tracing::Level::ERROR && msg.contains("error from ssr")),
        "console.error should emit ERROR-level event, captured: {captured:?}"
    );
}

fn make_isolate_with_actions(actions_js: &str) -> SsrIsolate {
    crate::init_v8();
    let bundle = format!(
        "{}\n{}\n{}",
        MOCK_REACT_RUNTIME,
        make_server_bundle(&[]),
        actions_js
    );
    SsrIsolate::new(&bundle, None).expect("failed to create isolate")
}

#[test]
fn test_call_server_action_sync() {
    let mut iso = make_isolate_with_actions(
        r#"
        globalThis.__rex_server_actions = {
            "act123": function(x) { return x + 1; }
        };
        globalThis.__rex_call_server_action = function(actionId, argsJson) {
            var actions = globalThis.__rex_server_actions || {};
            var fn = actions[actionId];
            if (!fn) return JSON.stringify({ error: "not found: " + actionId });
            var args = JSON.parse(argsJson);
            try {
                var result = fn.apply(null, args);
                return JSON.stringify({ result: result });
            } catch (e) {
                return JSON.stringify({ error: String(e) });
            }
        };
        "#,
    );
    let result = iso.call_server_action("act123", "[42]").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["result"], 43);
}

#[test]
fn test_call_server_action_not_found() {
    let mut iso = make_isolate_with_actions(
        r#"
        globalThis.__rex_server_actions = {};
        globalThis.__rex_call_server_action = function(actionId, argsJson) {
            var actions = globalThis.__rex_server_actions || {};
            var fn = actions[actionId];
            if (!fn) return JSON.stringify({ error: "Server action not found: " + actionId });
            return JSON.stringify({ result: null });
        };
        "#,
    );
    let result = iso.call_server_action("nonexistent", "[]").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert!(parsed["error"].as_str().unwrap().contains("not found"));
}

#[test]
fn test_call_server_action_not_loaded() {
    let mut iso = make_isolate(&[]);
    let err = iso.call_server_action("act123", "[]").unwrap_err();
    assert!(
        err.to_string().contains("not loaded"),
        "Should error when server actions not loaded, got: {err}"
    );
}

#[test]
fn test_call_server_action_async() {
    let mut iso = make_isolate_with_actions(
        r#"
        var _actionResult = null;
        var _actionDone = false;
        globalThis.__rex_call_server_action = function(actionId, argsJson) {
            _actionResult = null;
            _actionDone = false;
            var args = JSON.parse(argsJson);
            Promise.resolve(args[0] * 2).then(function(val) {
                _actionResult = JSON.stringify({ result: val });
                _actionDone = true;
            });
            return "__REX_ACTION_ASYNC__";
        };
        globalThis.__rex_resolve_action_pending = function() {
            return _actionDone ? "done" : "pending";
        };
        globalThis.__rex_finalize_action = function() {
            return _actionResult || JSON.stringify({ error: "no result" });
        };
        "#,
    );
    let result = iso.call_server_action("async_act", "[21]").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["result"], 42);
}

#[test]
fn test_load_rsc_bundles() {
    crate::init_v8();
    let bundle = format!("{}\n{}", MOCK_REACT_RUNTIME, make_server_bundle(&[]));
    let mut iso = SsrIsolate::new(&bundle, None).expect("failed to create isolate");

    // Before loading RSC bundles, flight functions should be None
    assert!(iso.rsc_flight_fn.is_none());

    // Minimal flight bundle that sets __rex_render_flight
    let flight_js = r#"
        globalThis.__rex_render_flight = function(routeKey, propsJson) {
            return "flight:" + routeKey;
        };
        globalThis.__rex_render_rsc_to_html = function(routeKey, propsJson) {
            return JSON.stringify({ body: "html:" + routeKey, head: "", flight: "" });
        };
        globalThis.__rex_call_server_action = function(actionId, argsJson) {
            return JSON.stringify({ result: "action:" + actionId });
        };
    "#;
    let ssr_js = r#"
        globalThis.__rex_rsc_flight_to_html = function(flight) {
            return JSON.stringify({ body: "ssr", head: "", flight: flight });
        };
    "#;

    iso.load_rsc_bundles(flight_js, ssr_js).unwrap();

    // After loading, flight function should be set
    assert!(iso.rsc_flight_fn.is_some());
    assert!(iso.rsc_to_html_fn.is_some());
    assert!(iso.server_action_fn.is_some());

    // Server action should work through the loaded function
    let result = iso.call_server_action("test_id", "[]").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["result"], "action:test_id");
}

#[test]
fn test_reload_preserves_server_action_fn() {
    let mut iso = make_isolate_with_actions(
        r#"
        globalThis.__rex_call_server_action = function(actionId, argsJson) {
            return JSON.stringify({ result: "v1" });
        };
        "#,
    );
    let r1 = iso.call_server_action("x", "[]").unwrap();
    assert!(r1.contains("v1"));

    // Reload with new bundle that has updated action
    let new_bundle = format!(
        "{}\n{}\n{}",
        MOCK_REACT_RUNTIME,
        make_server_bundle(&[]),
        r#"
        globalThis.__rex_call_server_action = function(actionId, argsJson) {
            return JSON.stringify({ result: "v2" });
        };
        "#
    );
    iso.reload(&new_bundle).unwrap();
    let r2 = iso.call_server_action("x", "[]").unwrap();
    assert!(r2.contains("v2"));
}
