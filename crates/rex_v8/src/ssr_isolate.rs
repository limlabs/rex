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
    /// App router route handler dispatch (route.ts)
    pub(crate) app_route_handler_fn: Option<v8::Global<v8::Function>>,
    /// getStaticPaths execution function
    pub(crate) gsp_paths_fn: Option<v8::Global<v8::Function>>,
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

        // Use explicit microtask policy so microtasks only run during
        // perform_microtask_checkpoint(). This prevents promise callbacks
        // from firing during Rust→JS calls (e.g., push_data in poll_tcp_sockets),
        // which would cause out-of-order execution in pg-pool's state machine.
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);

        // Log unhandled promise rejections for debugging
        isolate.set_promise_reject_callback(promise_reject_callback);

        // Support dynamic import() — resolves modules from the ESM registry
        isolate.set_host_import_module_dynamically_callback(
            crate::esm_module_registry::dynamic_import_callback,
        );

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
            app_route_handler_fn,
            gsp_paths_fn,
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

                // Install globalThis.queueMicrotask — V8 doesn't provide this
                // built-in in bare isolates. Without it, the JS polyfill falls back
                // to a synchronous `fn()` call, which breaks microtask ordering
                // with promise .then() handlers (those go through V8's internal
                // queue and are deferred until perform_microtask_checkpoint()).
                let t = v8::FunctionTemplate::new(scope, queue_microtask_callback);
                let f = t
                    .get_function(scope)
                    .ok_or_else(|| anyhow::anyhow!("Failed to create queueMicrotask"))?;
                let k = v8::String::new(scope, "queueMicrotask")
                    .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
                global.set(scope, k.into(), f.into());

                // Register fs polyfill callbacks
                crate::fs::register_fs_callbacks(scope, global)?;

                // Register TCP socket callbacks (for cloudflare:sockets polyfill)
                crate::tcp::register_tcp_callbacks(scope, global)?;

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

            // App router route handlers — only present when app/**/route.ts files exist
            let app_route_handler_fn =
                v8_get_optional_fn!(scope, global, "__rex_call_app_route_handler");

            // getStaticPaths — optional, only present when pages export getStaticPaths
            let gsp_paths_fn = v8_get_optional_fn!(scope, global, "__rex_get_static_paths");

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
                app_route_handler_fn,
                gsp_paths_fn,
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
            app_route_handler_fn,
            gsp_paths_fn,
            last_bundle: std::sync::Arc::new(server_bundle_js.to_string()),
        })
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

    /// Call __rex_call_app_route_handler(routePattern, reqJson) for app router route.ts handlers.
    /// Dispatches to the correct HTTP method export (GET, POST, etc.).
    /// Handles async handlers by pumping V8's microtask queue.
    pub fn call_app_route_handler(
        &mut self,
        route_pattern: &str,
        req_json: &str,
    ) -> Result<String> {
        let handler_fn = self
            .app_route_handler_fn
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("App route handlers not loaded"))?;

        let result_str = {
            v8::scope_with_context!(scope, &mut self.isolate, &self.context);

            let func = v8::Local::new(scope, handler_fn);
            let undef = v8::undefined(scope);
            let arg0 = v8::String::new(scope, route_pattern)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
            let arg1 = v8::String::new(scope, req_json)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;

            let result = v8_call!(scope, func, undef.into(), &[arg0.into(), arg1.into()])
                .map_err(|e| anyhow::anyhow!("App route handler error: {e}"))?;

            result.to_rust_string_lossy(scope)
        };

        if result_str == "__REX_APP_ROUTE_ASYNC__" {
            crate::fetch::run_fetch_loop(&mut self.isolate, &self.context);

            v8::scope_with_context!(scope, &mut self.isolate, &self.context);
            let resolve_result = v8_eval!(
                scope,
                "globalThis.__rex_resolve_app_route()",
                "<app-route-resolve>"
            )
            .map_err(|e| anyhow::anyhow!("App route handler error: {e}"))?;
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

        // Re-lookup app route handler dispatch (may be added/removed on reload)
        self.app_route_handler_fn =
            v8_get_optional_fn!(scope, global, "__rex_call_app_route_handler");

        // Re-lookup getStaticPaths function (may be added/removed on reload)
        self.gsp_paths_fn = v8_get_optional_fn!(scope, global, "__rex_get_static_paths");

        self.last_bundle = std::sync::Arc::new(server_bundle_js.to_string());
        debug!("SSR isolate reloaded");
        Ok(())
    }

    /// Call __rex_get_static_paths(routeKey) and return JSON.
    /// Handles async getStaticPaths functions by pumping V8's microtask queue.
    pub fn get_static_paths(&mut self, route_key: &str) -> Result<String> {
        let paths_fn = self
            .gsp_paths_fn
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("getStaticPaths runtime not loaded"))?;

        let result_str = {
            v8::scope_with_context!(scope, &mut self.isolate, &self.context);

            let func = v8::Local::new(scope, paths_fn);
            let undef = v8::undefined(scope);
            let arg0 = v8::String::new(scope, route_key)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;

            let result = v8_call!(scope, func, undef.into(), &[arg0.into()])
                .map_err(|e| anyhow::anyhow!("getStaticPaths error: {e}"))?;

            result.to_rust_string_lossy(scope)
        };

        if result_str == "__REX_GSP_PATHS_ASYNC__" {
            crate::fetch::run_fetch_loop(&mut self.isolate, &self.context);

            v8::scope_with_context!(scope, &mut self.isolate, &self.context);
            let resolve_result = v8_eval!(
                scope,
                "globalThis.__rex_resolve_static_paths()",
                "<gsp-paths-resolve>"
            )
            .map_err(|e| anyhow::anyhow!("getStaticPaths error: {e}"))?;
            Ok(resolve_result.to_rust_string_lossy(scope))
        } else {
            Ok(result_str)
        }
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
    // Merge env into existing process object (don't overwrite polyfilled methods)
    format!(
        "if(!globalThis.process){{globalThis.process={{}}}};globalThis.process.env={{{pairs}}};"
    )
}

fn format_args(scope: &mut v8::PinScope, args: &v8::FunctionCallbackArguments) -> String {
    let mut parts = Vec::new();
    for i in 0..args.length() {
        let arg = args.get(i);
        parts.push(arg.to_rust_string_lossy(scope));
    }
    parts.join(" ")
}

#[allow(unsafe_code)]
unsafe extern "C" fn promise_reject_callback(msg: v8::PromiseRejectMessage) {
    let event = msg.get_event();
    match event {
        v8::PromiseRejectEvent::PromiseRejectWithNoHandler => {
            tracing::warn!("Unhandled promise rejection (no handler attached)");
        }
        v8::PromiseRejectEvent::PromiseHandlerAddedAfterReject => {
            // Handler was added later — suppress
        }
        _ => {
            tracing::warn!("Promise reject event: {:?}", event);
        }
    }
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

/// `queueMicrotask(fn)` — enqueues a function into V8's microtask queue.
/// In bare V8 (no embedder), `queueMicrotask` is not a built-in global.
/// Without this, the JS polyfill falls back to `fn()` (synchronous), which
/// breaks ordering with promise `.then()` handlers.
fn queue_microtask_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _ret: v8::ReturnValue,
) {
    if args.length() < 1 || !args.get(0).is_function() {
        return;
    }
    let func = v8::Local::<v8::Function>::try_from(args.get(0)).expect("queueMicrotask arg");
    scope.enqueue_microtask(func);
}
