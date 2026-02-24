use anyhow::{Context, Result};
use tracing::debug;

/// An SSR isolate that owns a V8 isolate and can render pages.
/// Must be used on the same OS thread that created it (V8 isolates are !Send).
pub struct SsrIsolate {
    isolate: v8::OwnedIsolate,
    context: v8::Global<v8::Context>,
    render_fn: v8::Global<v8::Function>,
    gssp_fn: v8::Global<v8::Function>,
}

/// Evaluate a script in the given scope, using TryCatch for error handling.
/// The scope must already be a ContextScope. Returns the result value.
macro_rules! v8_eval {
    ($scope:expr, $code:expr, $filename:expr) => {{
        // Create a TryCatch scope
        v8::tc_scope!(tc, $scope);

        let source = v8::String::new(tc, $code)
            .ok_or_else(|| anyhow::anyhow!("Failed to create V8 string"))?;
        let name = v8::String::new(tc, $filename).unwrap();
        let origin = v8::ScriptOrigin::new(
            tc, name.into(), 0, 0, false, 0, None, false, false, false, None,
        );

        match v8::Script::compile(tc, source, Some(&origin)) {
            Some(script) => match script.run(tc) {
                Some(val) => Ok::<v8::Local<v8::Value>, anyhow::Error>(val),
                None => {
                    let msg = tc.exception()
                        .map(|e| e.to_rust_string_lossy(tc))
                        .unwrap_or_else(|| "Unknown error".into());
                    Err(anyhow::anyhow!("V8 error in {}: {}", $filename, msg))
                }
            },
            None => {
                let msg = tc.exception()
                    .map(|e| e.to_rust_string_lossy(tc))
                    .unwrap_or_else(|| "Unknown compile error".into());
                Err(anyhow::anyhow!("V8 compile error in {}: {}", $filename, msg))
            }
        }
    }};
}

/// Call a V8 function with args, using TryCatch for error handling.
macro_rules! v8_call {
    ($scope:expr, $func:expr, $recv:expr, $args:expr) => {{
        v8::tc_scope!(tc, $scope);

        match $func.call(tc, $recv, $args) {
            Some(val) => Ok::<v8::Local<v8::Value>, anyhow::Error>(val),
            None => {
                let msg = tc.exception()
                    .map(|e| e.to_rust_string_lossy(tc))
                    .unwrap_or_else(|| "Unknown call error".into());
                Err(anyhow::anyhow!("{}", msg))
            }
        }
    }};
}

impl SsrIsolate {
    /// Create a new SSR isolate and evaluate the server bundle.
    pub fn new(react_runtime_js: &str, server_bundle_js: &str) -> Result<Self> {
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());

        let (context, render_fn, gssp_fn) = {
            v8::scope!(scope, &mut isolate);

            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);

            // Install console + globalThis
            {
                let global = context.global(scope);

                let console = v8::Object::new(scope);

                let t = v8::FunctionTemplate::new(scope, console_log);
                let f = t.get_function(scope).unwrap();
                let k = v8::String::new(scope, "log").unwrap();
                console.set(scope, k.into(), f.into());

                let t = v8::FunctionTemplate::new(scope, console_warn);
                let f = t.get_function(scope).unwrap();
                let k = v8::String::new(scope, "warn").unwrap();
                console.set(scope, k.into(), f.into());

                let t = v8::FunctionTemplate::new(scope, console_error);
                let f = t.get_function(scope).unwrap();
                let k = v8::String::new(scope, "error").unwrap();
                console.set(scope, k.into(), f.into());

                let t = v8::FunctionTemplate::new(scope, console_log);
                let f = t.get_function(scope).unwrap();
                let k = v8::String::new(scope, "info").unwrap();
                console.set(scope, k.into(), f.into());

                let k = v8::String::new(scope, "console").unwrap();
                global.set(scope, k.into(), console.into());

                let k = v8::String::new(scope, "globalThis").unwrap();
                global.set(scope, k.into(), global.into());
            }

            // Evaluate React runtime
            v8_eval!(scope, react_runtime_js, "react-runtime.js")
                .context("Failed to evaluate React runtime")?;

            // Evaluate server bundle
            v8_eval!(scope, server_bundle_js, "server-bundle.js")
                .context("Failed to evaluate server bundle")?;

            // Get global functions
            let ctx = scope.get_current_context();
            let global = ctx.global(scope);

            let k = v8::String::new(scope, "__rex_render_page").unwrap();
            let v = global.get(scope, k.into())
                .ok_or_else(|| anyhow::anyhow!("__rex_render_page not found"))?;
            let render_fn = v8::Local::<v8::Function>::try_from(v)
                .map_err(|_| anyhow::anyhow!("__rex_render_page is not a function"))?;

