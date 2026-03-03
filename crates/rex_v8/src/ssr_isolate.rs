use anyhow::{Context, Result};
use serde::Deserialize;
use tracing::debug;

/// Result of SSR page rendering, containing both body HTML and head elements.
#[derive(Debug, Clone, Deserialize)]
pub struct RenderResult {
    pub body: String,
    #[serde(default)]
    pub head: String,
}

/// Result of RSC two-pass rendering: HTML body + flight data for hydration.
#[derive(Debug, Clone, Deserialize)]
pub struct RscRenderResult {
    pub body: String,
    #[serde(default)]
    pub head: String,
    pub flight: String,
}

/// An SSR isolate that owns a V8 isolate and can render pages.
/// Must be used on the same OS thread that created it (V8 isolates are !Send).
pub struct SsrIsolate {
    isolate: v8::OwnedIsolate,
    context: v8::Global<v8::Context>,
    render_fn: v8::Global<v8::Function>,
    gssp_fn: v8::Global<v8::Function>,
    gsp_fn: v8::Global<v8::Function>,
    api_handler_fn: Option<v8::Global<v8::Function>>,
    document_fn: Option<v8::Global<v8::Function>>,
    middleware_fn: Option<v8::Global<v8::Function>>,
    /// RSC flight data renderer (app/ routes only)
    rsc_flight_fn: Option<v8::Global<v8::Function>>,
    /// RSC two-pass renderer: flight + HTML (app/ routes only)
    rsc_to_html_fn: Option<v8::Global<v8::Function>>,
    mcp_call_fn: Option<v8::Global<v8::Function>>,
    mcp_list_fn: Option<v8::Global<v8::Function>>,
    /// Last successfully loaded bundle, used to restore state on failed reload.
    /// Uses `Arc<String>` to share memory across pool isolates instead of cloning.
    last_bundle: std::sync::Arc<String>,
}

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

impl SsrIsolate {
    /// Create a new SSR isolate and evaluate the server bundle.
    ///
    /// The bundle is self-contained: it includes V8 polyfills, React,
    /// all pages, and SSR runtime functions in a single IIFE.
    ///
    /// If `project_root` is provided, `fs` polyfill callbacks are registered
    /// on globalThis and sandboxed to that directory.
    pub fn new(server_bundle_js: &str, project_root: Option<&str>) -> Result<Self> {
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());

        let (
            context,
            render_fn,
            gssp_fn,
            gsp_fn,
            api_handler_fn,
            document_fn,
            middleware_fn,
            rsc_flight_fn,
            rsc_to_html_fn,
            mcp_call_fn,
            mcp_list_fn,
        ) = {
            v8::scope!(scope, &mut isolate);

            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);

            // Install console + globalThis
            {
                let global = context.global(scope);

                let console = v8::Object::new(scope);

                let t = v8::FunctionTemplate::new(scope, console_log);
                let f = t
                    .get_function(scope)
                    .ok_or_else(|| anyhow::anyhow!("Failed to create console.log"))?;
                let k = v8::String::new(scope, "log")
                    .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
                console.set(scope, k.into(), f.into());

                let t = v8::FunctionTemplate::new(scope, console_warn);
                let f = t
                    .get_function(scope)
                    .ok_or_else(|| anyhow::anyhow!("Failed to create console.warn"))?;
                let k = v8::String::new(scope, "warn")
                    .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
                console.set(scope, k.into(), f.into());

                let t = v8::FunctionTemplate::new(scope, console_error);
                let f = t
                    .get_function(scope)
                    .ok_or_else(|| anyhow::anyhow!("Failed to create console.error"))?;
                let k = v8::String::new(scope, "error")
                    .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
                console.set(scope, k.into(), f.into());

                let t = v8::FunctionTemplate::new(scope, console_log);
                let f = t
                    .get_function(scope)
                    .ok_or_else(|| anyhow::anyhow!("Failed to create console.info"))?;
                let k = v8::String::new(scope, "info")
                    .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
                console.set(scope, k.into(), f.into());

                let k = v8::String::new(scope, "console")
                    .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
                global.set(scope, k.into(), console.into());

                let k = v8::String::new(scope, "globalThis")
                    .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
                global.set(scope, k.into(), global.into());

                // Install globalThis.fetch
                let t = v8::FunctionTemplate::new(scope, crate::fetch::fetch_callback);
                let f = t
                    .get_function(scope)
                    .ok_or_else(|| anyhow::anyhow!("Failed to create fetch"))?;
                let k = v8::String::new(scope, "fetch")
                    .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
                global.set(scope, k.into(), f.into());

                // Register fs polyfill callbacks
                crate::fs::register_fs_callbacks(scope, global)?;

                // Set project root for fs sandboxing
                if let Some(root) = project_root {
                    let k = v8::String::new(scope, "__rex_project_root")
                        .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
                    let v = v8::String::new(scope, root)
                        .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
                    global.set(scope, k.into(), v.into());
                }
            }

            // Inject process.env from Rust's environment variables.
            // This runs before the bundle so the banner polyfill's
            // `if (typeof globalThis.process === 'undefined')` check
            // sees it already exists and skips the stub.
            let process_env_script = build_process_env_script();
            v8_eval!(scope, &process_env_script, "<process-env>")
                .context("Failed to inject process.env")?;

            // Evaluate the self-contained server bundle
            v8_eval!(scope, server_bundle_js, "server-bundle.js")
                .context("Failed to evaluate server bundle")?;

            // Get global functions
            let ctx = scope.get_current_context();
            let global = ctx.global(scope);

            let render_fn = v8_get_global_fn!(scope, global, "__rex_render_page")?;
            let gssp_fn = v8_get_global_fn!(scope, global, "__rex_get_server_side_props")?;
            let gsp_fn = v8_get_global_fn!(scope, global, "__rex_get_static_props")?;

