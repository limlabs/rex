//! `globalThis.fetch()` host function for V8 isolates.
//!
//! Uses a batching model: `fetch()` returns a pending promise and queues the
//! request. The Rust side drains the queue per microtask tick and fires all
//! requests concurrently with `join_all`. This enables `Promise.all([...])` to
//! run in parallel without a JS event loop.

use std::cell::RefCell;
use std::collections::HashMap;

/// A queued fetch request waiting to be dispatched.
pub struct FetchRequest {
    /// The V8 promise resolver to settle when the response arrives.
    pub resolver: v8::Global<v8::PromiseResolver>,
    pub url: String,
    pub method: String,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
}

thread_local! {
    /// Queue of pending fetch requests on this isolate's thread.
    pub static FETCH_QUEUE: RefCell<Vec<FetchRequest>> = const { RefCell::new(Vec::new()) };

    /// Reusable reqwest client (keeps connection pools alive).
    static HTTP_CLIENT: reqwest::Client = reqwest::Client::new();

    /// Single-threaded tokio runtime for `block_on` (one per isolate thread).
    static TOKIO_RT: tokio::runtime::Runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to create tokio runtime for fetch");
}

/// Set a named property on a V8 object. Works with any scope type from V8 macros.
macro_rules! set_v8_prop {
    ($scope:expr, $obj:expr, $name:expr, $value:expr) => {
        if let Some(k) = v8::String::new($scope, $name) {
            $obj.set($scope, k.into(), $value);
        }
    };
}

