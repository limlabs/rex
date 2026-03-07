/// Evaluate a script in the given scope, using TryCatch for error handling.
/// The scope must already be a ContextScope. Returns the result value.
macro_rules! v8_eval {
    ($scope:expr, $code:expr, $filename:expr) => {{
        // Create a TryCatch scope
        v8::tc_scope!(tc, $scope);

        let source = v8::String::new(tc, $code)
            .ok_or_else(|| anyhow::anyhow!("Failed to create V8 string"))?;
        let name = v8::String::new(tc, $filename)
            .ok_or_else(|| anyhow::anyhow!("Failed to create V8 filename string"))?;
        let origin = v8::ScriptOrigin::new(
            tc,
            name.into(),
            0,
            0,
            false,
            0,
            None,
            false,
            false,
            false,
            None,
        );

        match v8::Script::compile(tc, source, Some(&origin)) {
            Some(script) => match script.run(tc) {
                Some(val) => Ok::<v8::Local<v8::Value>, anyhow::Error>(val),
                None => {
                    let msg = tc
                        .exception()
                        .map(|e| e.to_rust_string_lossy(tc))
                        .unwrap_or_else(|| "Unknown error".into());
                    Err(anyhow::anyhow!("V8 error in {}: {}", $filename, msg))
                }
            },
            None => {
                let msg = tc
                    .exception()
                    .map(|e| e.to_rust_string_lossy(tc))
                    .unwrap_or_else(|| "Unknown compile error".into());
                Err(anyhow::anyhow!(
                    "V8 compile error in {}: {}",
                    $filename,
                    msg
                ))
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
                let msg = tc
                    .exception()
                    .map(|e| e.to_rust_string_lossy(tc))
                    .unwrap_or_else(|| "Unknown call error".into());
                Err(anyhow::anyhow!("{}", msg))
            }
        }
    }};
}

/// Look up a required global function by name.
macro_rules! v8_get_global_fn {
    ($scope:expr, $global:expr, $name:expr) => {{
        let k = v8::String::new($scope, $name)
            .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed for '{}'", $name))?;
        let v = $global
            .get($scope, k.into())
            .ok_or_else(|| anyhow::anyhow!("{} not found", $name))?;
        v8::Local::<v8::Function>::try_from(v)
            .map_err(|_| anyhow::anyhow!("{} is not a function", $name))
    }};
}

/// Look up an optional global function by name.
macro_rules! v8_get_optional_fn {
    ($scope:expr, $global:expr, $name:expr) => {{
        v8::String::new($scope, $name)
            .and_then(|k| $global.get($scope, k.into()))
            .and_then(|v| v8::Local::<v8::Function>::try_from(v).ok())
            .map(|f| v8::Global::new($scope, f))
    }};
}