            let k = v8::String::new(scope, "__rex_get_server_side_props").unwrap();
            let v = global.get(scope, k.into())
                .ok_or_else(|| anyhow::anyhow!("__rex_get_server_side_props not found"))?;
            let gssp_fn = v8::Local::<v8::Function>::try_from(v)
                .map_err(|_| anyhow::anyhow!("__rex_get_server_side_props is not a function"))?;

            (
                v8::Global::new(scope, context),
                v8::Global::new(scope, render_fn),
                v8::Global::new(scope, gssp_fn),
            )
        };

        Ok(Self {
            isolate,
            context,
            render_fn,
            gssp_fn,
        })
    }

    /// Call __rex_render_page(routeKey, propsJson) and return the HTML string.
    pub fn render_page(&mut self, route_key: &str, props_json: &str) -> Result<String> {
        v8::scope_with_context!(scope, &mut self.isolate, &self.context);

        let func = v8::Local::new(scope, &self.render_fn);
        let undef = v8::undefined(scope);
        let arg0 = v8::String::new(scope, route_key)
            .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
        let arg1 = v8::String::new(scope, props_json)
            .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;

        let result = v8_call!(scope, func, undef.into(), &[arg0.into(), arg1.into()])
            .map_err(|e| anyhow::anyhow!("SSR render error: {e}"))?;

        Ok(result.to_rust_string_lossy(scope))
    }

    /// Call __rex_get_server_side_props(routeKey, contextJson) and return JSON.
    /// Handles async GSSP functions by pumping V8's microtask queue.
    pub fn get_server_side_props(&mut self, route_key: &str, context_json: &str) -> Result<String> {
        let result_str = {
            v8::scope_with_context!(scope, &mut self.isolate, &self.context);

            let func = v8::Local::new(scope, &self.gssp_fn);
            let undef = v8::undefined(scope);
            let arg0 = v8::String::new(scope, route_key)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
            let arg1 = v8::String::new(scope, context_json)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;

            let result = v8_call!(scope, func, undef.into(), &[arg0.into(), arg1.into()])
                .map_err(|e| anyhow::anyhow!("GSSP error: {e}"))?;

            result.to_rust_string_lossy(scope)
        };

        if result_str == "__REX_ASYNC__" {
            // GSSP returned a promise — pump V8's microtask queue to resolve it
            self.isolate.perform_microtask_checkpoint();

            v8::scope_with_context!(scope, &mut self.isolate, &self.context);
            let resolve_result = v8_eval!(scope, "globalThis.__rex_resolve_gssp()", "<gssp-resolve>")
                .map_err(|e| anyhow::anyhow!("GSSP error: {e}"))?;
            Ok(resolve_result.to_rust_string_lossy(scope))
        } else {
            Ok(result_str)
        }
    }

    /// Reload the server bundle (for dev mode hot reload)
    pub fn reload(&mut self, server_bundle_js: &str) -> Result<()> {
        v8::scope_with_context!(scope, &mut self.isolate, &self.context);

        v8_eval!(scope, "globalThis.__rex_pages = {};", "<reload>")?;
        v8_eval!(scope, server_bundle_js, "server-bundle.js")
            .context("Failed to evaluate updated server bundle")?;

        let ctx = scope.get_current_context();
        let global = ctx.global(scope);

        let k = v8::String::new(scope, "__rex_render_page").unwrap();
        let v = global.get(scope, k.into()).unwrap();
        let render_fn = v8::Local::<v8::Function>::try_from(v).unwrap();

        let k = v8::String::new(scope, "__rex_get_server_side_props").unwrap();
        let v = global.get(scope, k.into()).unwrap();
        let gssp_fn = v8::Local::<v8::Function>::try_from(v).unwrap();

        self.render_fn = v8::Global::new(scope, render_fn);
        self.gssp_fn = v8::Global::new(scope, gssp_fn);

        debug!("SSR isolate reloaded");
        Ok(())
    }
}

fn format_args(scope: &mut v8::PinScope, args: &v8::FunctionCallbackArguments) -> String {
    let mut parts = Vec::new();
    for i in 0..args.length() {
        let arg = args.get(i);
        parts.push(arg.to_rust_string_lossy(scope));
    }
    parts.join(" ")
}

fn console_log(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _: v8::ReturnValue) {
    tracing::info!(target: "v8::console", "{}", format_args(scope, &args));
}

fn console_warn(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _: v8::ReturnValue) {
    tracing::warn!(target: "v8::console", "{}", format_args(scope, &args));
}

fn console_error(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _: v8::ReturnValue) {
    tracing::error!(target: "v8::console", "{}", format_args(scope, &args));
}
