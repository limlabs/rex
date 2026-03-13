//! ESM mode constructor and module invalidation for [`SsrIsolate`].
//!
//! Split out from `ssr_isolate.rs` to stay under the 700-line file limit.

use crate::esm_loader::EsmModuleRegistry;
use crate::ssr_isolate::{promise_reject_callback, setup_globals, IsolateMode, SsrIsolate};
use anyhow::{Context, Result};
use std::path::PathBuf;
use tracing::debug;

impl SsrIsolate {
    /// Create a new SSR isolate in ESM mode.
    ///
    /// The dep IIFE (React + polyfills) is evaluated as a script (setting globals).
    /// User code modules are compiled as ESM and evaluated via the module system.
    /// The SSR runtime JS is appended to the entry module source.
    pub fn new_esm(
        registry: EsmModuleRegistry,
        ssr_runtime: &str,
        page_sources: &[(String, PathBuf)],
        project_root: Option<&str>,
    ) -> Result<Self> {
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        isolate.set_promise_reject_callback(promise_reject_callback);

        // Phase 1: Create V8 context
        let context = {
            v8::scope!(scope, &mut isolate);
            let ctx = v8::Context::new(scope, Default::default());
            v8::Global::new(scope, ctx)
        };

        // Phase 2: Enter context, install globals, evaluate deps + user code
        let (
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
        ) = {
            v8::scope_with_context!(scope, &mut isolate, &context);

            setup_globals(scope, project_root)?;

            // Evaluate dep IIFE (React + polyfills → sets globalThis.__rex_*)
            v8_eval!(scope, registry.dep_iife(), "server-deps.js")
                .context("Failed to evaluate dep IIFE")?;

            // Build entry module source with SSR runtime appended
            let mut entry_source = registry.build_entry_source(page_sources);
            entry_source.push_str(ssr_runtime);

            // Compile and register all user modules + entry
            compile_and_evaluate_esm(scope, &registry, &entry_source)?;

            // Extract function handles from globals
            let ctx = scope.get_current_context();
            let global = ctx.global(scope);

            let render_fn = v8_get_global_fn!(scope, global, "__rex_render_page")?;
            let gssp_fn = v8_get_global_fn!(scope, global, "__rex_get_server_side_props")?;
            let gsp_fn = v8_get_global_fn!(scope, global, "__rex_get_static_props")?;

            let api_handler_fn = v8_get_optional_fn!(scope, global, "__rex_call_api_handler");
            let document_fn = v8_get_optional_fn!(scope, global, "__rex_render_document");
            let middleware_fn = v8_get_optional_fn!(scope, global, "__rex_run_middleware");
            let rsc_flight_fn = v8_get_optional_fn!(scope, global, "__rex_render_flight");
            let rsc_to_html_fn = v8_get_optional_fn!(scope, global, "__rex_render_rsc_to_html");
            let mcp_call_fn = v8_get_optional_fn!(scope, global, "__rex_call_mcp_tool");
            let mcp_list_fn = v8_get_optional_fn!(scope, global, "__rex_list_mcp_tools");
            let server_action_fn = v8_get_optional_fn!(scope, global, "__rex_call_server_action");
            let server_action_encoded_fn =
                v8_get_optional_fn!(scope, global, "__rex_call_server_action_encoded");
            let form_action_fn = v8_get_optional_fn!(scope, global, "__rex_call_form_action");
            let app_route_handler_fn =
                v8_get_optional_fn!(scope, global, "__rex_call_app_route_handler");

            (
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
            mode: IsolateMode::Esm {
                registry,
                ssr_runtime: ssr_runtime.to_string(),
            },
        })
    }