/// The `fetch(url, init?)` callback. Queues a request and returns a Promise.
///
/// # Security: SSRF risk
///
/// This function makes HTTP requests to **arbitrary URLs** from the server.
/// There is currently **no filtering** of private/internal addresses (RFC 1918,
/// link-local, cloud metadata endpoints like `169.254.169.254`). Server
/// components can reach internal services that are not exposed publicly.
///
/// **TODO:** Add an allowlist/blocklist or deny private IP ranges by default
/// before deploying to production environments with sensitive internal services.
///
/// Register on a global object with:
/// ```ignore
/// let t = v8::FunctionTemplate::new(scope, crate::fetch::fetch_callback);
/// let f = t.get_function(scope).expect("fetch fn");
/// let k = v8::String::new(scope, "fetch").expect("fetch key");
/// global.set(scope, k.into(), f.into());
/// ```
pub fn fetch_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut ret: v8::ReturnValue,
) {
    // Parse arguments.
    if args.length() < 1 || args.get(0).is_undefined() || args.get(0).is_null() {
        ret.set(v8::undefined(scope).into());
        return;
    }
    let url = args.get(0).to_rust_string_lossy(scope);

    let mut method = "GET".to_string();
    let mut headers = HashMap::new();
    let mut body = None;

    if args.length() >= 2 && args.get(1).is_object() {
        if let Some(init) = args.get(1).to_object(scope) {
            // method
            if let Some(method_key) = v8::String::new(scope, "method") {
                if let Some(m) = init.get(scope, method_key.into()) {
                    if !m.is_undefined() && !m.is_null() {
                        method = m.to_rust_string_lossy(scope).to_uppercase();
                    }
                }
            }

            // headers
            if let Some(headers_key) = v8::String::new(scope, "headers") {
                if let Some(h) = init.get(scope, headers_key.into()) {
                    if h.is_object() && !h.is_null() {
                        if let Some(obj) = h.to_object(scope) {
                            if let Some(names) =
                                obj.get_own_property_names(scope, Default::default())
                            {
                                for i in 0..names.length() {
                                    if let Some(key) = names.get_index(scope, i) {
                                        if let Some(val) = obj.get(scope, key) {
                                            headers.insert(
                                                key.to_rust_string_lossy(scope),
                                                val.to_rust_string_lossy(scope),
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // body
            if let Some(body_key) = v8::String::new(scope, "body") {
                if let Some(b) = init.get(scope, body_key.into()) {
                    if !b.is_undefined() && !b.is_null() {
                        body = Some(b.to_rust_string_lossy(scope));
                    }
                }
            }
        }
    }

    // Create the promise.
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        ret.set(v8::undefined(scope).into());
        return;
    };
    let promise = resolver.get_promise(scope);

    // Queue the request.
    let global_resolver = v8::Global::new(scope, resolver);
    FETCH_QUEUE.with(|q| {
        q.borrow_mut().push(FetchRequest {
            resolver: global_resolver,
            url,
            method,
            headers,
            body,
        });
    });

    ret.set(promise.into());
}

/// Result of a single HTTP fetch.
pub struct FetchResult {
    pub status: u16,
    pub status_text: String,
    pub headers: HashMap<String, String>,
    pub body: String,
    pub url: String,
}

/// Drain the fetch queue and return all pending requests.
pub fn drain_fetch_queue() -> Vec<FetchRequest> {
    FETCH_QUEUE.with(|q| q.borrow_mut().drain(..).collect())
}

/// Execute a batch of fetch requests concurrently.
/// Returns results in the same order as the input requests.
pub fn execute_fetch_batch(requests: &[FetchRequest]) -> Vec<Result<FetchResult, String>> {
    if requests.is_empty() {
        return vec![];
    }

    TOKIO_RT.with(|rt| {
        rt.block_on(async {
            let futures: Vec<_> = requests
                .iter()
                .map(|req| {
                    let url = req.url.clone();
                    let method = req.method.clone();
                    let headers = req.headers.clone();
                    let body = req.body.clone();
                    async move { do_fetch(&url, &method, &headers, body.as_deref()).await }
                })
                .collect();
            futures::future::join_all(futures).await
        })
    })
}

/// Perform a single HTTP fetch.
async fn do_fetch(
    url: &str,
    method: &str,
    headers: &HashMap<String, String>,
    body: Option<&str>,
) -> Result<FetchResult, String> {
    let method_parsed = method
        .parse::<reqwest::Method>()
        .map_err(|e| format!("Invalid method: {e}"))?;

    // Clone the client out of the thread-local. reqwest::Client is Arc-based,
    // so this is cheap and the clone can be used across async boundaries.
    let client = HTTP_CLIENT.with(|c| c.clone());

    let mut builder = client.request(method_parsed, url);

    for (k, v) in headers {
        builder = builder.header(k.as_str(), v.as_str());
    }

    if let Some(b) = body {
        builder = builder.body(b.to_string());
    }

    let resp = builder
        .send()
        .await
        .map_err(|e| format!("fetch error: {e}"))?;

    let status = resp.status().as_u16();
    let status_text = resp.status().canonical_reason().unwrap_or("").to_string();
    let url = resp.url().to_string();

    let resp_headers: HashMap<String, String> = resp
        .headers()
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_lowercase(),
                v.to_str().unwrap_or("").to_string(),
            )
        })
        .collect();

    let body_text = resp
        .text()
        .await
        .map_err(|e| format!("fetch body error: {e}"))?;

    Ok(FetchResult {
        status,
        status_text,
        headers: resp_headers,
        body: body_text,
        url,
    })
}

/// Build a Response object on the given scope and resolve the promise.
/// Must be called in a context where `scope` comes from a V8 scope macro.
macro_rules! resolve_fetch_promise {
    ($scope:expr, $resolver:expr, $result:expr) => {{
        let response = v8::Object::new($scope);

        // status
        set_v8_prop!(
            $scope,
            response,
            "status",
            v8::Integer::new($scope, $result.status as i32).into()
        );

        // statusText
        if let Some(v) = v8::String::new($scope, &$result.status_text) {
            set_v8_prop!($scope, response, "statusText", v.into());
        }

        // ok
        set_v8_prop!(
            $scope,
            response,
            "ok",
            v8::Boolean::new($scope, (200..300).contains(&$result.status)).into()
        );

        // url
        if let Some(v) = v8::String::new($scope, &$result.url) {
            set_v8_prop!($scope, response, "url", v.into());
        }

        // headers object with get() method
        let headers_obj = v8::Object::new($scope);
        for (hk, hv) in &$result.headers {
            if let (Some(k), Some(v)) = (v8::String::new($scope, hk), v8::String::new($scope, hv)) {
                headers_obj.set($scope, k.into(), v.into());
            }
        }
        let get_template = v8::FunctionTemplate::new($scope, headers_get_callback);
        if let Some(get_fn) = get_template.get_function($scope) {
            set_v8_prop!($scope, headers_obj, "get", get_fn.into());
        }
        set_v8_prop!($scope, response, "headers", headers_obj.into());

        // _body (internal, used by .json() and .text())
        if let Some(body_str) = v8::String::new($scope, &$result.body) {
            set_v8_prop!($scope, response, "_body", body_str.into());
        }

        // .json()
        let json_template = v8::FunctionTemplate::new($scope, response_json_callback);
        if let Some(json_fn) = json_template.get_function($scope) {
            set_v8_prop!($scope, response, "json", json_fn.into());
        }

        // .text()
        let text_template = v8::FunctionTemplate::new($scope, response_text_callback);
        if let Some(text_fn) = text_template.get_function($scope) {
            set_v8_prop!($scope, response, "text", text_fn.into());
        }

        $resolver.resolve($scope, response.into());
    }};
}

/// Reject a fetch promise with an error message.
macro_rules! reject_fetch_promise {
    ($scope:expr, $resolver:expr, $error_msg:expr) => {
        if let Some(msg) = v8::String::new($scope, $error_msg) {
            let err = v8::Exception::error($scope, msg);
            $resolver.reject($scope, err);
        }
    };
}

/// headers.get(name) implementation
fn headers_get_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut ret: v8::ReturnValue,
) {
    if args.length() < 1 {
        ret.set(v8::undefined(scope).into());
        return;
    }

    let name = args.get(0).to_rust_string_lossy(scope).to_lowercase();
    let this = args.this();

    if let Some(key) = v8::String::new(scope, &name) {
        if let Some(val) = this.get(scope, key.into()) {
            if !val.is_undefined() && !val.is_function() {
                ret.set(val);
                return;
            }
        }
    }
    ret.set(v8::null(scope).into());
}

/// response.json() implementation — returns a pre-resolved promise
fn response_json_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut ret: v8::ReturnValue,
) {
    let this = args.this();
    let Some(body_key) = v8::String::new(scope, "_body") else {
        return;
    };
    let body = this
        .get(scope, body_key.into())
        .unwrap_or_else(|| v8::undefined(scope).into());

    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);

    // Use v8::json::parse which is simpler and correctly throws on invalid JSON
    let json_str = body.to_rust_string_lossy(scope);
    let result = v8::String::new(scope, &json_str).and_then(|s| v8::json::parse(scope, s));

    match result {
        Some(parsed) => {
            resolver.resolve(scope, parsed);
        }
        None => {
            let msg = v8::String::new(scope, "Failed to parse JSON response body")
                .unwrap_or_else(|| v8::String::empty(scope));
            let err = v8::Exception::syntax_error(scope, msg);
            resolver.reject(scope, err);
        }
    }
    ret.set(promise.into());
}

/// response.text() implementation — returns a pre-resolved promise
fn response_text_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut ret: v8::ReturnValue,
) {
    let this = args.this();
    let Some(body_key) = v8::String::new(scope, "_body") else {
        return;
    };
    let body = this
        .get(scope, body_key.into())
        .unwrap_or_else(|| v8::undefined(scope).into());

    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    resolver.resolve(scope, body);
    ret.set(promise.into());
}

/// Default timeout for the fetch loop (30 seconds).
const FETCH_LOOP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Run the batch-and-resolve loop until all async work is settled.
///
/// This is the core pattern that enables `fetch()` in bare V8:
/// 1. Run microtask checkpoint (resolves pending .then chains)
/// 2. Drain the fetch queue
/// 3. If queue is empty, break
/// 4. Fire all queued requests concurrently via join_all
/// 5. Resolve/reject promises
/// 6. Repeat
///
/// Times out after 30 seconds to prevent runaway fetch chains.
pub fn run_fetch_loop(isolate: &mut v8::OwnedIsolate, context: &v8::Global<v8::Context>) {
    let deadline = std::time::Instant::now() + FETCH_LOOP_TIMEOUT;

    loop {
        if std::time::Instant::now() > deadline {
            tracing::error!(
                "fetch loop timed out after {}s — possible infinite fetch chain",
                FETCH_LOOP_TIMEOUT.as_secs()
            );
            // Reject all remaining queued fetches
            let remaining = drain_fetch_queue();
            if !remaining.is_empty() {
                v8::scope_with_context!(scope, isolate, context);
                for req in remaining {
                    let resolver = v8::Local::new(scope, &req.resolver);
                    if let Some(msg) = v8::String::new(scope, "fetch loop timed out") {
                        let err = v8::Exception::error(scope, msg);
                        resolver.reject(scope, err);
                    }
                }
            }
            break;
        }

        isolate.perform_microtask_checkpoint();

        let pending = drain_fetch_queue();
        if pending.is_empty() {
            break;
        }

        let results = execute_fetch_batch(&pending);

        // Resolve/reject each promise
        v8::scope_with_context!(scope, isolate, context);

        for (req, result) in pending.into_iter().zip(results) {
            let resolver = v8::Local::new(scope, &req.resolver);
            match result {
                Ok(ref resp) => {
                    resolve_fetch_promise!(scope, resolver, resp);
                }
                Err(ref e) => {
                    reject_fetch_promise!(scope, resolver, e);
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn init_v8() {
        crate::init_v8();
    }

    #[test]
    fn fetch_queue_starts_empty() {
        let pending = drain_fetch_queue();
        assert!(pending.is_empty());
    }

    #[test]
    fn install_fetch_on_global() {
        init_v8();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        v8::scope!(scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let global = context.global(scope);

        // Install fetch directly using the exported callback
        let t = v8::FunctionTemplate::new(scope, fetch_callback);
        let f = t.get_function(scope).expect("fetch function template");
        let k = v8::String::new(scope, "fetch").expect("fetch string");
        global.set(scope, k.into(), f.into());

        // Verify fetch exists on global
        let k = v8::String::new(scope, "fetch").unwrap();
        let v = global.get(scope, k.into()).unwrap();
        assert!(v.is_function());
    }

    #[test]
    fn fetch_returns_promise() {
        init_v8();
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

            // Call fetch('http://example.com') — should return a Promise
            let code = "typeof fetch('http://example.com')";
            let source = v8::String::new(scope, code).unwrap();
            let script = v8::Script::compile(scope, source, None).unwrap();
            let result = script.run(scope).unwrap();
            assert_eq!(result.to_rust_string_lossy(scope), "object");
        }

        // Should have queued one request
        let pending = drain_fetch_queue();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].url, "http://example.com");
        assert_eq!(pending[0].method, "GET");
    }

    /// Integration test: verifies that `run_fetch_loop` can resolve a fetch promise
    /// end-to-end. Requires a real HTTP server, so marked `#[ignore]` for CI.
    /// Run manually with: `cargo test --package rex_v8 test_run_fetch_loop -- --ignored`
    #[test]
    #[ignore]
    fn test_run_fetch_loop_resolves_promise() {
        init_v8();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());

        let context = {
            v8::scope!(scope, &mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let global = context.global(scope);

            // Install fetch
            let t = v8::FunctionTemplate::new(scope, fetch_callback);
            let f = t.get_function(scope).expect("fetch fn");
            let k = v8::String::new(scope, "fetch").expect("fetch key");
            global.set(scope, k.into(), f.into());

            // Evaluate: fetch a public URL, store result in globalThis._result
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

        // Run the fetch loop to resolve all pending promises
        run_fetch_loop(&mut isolate, &context);

        // Check the result
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
        init_v8();
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
}
