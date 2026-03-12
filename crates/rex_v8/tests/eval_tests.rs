//! Tests for the eval_once convenience function.
#![allow(clippy::unwrap_used)]

#[test]
fn eval_simple_expression() {
    // Non-promise return path
    let result = rex_v8::eval::eval_once("", &[("1 + 2", "test.js")]).unwrap();
    assert_eq!(result, "3");
}

#[test]
fn eval_string_result() {
    let result = rex_v8::eval::eval_once("", &[("'hello world'", "test.js")]).unwrap();
    assert_eq!(result, "hello world");
}

#[test]
fn eval_resolved_promise() {
    let result =
        rex_v8::eval::eval_once("", &[("Promise.resolve('async_val')", "test.js")]).unwrap();
    assert_eq!(result, "async_val");
}

#[test]
fn eval_rejected_promise() {
    let err = rex_v8::eval::eval_once("", &[("Promise.reject('oops')", "test.js")]).unwrap_err();
    assert!(
        err.to_string().contains("oops"),
        "Expected rejection message, got: {err}"
    );
}

#[test]
fn eval_console_usage() {
    // console.log should not crash (noop callback)
    let result = rex_v8::eval::eval_once(
        "",
        &[(
            "console.log('test'); console.warn('w'); console.error('e'); 42",
            "test.js",
        )],
    )
    .unwrap();
    assert_eq!(result, "42");
}

#[test]
fn eval_multiple_scripts() {
    let result = rex_v8::eval::eval_once(
        "",
        &[("globalThis.x = 10", "a.js"), ("globalThis.x + 5", "b.js")],
    )
    .unwrap();
    assert_eq!(result, "15");
}

#[test]
fn eval_with_polyfills() {
    let result = rex_v8::eval::eval_once(
        "globalThis.myHelper = function(x) { return x * 2; };",
        &[("myHelper(21)", "test.js")],
    )
    .unwrap();
    assert_eq!(result, "42");
}