            // API handler is optional — only present when api/ routes exist
            let api_handler_fn = v8_get_optional_fn!(scope, global, "__rex_call_api_handler");

            // Document renderer is optional — only present when _document exists
            let document_fn = v8_get_optional_fn!(scope, global, "__rex_render_document");

            // Middleware is optional — only present when middleware.ts exists
            let middleware_fn = v8_get_optional_fn!(scope, global, "__rex_run_middleware");

            // RSC functions — only present when app/ routes exist
            let rsc_flight_fn = v8_get_optional_fn!(scope, global, "__rex_render_flight");
            let rsc_to_html_fn = v8_get_optional_fn!(scope, global, "__rex_render_rsc_to_html");

            // MCP tools are optional — only present when mcp/ directory has tool files
            let mcp_call_fn = v8_get_optional_fn!(scope, global, "__rex_call_mcp_tool");
            let mcp_list_fn = v8_get_optional_fn!(scope, global, "__rex_list_mcp_tools");

            (
                v8::Global::new(scope, context),
                v8::Global::new(scope, render_fn),
                v8::Global::new(scope, gssp_fn),
                v8::Global::new(scope, gsp_fn),
                api_handler_fn,
                document_fn,
                middleware_fn,
                rsc_flight_fn,
                rsc_to_html_fn,
                mcp_call_fn,
                mcp_list_fn,
            )
        };

        Ok(Self {
            isolate,
            context,
            render_fn,
            gssp_fn,
            gsp_fn,
            api_handler_fn,
            document_fn,
            middleware_fn,
            rsc_flight_fn,
            rsc_to_html_fn,
            mcp_call_fn,
            mcp_list_fn,
            last_bundle: std::sync::Arc::new(server_bundle_js.to_string()),
        })
    }

    /// Load RSC bundles (flight + SSR) into the V8 context.
    ///
    /// Both bundles are IIFEs evaluated sequentially in the same context.
    /// The flight bundle sets `__rex_render_flight`, `__rex_render_rsc_to_html`, etc.
    /// The SSR bundle sets `__rex_rsc_flight_to_html`, `__rex_resolve_ssr_pending`, etc.
    pub fn load_rsc_bundles(&mut self, flight_bundle_js: &str, ssr_bundle_js: &str) -> Result<()> {
        {
            v8::scope_with_context!(scope, &mut self.isolate, &self.context);

            // Evaluate the flight bundle (sets __rex_render_flight, etc.)
            v8_eval!(scope, flight_bundle_js, "rsc-server-bundle.js")
                .context("Failed to evaluate RSC flight bundle")?;

            // Evaluate the SSR bundle (sets __rex_rsc_flight_to_html, etc.)
            v8_eval!(scope, ssr_bundle_js, "rsc-ssr-bundle.js")
                .context("Failed to evaluate RSC SSR bundle")?;

            // Re-lookup RSC functions now that both bundles are loaded
            let ctx = scope.get_current_context();
            let global = ctx.global(scope);

            self.rsc_flight_fn = v8_get_optional_fn!(scope, global, "__rex_render_flight");
            self.rsc_to_html_fn = v8_get_optional_fn!(scope, global, "__rex_render_rsc_to_html");
        }

        debug!("RSC bundles loaded into V8 context");
        Ok(())
    }

    /// Call __rex_render_page(routeKey, propsJson) and return the rendered body + head HTML.
    pub fn render_page(&mut self, route_key: &str, props_json: &str) -> Result<RenderResult> {
        v8::scope_with_context!(scope, &mut self.isolate, &self.context);

        let func = v8::Local::new(scope, &self.render_fn);
        let undef = v8::undefined(scope);
        let arg0 = v8::String::new(scope, route_key)
            .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
        let arg1 = v8::String::new(scope, props_json)
            .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;

        let result = v8_call!(scope, func, undef.into(), &[arg0.into(), arg1.into()])
            .map_err(|e| anyhow::anyhow!("SSR render error: {e}"))?;

        let json_str = result.to_rust_string_lossy(scope);
        serde_json::from_str(&json_str).context("Failed to parse render result JSON")
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
            // GSSP returned a promise — run the fetch loop to resolve any fetch()
            // calls, then pump microtasks to settle the promise chain.
            crate::fetch::run_fetch_loop(&mut self.isolate, &self.context);

            v8::scope_with_context!(scope, &mut self.isolate, &self.context);
            let resolve_result =
                v8_eval!(scope, "globalThis.__rex_resolve_gssp()", "<gssp-resolve>")
                    .map_err(|e| anyhow::anyhow!("GSSP error: {e}"))?;
            Ok(resolve_result.to_rust_string_lossy(scope))
        } else {
            Ok(result_str)
        }
    }

    /// Call __rex_get_static_props(routeKey, contextJson) and return JSON.
    /// Handles async GSP functions by pumping V8's microtask queue.
    pub fn get_static_props(&mut self, route_key: &str, context_json: &str) -> Result<String> {
        let result_str = {
            v8::scope_with_context!(scope, &mut self.isolate, &self.context);

            let func = v8::Local::new(scope, &self.gsp_fn);
            let undef = v8::undefined(scope);
            let arg0 = v8::String::new(scope, route_key)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
            let arg1 = v8::String::new(scope, context_json)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;

            let result = v8_call!(scope, func, undef.into(), &[arg0.into(), arg1.into()])
                .map_err(|e| anyhow::anyhow!("GSP error: {e}"))?;

            result.to_rust_string_lossy(scope)
        };

        if result_str == "__REX_GSP_ASYNC__" {
            crate::fetch::run_fetch_loop(&mut self.isolate, &self.context);

            v8::scope_with_context!(scope, &mut self.isolate, &self.context);
            let resolve_result = v8_eval!(scope, "globalThis.__rex_resolve_gsp()", "<gsp-resolve>")
                .map_err(|e| anyhow::anyhow!("GSP error: {e}"))?;
            Ok(resolve_result.to_rust_string_lossy(scope))
        } else {
            Ok(result_str)
        }
    }

    /// Call __rex_call_api_handler(routeKey, reqJson) and return JSON response.
    /// Handles async handlers by pumping V8's microtask queue.
    pub fn call_api_handler(&mut self, route_key: &str, req_json: &str) -> Result<String> {
        let api_fn = self
            .api_handler_fn
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("API handlers not loaded"))?;

        let result_str = {
            v8::scope_with_context!(scope, &mut self.isolate, &self.context);

            let func = v8::Local::new(scope, api_fn);
            let undef = v8::undefined(scope);
            let arg0 = v8::String::new(scope, route_key)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
            let arg1 = v8::String::new(scope, req_json)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;

            let result = v8_call!(scope, func, undef.into(), &[arg0.into(), arg1.into()])
                .map_err(|e| anyhow::anyhow!("API handler error: {e}"))?;

            result.to_rust_string_lossy(scope)
        };

        if result_str == "__REX_API_ASYNC__" {
            crate::fetch::run_fetch_loop(&mut self.isolate, &self.context);

            v8::scope_with_context!(scope, &mut self.isolate, &self.context);
            let resolve_result = v8_eval!(scope, "globalThis.__rex_resolve_api()", "<api-resolve>")
                .map_err(|e| anyhow::anyhow!("API handler error: {e}"))?;
            Ok(resolve_result.to_rust_string_lossy(scope))
        } else {
            Ok(result_str)
        }
    }

    /// Call __rex_render_document() to get document descriptor JSON.
    /// Returns None if no custom _document is loaded.
    pub fn render_document(&mut self) -> Result<Option<String>> {
        let doc_fn = match &self.document_fn {
            Some(f) => f,
            None => return Ok(None),
        };

        v8::scope_with_context!(scope, &mut self.isolate, &self.context);

        let func = v8::Local::new(scope, doc_fn);
        let undef = v8::undefined(scope);

        let result = v8_call!(scope, func, undef.into(), &[])
            .map_err(|e| anyhow::anyhow!("Document render error: {e}"))?;

        Ok(Some(result.to_rust_string_lossy(scope)))
    }

    /// Call __rex_run_middleware(reqJson) and return JSON result.
    /// Returns Ok(None) if no middleware is loaded.
    /// Handles async middleware by pumping V8's microtask queue.
    pub fn run_middleware(&mut self, req_json: &str) -> Result<Option<String>> {
        let mw_fn = match &self.middleware_fn {
            Some(f) => f,
            None => return Ok(None),
        };

        let result_str = {
            v8::scope_with_context!(scope, &mut self.isolate, &self.context);

            let func = v8::Local::new(scope, mw_fn);
            let undef = v8::undefined(scope);
            let arg0 = v8::String::new(scope, req_json)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;

            let result = v8_call!(scope, func, undef.into(), &[arg0.into()])
                .map_err(|e| anyhow::anyhow!("Middleware error: {e}"))?;

            result.to_rust_string_lossy(scope)
        };

        if result_str == "__REX_MW_ASYNC__" {
            crate::fetch::run_fetch_loop(&mut self.isolate, &self.context);

            v8::scope_with_context!(scope, &mut self.isolate, &self.context);
            let resolve_result = v8_eval!(
                scope,
                "globalThis.__rex_resolve_middleware()",
                "<mw-resolve>"
            )
            .map_err(|e| anyhow::anyhow!("Middleware error: {e}"))?;
            Ok(Some(resolve_result.to_rust_string_lossy(scope)))
        } else {
            Ok(Some(result_str))
        }
    }

    /// Render RSC flight data for a route (app/ routes only).
    /// Returns the flight data string for client-side navigation.
    /// Handles async server components via iterative resolve loop.
    pub fn render_rsc_flight(&mut self, route_key: &str, props_json: &str) -> Result<String> {
        let rsc_fn = self
            .rsc_flight_fn
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("RSC flight renderer not loaded"))?;

        let result_str = {
            v8::scope_with_context!(scope, &mut self.isolate, &self.context);

            let func = v8::Local::new(scope, rsc_fn);
            let undef = v8::undefined(scope);
            let arg0 = v8::String::new(scope, route_key)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
            let arg1 = v8::String::new(scope, props_json)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;

            let result = v8_call!(scope, func, undef.into(), &[arg0.into(), arg1.into()])
                .map_err(|e| anyhow::anyhow!("RSC flight render error: {e}"))?;

            result.to_rust_string_lossy(scope)
        };

        // Run fetch loop in case server components used fetch()
        crate::fetch::run_fetch_loop(&mut self.isolate, &self.context);

        if result_str == "__REX_RSC_ASYNC__" {
            self.resolve_rsc_async()?;

            let finalized = {
                v8::scope_with_context!(scope, &mut self.isolate, &self.context);
                let result = v8_eval!(
                    scope,
                    "globalThis.__rex_finalize_rsc_flight()",
                    "<rsc-finalize-flight>"
                )
                .map_err(|e| anyhow::anyhow!("RSC finalize error: {e}"))?;
                result.to_rust_string_lossy(scope)
            };
            return Ok(finalized);
        }

        Ok(result_str)
    }

    /// Two-pass RSC render: flight data + HTML (app/ routes only).
    /// Returns RenderResult with body HTML, head, and flight data.
    /// Handles async server components via iterative resolve loop.
    pub fn render_rsc_to_html(
        &mut self,
        route_key: &str,
        props_json: &str,
    ) -> Result<RscRenderResult> {
        let rsc_fn = self
            .rsc_to_html_fn
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("RSC HTML renderer not loaded"))?;

        let result_str = {
            v8::scope_with_context!(scope, &mut self.isolate, &self.context);

            let func = v8::Local::new(scope, rsc_fn);
            let undef = v8::undefined(scope);
            let arg0 = v8::String::new(scope, route_key)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
            let arg1 = v8::String::new(scope, props_json)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;

            let result = v8_call!(scope, func, undef.into(), &[arg0.into(), arg1.into()])
                .map_err(|e| anyhow::anyhow!("RSC render error: {e}"))?;

            result.to_rust_string_lossy(scope)
        };

        // Run fetch loop in case server components used fetch()
        crate::fetch::run_fetch_loop(&mut self.isolate, &self.context);

        if result_str == "__REX_RSC_HTML_ASYNC__" {
            self.resolve_rsc_async()?;

            let finalized = {
                v8::scope_with_context!(scope, &mut self.isolate, &self.context);
                let result = v8_eval!(
                    scope,
                    "globalThis.__rex_finalize_rsc_to_html()",
                    "<rsc-finalize-html>"
                )
                .map_err(|e| anyhow::anyhow!("RSC finalize error: {e}"))?;
                result.to_rust_string_lossy(scope)
            };

            let parsed: RscRenderResult =
                serde_json::from_str(&finalized).context("Failed to parse RSC finalize result")?;
            return Ok(parsed);
        }

        let parsed: RscRenderResult =
            serde_json::from_str(&result_str).context("Failed to parse RSC render result")?;
        Ok(parsed)
    }

    /// Iterative resolve loop for async server components.
    /// Runs fetch loop + microtask pump, then calls __rex_resolve_rsc_pending()
    /// until all async slots are resolved (or timeout).
    fn resolve_rsc_async(&mut self) -> Result<()> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            if std::time::Instant::now() > deadline {
                return Err(anyhow::anyhow!("RSC async resolution timed out after 30s"));
            }

            crate::fetch::run_fetch_loop(&mut self.isolate, &self.context);

            let status = {
                v8::scope_with_context!(scope, &mut self.isolate, &self.context);
                let result = v8_eval!(
                    scope,
                    "globalThis.__rex_resolve_rsc_pending()",
                    "<rsc-resolve>"
                )
                .map_err(|e| anyhow::anyhow!("RSC resolve error: {e}"))?;
                result.to_rust_string_lossy(scope)
            };

            match status.as_str() {
                "done" => break,
                "pending" => {
                    // Yield briefly to avoid CPU-spinning when async slots are
                    // waiting on microtasks but no fetch requests are queued.
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                }
                other => {
                    return Err(anyhow::anyhow!("Unexpected RSC resolve status: {}", other));
                }
            }
        }
        Ok(())
    }

    /// List registered MCP tools. Returns JSON array of {name, description, parameters}.
    /// Returns Ok(None) if no MCP tools are loaded.
    pub fn list_mcp_tools(&mut self) -> Result<Option<String>> {
        let list_fn = match &self.mcp_list_fn {
            Some(f) => f,
            None => return Ok(None),
        };

        v8::scope_with_context!(scope, &mut self.isolate, &self.context);

        let func = v8::Local::new(scope, list_fn);
        let undef = v8::undefined(scope);

        let result = v8_call!(scope, func, undef.into(), &[])
            .map_err(|e| anyhow::anyhow!("MCP list error: {e}"))?;

        Ok(Some(result.to_rust_string_lossy(scope)))
    }

    /// Call an MCP tool by name with JSON parameters. Returns JSON result.
    /// Handles async tool handlers by pumping V8's microtask queue.
    pub fn call_mcp_tool(&mut self, name: &str, params_json: &str) -> Result<String> {
        let call_fn = self
            .mcp_call_fn
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("MCP tools not loaded"))?;

        let result_str = {
            v8::scope_with_context!(scope, &mut self.isolate, &self.context);

            let func = v8::Local::new(scope, call_fn);
            let undef = v8::undefined(scope);
            let arg0 = v8::String::new(scope, name)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
            let arg1 = v8::String::new(scope, params_json)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;

            let result = v8_call!(scope, func, undef.into(), &[arg0.into(), arg1.into()])
                .map_err(|e| anyhow::anyhow!("MCP tool error: {e}"))?;

            result.to_rust_string_lossy(scope)
        };

        if result_str == "__REX_MCP_ASYNC__" {
            self.isolate.perform_microtask_checkpoint();

            v8::scope_with_context!(scope, &mut self.isolate, &self.context);
            let resolve_result = v8_eval!(scope, "globalThis.__rex_resolve_mcp()", "<mcp-resolve>")
                .map_err(|e| anyhow::anyhow!("MCP tool error: {e}"))?;
            Ok(resolve_result.to_rust_string_lossy(scope))
        } else {
            Ok(result_str)
        }
    }

    /// Reload the server bundle (for dev mode hot reload)
    pub fn reload(&mut self, server_bundle_js: &str) -> Result<()> {
        v8::scope_with_context!(scope, &mut self.isolate, &self.context);

        // Clear page registry and try the new bundle
        v8_eval!(scope, "globalThis.__rex_pages = {};", "<reload>")?;
        if let Err(e) = v8_eval!(scope, server_bundle_js, "server-bundle.js") {
            // Restore the last working bundle so the isolate remains functional
            tracing::warn!("New bundle failed, restoring previous: {e}");
            v8_eval!(scope, "globalThis.__rex_pages = {};", "<restore>")?;
            v8_eval!(scope, &self.last_bundle, "server-bundle.js")
                .context("Failed to restore previous server bundle")?;
            return Err(e.context("Failed to evaluate updated server bundle"));
        }

        let ctx = scope.get_current_context();
        let global = ctx.global(scope);

        let render_fn = v8_get_global_fn!(scope, global, "__rex_render_page")?;
        let gssp_fn = v8_get_global_fn!(scope, global, "__rex_get_server_side_props")?;
        let gsp_fn = v8_get_global_fn!(scope, global, "__rex_get_static_props")?;

        self.render_fn = v8::Global::new(scope, render_fn);
        self.gssp_fn = v8::Global::new(scope, gssp_fn);
        self.gsp_fn = v8::Global::new(scope, gsp_fn);

        // Re-lookup API handler (may be added/removed on reload)
        self.api_handler_fn = v8_get_optional_fn!(scope, global, "__rex_call_api_handler");

        // Re-lookup document renderer (may be added/removed on reload)
        self.document_fn = v8_get_optional_fn!(scope, global, "__rex_render_document");

        // Re-lookup middleware (may be added/removed on reload)
        self.middleware_fn = v8_get_optional_fn!(scope, global, "__rex_run_middleware");

        // Re-lookup RSC functions (may be added/removed on reload)
        self.rsc_flight_fn = v8_get_optional_fn!(scope, global, "__rex_render_flight");
        self.rsc_to_html_fn = v8_get_optional_fn!(scope, global, "__rex_render_rsc_to_html");

        // Re-lookup MCP tools (may be added/removed on reload)
        self.mcp_call_fn = v8_get_optional_fn!(scope, global, "__rex_call_mcp_tool");
        self.mcp_list_fn = v8_get_optional_fn!(scope, global, "__rex_list_mcp_tools");

        self.last_bundle = std::sync::Arc::new(server_bundle_js.to_string());
        debug!("SSR isolate reloaded");
        Ok(())
    }
}

