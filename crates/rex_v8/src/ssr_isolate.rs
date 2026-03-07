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
    pub(crate) isolate: v8::OwnedIsolate,
    pub(crate) context: v8::Global<v8::Context>,
    pub(crate) render_fn: v8::Global<v8::Function>,
    pub(crate) gssp_fn: v8::Global<v8::Function>,
    pub(crate) gsp_fn: v8::Global<v8::Function>,
    pub(crate) api_handler_fn: Option<v8::Global<v8::Function>>,
    pub(crate) document_fn: Option<v8::Global<v8::Function>>,
    pub(crate) middleware_fn: Option<v8::Global<v8::Function>>,
    /// RSC flight data renderer (app/ routes only)
    pub(crate) rsc_flight_fn: Option<v8::Global<v8::Function>>,
    /// RSC two-pass renderer: flight + HTML (app/ routes only)
    pub(crate) rsc_to_html_fn: Option<v8::Global<v8::Function>>,
    pub(crate) mcp_call_fn: Option<v8::Global<v8::Function>>,
    pub(crate) mcp_list_fn: Option<v8::Global<v8::Function>>,
    /// Server action dispatch function (app/ routes only)
    pub(crate) server_action_fn: Option<v8::Global<v8::Function>>,
    /// Encoded reply server action dispatch (uses decodeReply)
    pub(crate) server_action_encoded_fn: Option<v8::Global<v8::Function>>,
    /// Form action dispatch (uses decodeAction)
    pub(crate) form_action_fn: Option<v8::Global<v8::Function>>,
    /// Last successfully loaded bundle, used to restore state on failed reload.
    /// Uses `Arc<String>` to share memory across pool isolates instead of cloning.
    pub(crate) last_bundle: std::sync::Arc<String>,
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
            server_action_fn,
            server_action_encoded_fn,
            form_action_fn,
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

            // Server actions — only present when "use server" modules exist
            let server_action_fn = v8_get_optional_fn!(scope, global, "__rex_call_server_action");
            let server_action_encoded_fn =
                v8_get_optional_fn!(scope, global, "__rex_call_server_action_encoded");
            let form_action_fn = v8_get_optional_fn!(scope, global, "__rex_call_form_action");

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
                server_action_fn,
                server_action_encoded_fn,
                form_action_fn,
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
            server_action_fn,
            server_action_encoded_fn,
            form_action_fn,
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
            self.server_action_fn = v8_get_optional_fn!(scope, global, "__rex_call_server_action");
            self.server_action_encoded_fn =
                v8_get_optional_fn!(scope, global, "__rex_call_server_action_encoded");
            self.form_action_fn = v8_get_optional_fn!(scope, global, "__rex_call_form_action");
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

    /// Call __rex_call_server_action(actionId, argsJson) and return JSON response.
    /// Handles async actions by pumping V8's microtask queue + fetch loop.
    pub fn call_server_action(&mut self, action_id: &str, args_json: &str) -> Result<String> {
        let action_fn = self
            .server_action_fn
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Server actions not loaded"))?;

        let result_str = {
            v8::scope_with_context!(scope, &mut self.isolate, &self.context);

            let func = v8::Local::new(scope, action_fn);
            let undef = v8::undefined(scope);
            let arg0 = v8::String::new(scope, action_id)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
            let arg1 = v8::String::new(scope, args_json)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;

            let result = v8_call!(scope, func, undef.into(), &[arg0.into(), arg1.into()])
                .map_err(|e| anyhow::anyhow!("Server action error: {e}"))?;

            result.to_rust_string_lossy(scope)
        };

        self.pump_action_loop(&result_str)
    }

    /// Call __rex_call_server_action_encoded(actionId, body) using React's decodeReply.
    /// The body is an encoded string from the client's encodeReply.
    /// Always async since decodeReply returns a Promise.
    pub fn call_server_action_encoded(&mut self, action_id: &str, body: &str) -> Result<String> {
        let action_fn = self
            .server_action_encoded_fn
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Encoded server actions not loaded"))?;

        let result_str = {
            v8::scope_with_context!(scope, &mut self.isolate, &self.context);

            let func = v8::Local::new(scope, action_fn);
            let undef = v8::undefined(scope);
            let arg0 = v8::String::new(scope, action_id)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
            let arg1 = v8::String::new(scope, body)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;

            let result = v8_call!(scope, func, undef.into(), &[arg0.into(), arg1.into()])
                .map_err(|e| anyhow::anyhow!("Encoded server action error: {e}"))?;

            result.to_rust_string_lossy(scope)
        };

        // Always async — pump microtask queue + fetch loop
        self.pump_action_loop(&result_str)
    }

    /// Call __rex_call_form_action(fieldsJson) using React's decodeAction.
    /// fieldsJson is a JSON array of [key, value] pairs from multipart parsing.
    /// The action ID is extracted from the FormData by React's decodeAction.
    pub fn call_form_action(&mut self, _action_id: &str, fields_json: &str) -> Result<String> {
        let action_fn = self
            .form_action_fn
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Form actions not loaded"))?;

        let result_str = {
            v8::scope_with_context!(scope, &mut self.isolate, &self.context);

            let func = v8::Local::new(scope, action_fn);
            let undef = v8::undefined(scope);
            let arg0 = v8::String::new(scope, fields_json)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;

            let result = v8_call!(scope, func, undef.into(), &[arg0.into()])
                .map_err(|e| anyhow::anyhow!("Form action error: {e}"))?;

            result.to_rust_string_lossy(scope)
        };

        // Always async — pump microtask queue + fetch loop
        self.pump_action_loop(&result_str)
    }

    /// Shared async resolution loop for server action results.
    /// Pumps V8 microtasks and the fetch loop until the action resolves.
    fn pump_action_loop(&mut self, initial_result: &str) -> Result<String> {
        if initial_result == "__REX_ACTION_ASYNC__" {
            crate::fetch::run_fetch_loop(&mut self.isolate, &self.context);

            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            loop {
                if std::time::Instant::now() > deadline {
                    return Err(anyhow::anyhow!("Server action timed out after 30s"));
                }

                crate::fetch::run_fetch_loop(&mut self.isolate, &self.context);

                let status = {
                    v8::scope_with_context!(scope, &mut self.isolate, &self.context);
                    let result = v8_eval!(
                        scope,
                        "globalThis.__rex_resolve_action_pending()",
                        "<action-resolve>"
                    )
                    .map_err(|e| anyhow::anyhow!("Server action resolve error: {e}"))?;
                    result.to_rust_string_lossy(scope)
                };

                match status.as_str() {
                    "done" => break,
                    "pending" => {
                        std::thread::sleep(std::time::Duration::from_millis(1));
                        continue;
                    }
                    other => {
                        return Err(anyhow::anyhow!(
                            "Unexpected action resolve status: {}",
                            other
                        ));
                    }
                }
            }

            v8::scope_with_context!(scope, &mut self.isolate, &self.context);
            let resolve_result = v8_eval!(
                scope,
                "globalThis.__rex_finalize_action()",
                "<action-finalize>"
            )
            .map_err(|e| anyhow::anyhow!("Server action finalize error: {e}"))?;
            Ok(resolve_result.to_rust_string_lossy(scope))
        } else {
            Ok(initial_result.to_string())
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

        // Re-lookup server action dispatch (may be added/removed on reload)
        self.server_action_fn = v8_get_optional_fn!(scope, global, "__rex_call_server_action");
        self.server_action_encoded_fn =
            v8_get_optional_fn!(scope, global, "__rex_call_server_action_encoded");
        self.form_action_fn = v8_get_optional_fn!(scope, global, "__rex_call_form_action");

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
#[path = "ssr_isolate_tests.rs"]
mod tests;