    /// Invalidate a single module in ESM mode after a file change.
    ///
    /// Re-compiles all user modules from cached transforms, re-evaluates the
    /// entry module, and re-extracts function handles. The dep IIFE is NOT
    /// re-evaluated (React + polyfills stay in the context).
    pub fn invalidate_module(
        &mut self,
        path: PathBuf,
        new_source: String,
        page_sources: &[(String, PathBuf)],
    ) -> Result<()> {
        let ssr_runtime = match &mut self.mode {
            IsolateMode::Esm {
                registry,
                ssr_runtime,
            } => {
                registry.update_source(path, new_source);
                ssr_runtime.clone()
            }
            IsolateMode::Bundled { .. } => {
                anyhow::bail!("Cannot invalidate modules in Bundled mode; use reload()");
            }
        };

        // Re-build entry and re-evaluate all modules
        v8::scope_with_context!(scope, &mut self.isolate, &self.context);

        // Clear page registry
        v8_eval!(scope, "globalThis.__rex_pages = {};", "<esm-invalidate>")?;

        let registry = match &self.mode {
            IsolateMode::Esm { registry, .. } => registry,
            _ => unreachable!(),
        };

        let mut entry_source = registry.build_entry_source(page_sources);
        entry_source.push_str(&ssr_runtime);

        compile_and_evaluate_esm(scope, registry, &entry_source)?;

        // Re-extract function handles
        let ctx = scope.get_current_context();
        let global = ctx.global(scope);

        let render_fn = v8_get_global_fn!(scope, global, "__rex_render_page")?;
        let gssp_fn = v8_get_global_fn!(scope, global, "__rex_get_server_side_props")?;
        let gsp_fn = v8_get_global_fn!(scope, global, "__rex_get_static_props")?;

        self.render_fn = v8::Global::new(scope, render_fn);
        self.gssp_fn = v8::Global::new(scope, gssp_fn);
        self.gsp_fn = v8::Global::new(scope, gsp_fn);

        self.api_handler_fn = v8_get_optional_fn!(scope, global, "__rex_call_api_handler");
        self.document_fn = v8_get_optional_fn!(scope, global, "__rex_render_document");
        self.middleware_fn = v8_get_optional_fn!(scope, global, "__rex_run_middleware");
        self.rsc_flight_fn = v8_get_optional_fn!(scope, global, "__rex_render_flight");
        self.rsc_to_html_fn = v8_get_optional_fn!(scope, global, "__rex_render_rsc_to_html");
        self.mcp_call_fn = v8_get_optional_fn!(scope, global, "__rex_call_mcp_tool");
        self.mcp_list_fn = v8_get_optional_fn!(scope, global, "__rex_list_mcp_tools");
        self.server_action_fn = v8_get_optional_fn!(scope, global, "__rex_call_server_action");
        self.server_action_encoded_fn =
            v8_get_optional_fn!(scope, global, "__rex_call_server_action_encoded");
        self.form_action_fn = v8_get_optional_fn!(scope, global, "__rex_call_form_action");
        self.app_route_handler_fn =
            v8_get_optional_fn!(scope, global, "__rex_call_app_route_handler");

        debug!("ESM module invalidated and re-evaluated");
        Ok(())
    }
}

/// Compile all user modules as scripts and evaluate via a generated entry.
///
/// Each source file has been OXC-transformed to strip TS and transform JSX.
/// We wrap each in an IIFE to provide module-like scoping, then evaluate the
/// entry source which registers pages on globalThis.
fn compile_and_evaluate_esm<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    registry: &EsmModuleRegistry,
    entry_source: &str,
) -> Result<()> {
    // Evaluate each user source file as a script wrapped in an IIFE
    for (path, source) in registry.sources() {
        let filename = path.to_string_lossy();
        let wrapped = format!("(function() {{\n{source}\n}})();\n");
        if let Err(e) = v8_eval!(scope, &wrapped, &filename) {
            tracing::warn!(file = %filename, "ESM source eval failed: {e}");
        }
    }

    // Evaluate the entry source (registers pages on globalThis)
    v8_eval!(scope, entry_source, "<esm-entry>").context("Failed to evaluate ESM entry module")?;

    Ok(())
}
