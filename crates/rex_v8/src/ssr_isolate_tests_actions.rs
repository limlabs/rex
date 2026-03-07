use super::*;

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

#[test]
fn test_call_server_action_encoded() {
    let mut iso = make_isolate_with_actions(
        r#"
        globalThis.__rex_call_server_action_encoded = function(actionId, body) {
            var args = JSON.parse(body);
            if (!Array.isArray(args)) args = [args];
            return JSON.stringify({ result: args[0] + 10 });
        };
        "#,
    );
    let result = iso
        .call_server_action_encoded("enc_act", "[5]", false)
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["result"], 15);
}

#[test]
fn test_call_server_action_encoded_not_loaded() {
    let mut iso = make_isolate(&[]);
    let err = iso
        .call_server_action_encoded("act", "[]", false)
        .unwrap_err();
    assert!(
        err.to_string().contains("not loaded"),
        "Should error when encoded actions not loaded, got: {err}"
    );
}

#[test]
fn test_call_form_action_sync() {
    let mut iso = make_isolate_with_actions(
        r#"
        globalThis.__rex_call_form_action = function(fieldsJson) {
            var fields = JSON.parse(fieldsJson);
            var name = "";
            for (var i = 0; i < fields.length; i++) {
                if (fields[i][0] === "name") name = fields[i][1];
            }
            return JSON.stringify({ result: "Hello, " + name + "!" });
        };
        "#,
    );
    let fields_json = r#"[["name","Alice"],["$ACTION_ID_abc",""]]"#;
    let result = iso.call_form_action("abc", fields_json).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["result"], "Hello, Alice!");
}

#[test]
fn test_call_form_action_not_loaded() {
    let mut iso = make_isolate(&[]);
    let err = iso.call_form_action("act", "[]").unwrap_err();
    assert!(
        err.to_string().contains("not loaded"),
        "Should error when form actions not loaded, got: {err}"
    );
}
