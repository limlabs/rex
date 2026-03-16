//! ESM module loading for [`SsrIsolate`].
//!
//! Adds `load_esm_modules()` and `invalidate_esm_module()` to SsrIsolate,
//! enabling native V8 ESM loading instead of monolithic IIFE bundles.

use anyhow::{Context, Result};

use crate::esm_module_registry::EsmModuleRegistry;
use crate::ssr_isolate::SsrIsolate;

/// Source modules to load into the ESM registry.
/// Used for both pre-bundled dep modules (rolldown ESM output)
/// and OXC-transformed user source files.
#[derive(Clone)]
pub struct EsmSourceModule {
    /// Module specifier (bare specifier like "react" or absolute path).
    pub specifier: String,
    /// ESM source code.
    pub source: String,
}

impl SsrIsolate {
    /// Load modules into the V8 context using the native ESM module system.
    ///
    /// 1. Evaluate polyfills as a script (V8 globals like setTimeout, TextEncoder)
    /// 2. Compile all dep modules (pre-bundled by rolldown as ESM)
    /// 3. Compile all source modules (OXC-transformed user files)
    /// 4. Compile + instantiate + evaluate the entry module
    /// 5. Re-extract function handles from globalThis
    ///
    /// `polyfills_js` is evaluated as a script before any modules.
    /// `dep_modules` are rolldown-bundled ESM deps (react, react-dom/server, etc.).
    /// `source_modules` are OXC-transformed user files.
    /// `entry_source` is the generated entry that imports everything.
    pub fn load_esm_modules(
        &mut self,
        polyfills_js: &str,
        dep_modules: &[EsmSourceModule],
        source_modules: &[EsmSourceModule],
        entry_specifier: &str,
        entry_source: &str,
    ) -> Result<()> {
        let mut registry = EsmModuleRegistry::new();

        v8::scope_with_context!(scope, &mut self.isolate, &self.context);

        // 1. Evaluate polyfills as a script (sets V8 globals)
        if !polyfills_js.is_empty() {
            v8_eval!(scope, polyfills_js, "v8-polyfills.js")
                .context("Failed to evaluate V8 polyfills")?;
        }

        // 2. Compile dep modules (pre-bundled ESM from rolldown)
        for module in dep_modules {
            registry
                .compile_module(scope, &module.specifier, &module.source)
                .with_context(|| format!("Failed to compile dep module: {}", module.specifier))?;
        }

        // 3. Compile source modules (OXC-transformed user files)
        for module in source_modules {
            registry
                .compile_module(scope, &module.specifier, &module.source)
                .with_context(|| format!("Failed to compile module: {}", module.specifier))?;
        }

        // 4. Compile and evaluate the entry module
        registry
            .compile_module(scope, entry_specifier, entry_source)
            .with_context(|| format!("Failed to compile entry module: {entry_specifier}"))?;

        registry
            .instantiate_and_evaluate(scope, entry_specifier)
            .with_context(|| format!("Failed to evaluate entry module: {entry_specifier}"))?;

        // 5. Re-extract function handles
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
        self.gsp_paths_fn = v8_get_optional_fn!(scope, global, "__rex_get_static_paths");

        tracing::debug!("ESM modules loaded into V8 context");
        Ok(())
    }

    /// Invalidate ESM modules for HMR.
    ///
    /// V8 doesn't support re-instantiating modules, so we:
    /// 1. Clear the page registry
    /// 2. Create a fresh module registry
    /// 3. Recompile all dep + source modules + new entry
    /// 4. Instantiate + evaluate
    /// 5. Re-extract function handles
    ///
    /// Polyfills are NOT re-evaluated (they persist in globalThis).
    pub fn invalidate_esm_module(
        &mut self,
        dep_modules: &[EsmSourceModule],
        source_modules: &[EsmSourceModule],
        entry_specifier: &str,
        entry_source: &str,
    ) -> Result<()> {
        let mut registry = EsmModuleRegistry::new();

        v8::scope_with_context!(scope, &mut self.isolate, &self.context);

        // Clear page registry so the new entry can re-register
        v8_eval!(scope, "globalThis.__rex_pages = {};", "<invalidate>")?;

        // Recompile dep modules
        for module in dep_modules {
            registry
                .compile_module(scope, &module.specifier, &module.source)
                .with_context(|| format!("Failed to recompile dep module: {}", module.specifier))?;
        }

        // Recompile source modules (including the changed one)
        for module in source_modules {
            registry
                .compile_module(scope, &module.specifier, &module.source)
                .with_context(|| format!("Failed to recompile module: {}", module.specifier))?;
        }

        // Compile and evaluate new entry
        registry
            .compile_module(scope, entry_specifier, entry_source)
            .with_context(|| format!("Failed to compile entry: {entry_specifier}"))?;

        registry
            .instantiate_and_evaluate(scope, entry_specifier)
            .with_context(|| format!("Failed to evaluate entry: {entry_specifier}"))?;

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
        self.gsp_paths_fn = v8_get_optional_fn!(scope, global, "__rex_get_static_paths");

        tracing::debug!("ESM module invalidated and reloaded");
        Ok(())
    }
}
