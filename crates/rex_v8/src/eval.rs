use anyhow::Result;

/// Evaluate a sequence of JS scripts in a one-off V8 isolate.
///
/// Creates a fresh isolate with console/globalThis installed, evaluates
/// the polyfills script, then evaluates each `(source, filename)` pair
/// in order. Returns the string representation of the last script's result.
///
/// If the last script returns a Promise, microtasks are pumped and the
/// resolved value is returned (or the rejection reason as an error).
///
/// This is a convenience function for running self-contained JS outside
/// the SSR pipeline (e.g., Tailwind CSS compilation).
pub fn eval_once(polyfills: &str, scripts: &[(&str, &str)]) -> Result<String> {
    crate::platform::init_v8();

    let mut isolate = v8::Isolate::new(v8::CreateParams::default());

    v8::scope!(scope, &mut isolate);

    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);

    // Install console + globalThis
    {
        let global = context.global(scope);

        let console = v8::Object::new(scope);
        for method in ["log", "warn", "error", "info", "debug"] {
            let t = v8::FunctionTemplate::new(scope, console_noop);
            let f = t
                .get_function(scope)
                .ok_or_else(|| anyhow::anyhow!("Failed to create console.{method}"))?;
            let k = v8::String::new(scope, method)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
            console.set(scope, k.into(), f.into());
        }
        let k = v8::String::new(scope, "console")
            .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
        global.set(scope, k.into(), console.into());

        let k = v8::String::new(scope, "globalThis")
            .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
        global.set(scope, k.into(), global.into());
    }

    // Evaluate polyfills
    v8_eval!(scope, polyfills, "<polyfills>")?;

    // Evaluate all scripts
    let mut last_value = None;
    for &(source, filename) in scripts {
        let val = v8_eval!(scope, source, filename)?;
        last_value = Some(val);
    }

    let result = last_value.ok_or_else(|| anyhow::anyhow!("No scripts to evaluate"))?;

    // Handle Promise results
    if let Ok(promise) = v8::Local::<v8::Promise>::try_from(result) {
        scope.perform_microtask_checkpoint();

        match promise.state() {
            v8::PromiseState::Fulfilled => {
                let value = promise.result(scope);
                let s = value
                    .to_string(scope)
                    .ok_or_else(|| anyhow::anyhow!("Failed to convert result to string"))?;
                Ok(s.to_rust_string_lossy(scope))
            }
            v8::PromiseState::Rejected => {
                let error = promise.result(scope);
                let msg = error.to_rust_string_lossy(scope);
                anyhow::bail!("JS Promise rejected: {msg}");
            }
            v8::PromiseState::Pending => {
                anyhow::bail!("JS Promise still pending after microtask checkpoint");
            }
        }
    } else {
        let s = result
            .to_string(scope)
            .ok_or_else(|| anyhow::anyhow!("Failed to convert result to string"))?;
        Ok(s.to_rust_string_lossy(scope))
    }
}

/// No-op console callback for eval_once isolates.
fn console_noop(
    _scope: &mut v8::PinScope,
    _args: v8::FunctionCallbackArguments,
    _ret: v8::ReturnValue,
) {
}
