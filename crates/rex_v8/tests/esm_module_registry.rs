//! Tests for the V8 native ESM module registry.

#![allow(clippy::unwrap_used)]

use rex_v8::esm_module_registry::EsmModuleRegistry;

fn init_v8_once() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        rex_v8::init_v8();
    });
}

/// Helper: create a V8 isolate and context, run a closure with the registry and scope.
fn with_isolate<F>(f: F)
where
    F: FnOnce(&mut EsmModuleRegistry, &mut v8::PinScope),
{
    init_v8_once();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());

    v8::scope!(scope, &mut isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);

    // Install globalThis so modules can reference it
    let global = context.global(scope);
    let k = v8::String::new(scope, "globalThis").unwrap();
    global.set(scope, k.into(), global.into());

    let mut registry = EsmModuleRegistry::new();
    f(&mut registry, scope);

    // Clear thread-local state between tests
    registry.clear();
}

#[test]
fn compile_and_evaluate_simple_module() {
    with_isolate(|registry, scope| {
        // Compile a simple module that sets a global
        registry
            .compile_module(scope, "test:simple", "globalThis.__test_simple = 42;")
            .expect("compile should succeed");

        // Instantiate and evaluate
        registry
            .instantiate_and_evaluate(scope, "test:simple")
            .expect("instantiate+evaluate should succeed");

        // Verify the global was set
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "__test_simple").unwrap();
        let val = global.get(scope, key.into()).unwrap();
        assert_eq!(val.int32_value(scope).unwrap(), 42);
    });
}

#[test]
fn module_importing_another_module() {
    with_isolate(|registry, scope| {
        // Compile a "library" module that exports via globalThis
        registry
            .compile_module(
                scope,
                "lib",
                "globalThis.__lib_value = 100; export const VALUE = 100;",
            )
            .expect("compile lib");

        // Compile a module that imports from the library
        registry
            .compile_module(
                scope,
                "app",
                "import { VALUE } from 'lib';\nglobalThis.__app_result = VALUE + 1;",
            )
            .expect("compile app");

        // Evaluate the app module (should trigger resolve for 'lib')
        registry
            .instantiate_and_evaluate(scope, "app")
            .expect("evaluate app");

        // Verify the result
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "__app_result").unwrap();
        let val = global.get(scope, key.into()).unwrap();
        assert_eq!(val.int32_value(scope).unwrap(), 101);
    });
}

#[test]
fn synthetic_module_wrapping_global() {
    with_isolate(|registry, scope| {
        // Set up a global that the synthetic module will wrap
        {
            v8::tc_scope!(tc, scope);
            let code = v8::String::new(
                tc,
                "globalThis.__test_react = { createElement: function(t) { return t; }, useState: function() { return [0, function(){}]; } };",
            )
            .unwrap();
            let script = v8::Script::compile(tc, code, None).unwrap();
            script.run(tc).unwrap();
        }

        // Create a synthetic module
        registry
            .create_synthetic_module(
                scope,
                "react",
                &["createElement", "useState"],
                "globalThis.__test_react",
            )
            .expect("create synthetic module");

        // Compile a module that imports from the synthetic module
        registry
            .compile_module(
                scope,
                "consumer",
                "import { createElement } from 'react';\nglobalThis.__consumer_result = typeof createElement;",
            )
            .expect("compile consumer");

        // Evaluate
        registry
            .instantiate_and_evaluate(scope, "consumer")
            .expect("evaluate consumer");

        // Verify createElement was available
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "__consumer_result").unwrap();
        let val = global.get(scope, key.into()).unwrap();
        assert_eq!(val.to_rust_string_lossy(scope), "function");
    });
}

#[test]
fn module_recompilation_for_hmr() {
    with_isolate(|registry, scope| {
        // Compile initial version
        registry
            .compile_module(
                scope,
                "page",
                "export const title = 'v1'; globalThis.__page_title = 'v1';",
            )
            .expect("compile v1");

        // Verify it detects changes
        assert!(registry.has_changed("page", "export const title = 'v2';"));
        assert!(!registry.has_changed(
            "page",
            "export const title = 'v1'; globalThis.__page_title = 'v1';"
        ));

        // Remove and recompile with new content
        registry.remove_module("page");
        assert!(!registry.contains("page"));

        registry
            .compile_module(
                scope,
                "page",
                "export const title = 'v2'; globalThis.__page_title = 'v2';",
            )
            .expect("compile v2");

        // Compile a new entry that imports the updated module
        registry
            .compile_module(
                scope,
                "entry",
                "import { title } from 'page';\nglobalThis.__entry_title = title;",
            )
            .expect("compile entry");

        registry
            .instantiate_and_evaluate(scope, "entry")
            .expect("evaluate entry");

        // Verify we get the updated value
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "__entry_title").unwrap();
        let val = global.get(scope, key.into()).unwrap();
        assert_eq!(val.to_rust_string_lossy(scope), "v2");
    });
}

#[test]
fn compile_error_produces_useful_message() {
    with_isolate(|registry, scope| {
        // Invalid JS should fail to compile
        let result = registry.compile_module(scope, "bad.js", "export {{{}}");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("bad.js"),
            "Error should mention the module name: {err_msg}"
        );
    });
}

#[test]
fn unresolved_import_fails_gracefully() {
    with_isolate(|registry, scope| {
        // Compile a module that imports from a non-existent module
        registry
            .compile_module(
                scope,
                "orphan",
                "import { foo } from 'nonexistent';\nglobalThis.x = foo;",
            )
            .expect("compile should succeed (imports are resolved at instantiate time)");

        // Instantiation should fail because 'nonexistent' is not in the registry
        let result = registry.instantiate_and_evaluate(scope, "orphan");
        assert!(result.is_err());
    });
}

#[test]
fn clear_removes_all_modules() {
    with_isolate(|registry, scope| {
        registry
            .compile_module(scope, "a", "export const a = 1;")
            .unwrap();
        registry
            .compile_module(scope, "b", "export const b = 2;")
            .unwrap();

        assert!(registry.contains("a"));
        assert!(registry.contains("b"));

        registry.clear();

        assert!(!registry.contains("a"));
        assert!(!registry.contains("b"));
    });
}