/// Build a JS snippet that sets `globalThis.process` with `env` populated
/// from the current Rust process's environment variables.
/// The env object is mutable (not frozen) to match Node.js behavior where
/// libraries commonly assign to `process.env` at runtime.
fn build_process_env_script() -> String {
    use std::fmt::Write;

    let mut pairs = String::new();
    for (key, value) in std::env::vars() {
        // JSON-encode both key and value to safely handle special characters
        let key_json = serde_json::to_string(&key).unwrap_or_default();
        let val_json = serde_json::to_string(&value).unwrap_or_default();
        if !pairs.is_empty() {
            pairs.push(',');
        }
        let _ = write!(pairs, "{key_json}:{val_json}");
    }
    format!("globalThis.process = {{ env: {{{pairs}}} }};")
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

fn console_error(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _: v8::ReturnValue,
) {
    tracing::error!(target: "v8::console", "{}", format_args(scope, &args));
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Minimal React stub: provides React.createElement and ReactDOMServer.renderToString
    /// without needing node_modules. Renders elements as simple HTML strings.
    const MOCK_REACT_RUNTIME: &str = r#"
        globalThis.__React = {
            createElement: function(type, props) {
                var children = Array.prototype.slice.call(arguments, 2);
                return { type: type, props: props || {}, children: children };
            },
            Suspense: Symbol.for('react.suspense')
        };
        var React = globalThis.__React;

        function renderElement(el) {
            if (el === null || el === undefined) return '';
            if (typeof el === 'string') return el;
            if (typeof el === 'number') return String(el);
            if (Array.isArray(el)) return el.map(renderElement).join('');
            if (el.type === globalThis.__React.Suspense) {
                try {
                    var inner = '';
                    if (el.props && el.props.children) inner += renderElement(el.props.children);
                    inner += el.children.map(renderElement).join('');
                    return inner;
                } catch (e) {
                    if (e && typeof e.then === 'function') {
                        return renderElement(el.props && el.props.fallback);
                    }
                    throw e;
                }
            }
            if (typeof el.type === 'function') {
                var merged = Object.assign({}, el.props);
                if (el.children.length > 0) merged.children = el.children.length === 1 ? el.children[0] : el.children;
                return renderElement(el.type(merged));
            }
            if (typeof el.type === 'string') {
                var attrs = '';
                var p = el.props || {};
                for (var k in p) {
                    if (k === 'children' || k === 'dangerouslySetInnerHTML') continue;
                    if (!p.hasOwnProperty(k)) continue;
                    var v = p[k];
                    // Skip event handlers and undefined/null
                    if (typeof v === 'function' || v === null || v === undefined) continue;
                    // Convert React prop names to HTML attributes
                    var attrName = k;
                    if (k === 'className') attrName = 'class';
                    else if (k === 'htmlFor') attrName = 'for';
                    else if (k === 'fetchPriority') attrName = 'fetchpriority';
                    // Serialize style objects to CSS strings
                    if (k === 'style' && typeof v === 'object') {
                        var css = '';
                        for (var sk in v) {
                            if (v.hasOwnProperty(sk)) {
                                // Convert camelCase to kebab-case
                                var prop = sk.replace(/([A-Z])/g, '-$1').toLowerCase();
                                var sv = v[sk];
                                if (typeof sv === 'number' && sv !== 0 && prop !== 'opacity' && prop !== 'z-index' && prop !== 'font-weight' && prop !== 'line-height' && prop !== 'flex' && prop !== 'order') sv = sv + 'px';
                                css += prop + ':' + sv + ';';
                            }
                        }
                        attrs += ' style="' + css + '"';
                        continue;
                    }
                    // Boolean attributes
                    if (v === true) { attrs += ' ' + attrName; continue; }
                    if (v === false) continue;
                    attrs += ' ' + attrName + '="' + String(v).replace(/&/g,'&amp;').replace(/"/g,'&quot;') + '"';
                }
                var inner = '';
                if (p.children) inner += renderElement(p.children);
                inner += el.children.map(renderElement).join('');
                if (!inner) return '<' + el.type + attrs + '/>';
                return '<' + el.type + attrs + '>' + inner + '</' + el.type + '>';
            }
            return '';
        }

        globalThis.__ReactDOMServer = {
            renderToString: function(el) { return renderElement(el); }
        };
    "#;

    /// Page definition for test server bundle.
    struct TestPage<'a> {
        key: &'a str,
        component: &'a str,
        gssp: Option<&'a str>,
        gsp: Option<&'a str>,
    }

    /// Build a minimal server bundle JS with given page definitions.
    /// Each page entry: (route_key, component_js, gssp_js)
    fn make_server_bundle(pages: &[(&str, &str, Option<&str>)]) -> String {
        let test_pages: Vec<TestPage> = pages
            .iter()
            .map(|(key, component, gssp)| TestPage {
                key,
                component,
                gssp: *gssp,
                gsp: None,
            })
            .collect();
        make_server_bundle_ext(&test_pages)
    }

    fn make_server_bundle_ext(pages: &[TestPage]) -> String {
        let mut bundle = String::new();
        bundle.push_str("'use strict';\n");
        bundle.push_str("globalThis.__rex_pages = globalThis.__rex_pages || {};\n\n");

        for page in pages {
            bundle.push_str(&format!(
                "globalThis.__rex_pages['{}'] = (function() {{\n  var exports = {{}};\n",
                page.key
            ));
            bundle.push_str(&format!("  exports.default = {};\n", page.component));
            if let Some(gssp_code) = page.gssp {
                bundle.push_str(&format!("  exports.getServerSideProps = {};\n", gssp_code));
            }
            if let Some(gsp_code) = page.gsp {
                bundle.push_str(&format!("  exports.getStaticProps = {};\n", gsp_code));
            }
            bundle.push_str("  return exports;\n})();\n\n");
        }

        // SSR runtime (same as bundler.rs produces)
        bundle.push_str(
            r#"
globalThis.__rex_head_elements = [];
globalThis.__rex_head_component = function Head(props) {
    if (props.children) {
        var children = Array.isArray(props.children) ? props.children : [props.children];
        for (var i = 0; i < children.length; i++) {
            if (children[i]) globalThis.__rex_head_elements.push(children[i]);
        }
    }
    return null;
};

globalThis.__rex_render_page = function(routeKey, propsJson) {
    var React = globalThis.__React;
    var ReactDOMServer = globalThis.__ReactDOMServer;
    if (!React || !ReactDOMServer) {
        throw new Error('React/ReactDOMServer not loaded');
    }
    var page = globalThis.__rex_pages[routeKey];
    if (!page) throw new Error('Page not found: ' + routeKey);
    var Component = page.default;
    if (!Component) throw new Error('No default export: ' + routeKey);
    var props = JSON.parse(propsJson);

    globalThis.__rex_head_elements = [];
    var element = React.createElement(Component, props);
    var bodyHtml = ReactDOMServer.renderToString(element);

    var headHtml = '';
    for (var i = 0; i < globalThis.__rex_head_elements.length; i++) {
        headHtml += ReactDOMServer.renderToString(globalThis.__rex_head_elements[i]);
    }

    return JSON.stringify({ body: bodyHtml, head: headHtml });
};

globalThis.__rex_gssp_resolved = null;
globalThis.__rex_gssp_rejected = null;

globalThis.__rex_get_server_side_props = function(routeKey, contextJson) {
    var page = globalThis.__rex_pages[routeKey];
    if (!page || !page.getServerSideProps) {
        return JSON.stringify({ props: {} });
    }
    var context = JSON.parse(contextJson);
    var result = page.getServerSideProps(context);
    if (result && typeof result.then === 'function') {
        globalThis.__rex_gssp_resolved = null;
        globalThis.__rex_gssp_rejected = null;
        result.then(
            function(v) { globalThis.__rex_gssp_resolved = v; },
            function(e) { globalThis.__rex_gssp_rejected = e; }
        );
        return '__REX_ASYNC__';
    }
    return JSON.stringify(result);
};

globalThis.__rex_resolve_gssp = function() {
    if (globalThis.__rex_gssp_rejected) throw globalThis.__rex_gssp_rejected;
    if (globalThis.__rex_gssp_resolved !== null) return JSON.stringify(globalThis.__rex_gssp_resolved);
    throw new Error('GSSP promise did not resolve');
};

globalThis.__rex_gsp_resolved = null;
globalThis.__rex_gsp_rejected = null;

globalThis.__rex_get_static_props = function(routeKey, contextJson) {
    var page = globalThis.__rex_pages[routeKey];
    if (!page || !page.getStaticProps) return JSON.stringify({ props: {} });
    var context = JSON.parse(contextJson);
    var result = page.getStaticProps(context);
    if (result && typeof result.then === 'function') {
        globalThis.__rex_gsp_resolved = null;
        globalThis.__rex_gsp_rejected = null;
        result.then(
            function(v) { globalThis.__rex_gsp_resolved = v; },
            function(e) { globalThis.__rex_gsp_rejected = e; }
        );
        return '__REX_GSP_ASYNC__';
    }
    return JSON.stringify(result);
};

globalThis.__rex_resolve_gsp = function() {
    if (globalThis.__rex_gsp_rejected) throw globalThis.__rex_gsp_rejected;
    if (globalThis.__rex_gsp_resolved !== null) return JSON.stringify(globalThis.__rex_gsp_resolved);
    throw new Error('GSP promise did not resolve');
};
"#,
        );
        bundle
    }

    fn make_isolate(pages: &[(&str, &str, Option<&str>)]) -> SsrIsolate {
        crate::init_v8();
        let bundle = format!("{}\n{}", MOCK_REACT_RUNTIME, make_server_bundle(pages));
        SsrIsolate::new(&bundle, None).expect("failed to create isolate")
    }

    #[test]
    fn test_render_simple_page() {
        let mut iso = make_isolate(&[(
            "index",
            "function Index() { return React.createElement('h1', null, 'Hello'); }",
            None,
        )]);
        let result = iso.render_page("index", "{}").unwrap();
        assert_eq!(result.body, "<h1>Hello</h1>");
        assert_eq!(result.head, "");
    }

    #[test]
    fn test_render_with_props() {
        let mut iso = make_isolate(&[(
            "greet",
            "function Greet(props) { return React.createElement('p', null, 'Hi ' + props.name); }",
            None,
        )]);
        let result = iso.render_page("greet", r#"{"name":"Rex"}"#).unwrap();
        assert_eq!(result.body, "<p>Hi Rex</p>");
    }

    #[test]
    fn test_render_nested_elements() {
        let mut iso = make_isolate(&[(
            "nested",
            r#"function Page() {
                return React.createElement('div', {class: 'wrapper'},
                    React.createElement('h1', null, 'Title'),
                    React.createElement('p', null, 'Body')
                );
            }"#,
            None,
        )]);
        let result = iso.render_page("nested", "{}").unwrap();
        assert_eq!(
            result.body,
            r#"<div class="wrapper"><h1>Title</h1><p>Body</p></div>"#
        );
    }

    #[test]
    fn test_render_missing_page() {
        let mut iso = make_isolate(&[]);
        let err = iso.render_page("nonexistent", "{}").unwrap_err();
        assert!(
            err.to_string().contains("Page not found"),
            "expected 'Page not found', got: {err}"
        );
    }

    #[test]
    fn test_render_component_throws() {
        let mut iso = make_isolate(&[(
            "bad",
            "function Bad() { throw new Error('component broke'); }",
            None,
        )]);
        let err = iso.render_page("bad", "{}").unwrap_err();
        assert!(
            err.to_string().contains("component broke"),
            "expected 'component broke', got: {err}"
        );
    }

    #[test]
    fn test_gssp_sync() {
        let mut iso = make_isolate(&[(
            "page",
            "function Page(props) { return React.createElement('span', null, props.title); }",
            Some("function(ctx) { return { props: { title: 'from gssp' } }; }"),
        )]);
        let json = iso
            .get_server_side_props("page", r#"{"params":{},"query":{}}"#)
            .unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["props"]["title"], "from gssp");
    }

    #[test]
    fn test_gssp_no_gssp_returns_empty_props() {
        let mut iso = make_isolate(&[(
            "page",
            "function Page() { return React.createElement('div', null, 'hi'); }",
            None,
        )]);
        let json = iso
            .get_server_side_props("page", r#"{"params":{},"query":{}}"#)
            .unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["props"], serde_json::json!({}));
    }

    #[test]
    fn test_gssp_receives_context() {
        let mut iso = make_isolate(&[(
            "page",
            "function Page() { return React.createElement('div'); }",
            Some("function(ctx) { return { props: { slug: ctx.params.slug, url: ctx.resolved_url } }; }"),
        )]);
        let context = r#"{"params":{"slug":"hello"},"query":{},"resolved_url":"/blog/hello","headers":{},"cookies":{}}"#;
        let json = iso.get_server_side_props("page", context).unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["props"]["slug"], "hello");
        assert_eq!(val["props"]["url"], "/blog/hello");
    }

    #[test]
    fn test_gssp_async() {
        let mut iso = make_isolate(&[(
            "page",
            "function Page() { return React.createElement('div'); }",
            Some("function(ctx) { return Promise.resolve({ props: { async: true } }); }"),
        )]);
        let json = iso
            .get_server_side_props("page", r#"{"params":{},"query":{}}"#)
            .unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["props"]["async"], true);
    }

    #[test]
    fn test_gssp_throws() {
        let mut iso = make_isolate(&[(
            "page",
            "function Page() { return React.createElement('div'); }",
            Some("function(ctx) { throw new Error('gssp failed'); }"),
        )]);
        let err = iso
            .get_server_side_props("page", r#"{"params":{},"query":{}}"#)
            .unwrap_err();
        assert!(
            err.to_string().contains("gssp failed"),
            "expected 'gssp failed', got: {err}"
        );
    }

    #[test]
    fn test_gssp_missing_page() {
        let mut iso = make_isolate(&[]);
        let json = iso
            .get_server_side_props("nonexistent", r#"{"params":{},"query":{}}"#)
            .unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["props"], serde_json::json!({}));
    }

    #[test]
    fn test_reload_replaces_pages() {
        let mut iso = make_isolate(&[(
            "page",
            "function Page() { return React.createElement('p', null, 'v1'); }",
            None,
        )]);
        assert_eq!(iso.render_page("page", "{}").unwrap().body, "<p>v1</p>");

        let new_bundle = make_server_bundle(&[(
            "page",
            "function Page() { return React.createElement('p', null, 'v2'); }",
            None,
        )]);
        iso.reload(&new_bundle).unwrap();
        assert_eq!(iso.render_page("page", "{}").unwrap().body, "<p>v2</p>");
    }

    #[test]
    fn test_reload_adds_new_pages() {
        let mut iso = make_isolate(&[(
            "page",
            "function Page() { return React.createElement('p', null, 'original'); }",
            None,
        )]);

        let new_bundle = make_server_bundle(&[
            (
                "page",
                "function Page() { return React.createElement('p', null, 'original'); }",
                None,
            ),
            (
                "about",
                "function About() { return React.createElement('h1', null, 'About'); }",
                None,
            ),
        ]);
        iso.reload(&new_bundle).unwrap();
        assert_eq!(
            iso.render_page("about", "{}").unwrap().body,
            "<h1>About</h1>"
        );
    }

    #[test]
    fn test_invalid_server_bundle() {
        crate::init_v8();
        let result = SsrIsolate::new("this is not valid javascript {{{{", None);
        assert!(result.is_err());
    }

    #[test]
    fn test_multiple_renders_same_isolate() {
        let mut iso = make_isolate(&[(
            "page",
            "function Page(props) { return React.createElement('b', null, props.n); }",
            None,
        )]);
        for i in 0..5 {
            let result = iso.render_page("page", &format!(r#"{{"n":{i}}}"#)).unwrap();
            assert_eq!(result.body, format!("<b>{i}</b>"));
        }
    }

    #[test]
    fn test_render_with_head_elements() {
        let mut iso = make_isolate(&[(
            "seo",
            r#"function SeoPage(props) {
                var Head = globalThis.__rex_head_component;
                return React.createElement('div', null,
                    React.createElement(Head, null,
                        React.createElement('title', null, props.title),
                        React.createElement('meta', { name: 'description', content: 'A test page' })
                    ),
                    React.createElement('h1', null, props.title)
                );
            }"#,
            None,
        )]);
        let result = iso.render_page("seo", r#"{"title":"My Page"}"#).unwrap();
        assert!(
            result.body.contains("<h1>My Page</h1>"),
            "body should have h1: {}",
            result.body
        );
        assert!(
            !result.body.contains("<title>"),
            "body should NOT contain title: {}",
            result.body
        );
        assert!(
            result.head.contains("<title>My Page</title>"),
            "head should contain title: {}",
            result.head
        );
        assert!(
            result.head.contains("description"),
            "head should contain meta description: {}",
            result.head
        );
    }

    fn make_isolate_ext(pages: &[TestPage]) -> SsrIsolate {
        crate::init_v8();
        let bundle = format!("{}\n{}", MOCK_REACT_RUNTIME, make_server_bundle_ext(pages));
        SsrIsolate::new(&bundle, None).expect("failed to create isolate")
    }

    #[test]
    fn test_gsp_sync() {
        let mut iso = make_isolate_ext(&[TestPage {
            key: "page",
            component:
                "function Page(props) { return React.createElement('span', null, props.title); }",
            gssp: None,
            gsp: Some("function(ctx) { return { props: { title: 'from gsp' } }; }"),
        }]);
        let json = iso.get_static_props("page", r#"{"params":{}}"#).unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["props"]["title"], "from gsp");
    }

    #[test]
    fn test_gsp_async() {
        let mut iso = make_isolate_ext(&[TestPage {
            key: "page",
            component: "function Page() { return React.createElement('div'); }",
            gssp: None,
            gsp: Some("function(ctx) { return Promise.resolve({ props: { async: true } }); }"),
        }]);
        let json = iso.get_static_props("page", r#"{"params":{}}"#).unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["props"]["async"], true);
    }

    #[test]
    fn test_render_suspense_renders_children() {
        let mut iso = make_isolate(&[(
            "page",
            r#"function Page() {
                return React.createElement(React.Suspense, { fallback: 'Loading' },
                    React.createElement('div', null, 'Suspense child content')
                );
            }"#,
            None,
        )]);
        let result = iso.render_page("page", "{}").unwrap();
        assert!(
            result.body.contains("Suspense child content"),
            "should render children, not fallback: {}",
            result.body
        );
        assert!(
            !result.body.contains("Loading"),
            "should NOT render fallback when children render normally: {}",
            result.body
        );
    }

    #[test]
    fn test_render_suspense_fallback_on_throw() {
        let mut iso = make_isolate(&[(
            "page",
            r#"function Page() {
                function Thrower() { throw Promise.resolve(); }
                return React.createElement(React.Suspense, { fallback: 'Loading...' },
                    React.createElement(Thrower)
                );
            }"#,
            None,
        )]);
        let result = iso.render_page("page", "{}").unwrap();
        assert!(
            result.body.contains("Loading..."),
            "should render fallback when child throws a promise: {}",
            result.body
        );
    }

    #[test]
    fn test_head_reset_between_renders() {
        let mut iso = make_isolate(&[
            (
                "page1",
                r#"function Page1() {
                    var Head = globalThis.__rex_head_component;
                    return React.createElement('div', null,
                        React.createElement(Head, null, React.createElement('title', null, 'Page 1'))
                    );
                }"#,
                None,
            ),
            (
                "page2",
                r#"function Page2() {
                    return React.createElement('div', null, 'No head');
                }"#,
                None,
            ),
        ]);
        let r1 = iso.render_page("page1", "{}").unwrap();
        assert!(
            r1.head.contains("<title>Page 1</title>"),
            "page1 should have title"
        );

        let r2 = iso.render_page("page2", "{}").unwrap();
        assert_eq!(
            r2.head, "",
            "page2 should have empty head (no leak from page1)"
        );
    }

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
        let pages = &[("index", "function Index(props) { return React.createElement('div', null, JSON.stringify(props)); }", Some(gssp_code))];
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
                .any(|(level, msg)| *level == tracing::Level::WARN
                    && msg.contains("warning from ssr")),
            "console.warn should emit WARN-level event, captured: {captured:?}"
        );
        assert!(
            captured
                .iter()
                .any(|(level, msg)| *level == tracing::Level::ERROR
                    && msg.contains("error from ssr")),
            "console.error should emit ERROR-level event, captured: {captured:?}"
        );
    }
}
