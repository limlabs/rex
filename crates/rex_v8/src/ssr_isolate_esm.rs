//! ESM module loading for [`SsrIsolate`].
//!
//! Adds `load_esm_modules()` and `invalidate_esm_module()` to SsrIsolate,
//! enabling native V8 ESM loading instead of monolithic IIFE bundles.

use anyhow::{Context, Result};

use crate::esm_module_registry::EsmModuleRegistry;
use crate::ssr_isolate::SsrIsolate;

/// Pre-bundled dependency IIFE content and the list of synthetic module
/// specifiers to create from the globals it sets.
pub struct DepModuleConfig {
    /// IIFE JS to evaluate as a script (sets globalThis.__rex_deps or similar).
    pub iife_js: String,
    /// Synthetic module definitions: (specifier, export_names, globals_expr).
    /// e.g., ("react", &["createElement", "useState", ...], "globalThis.__rex_deps.react")
    pub synthetic_modules: Vec<SyntheticModuleDef>,
}

/// Definition for a synthetic module wrapping dep globals.
pub struct SyntheticModuleDef {
    pub specifier: String,
    pub export_names: Vec<String>,
    pub globals_expr: String,
}

/// Source modules to load into the ESM registry.
pub struct EsmSourceModule {
    /// Canonical specifier (typically absolute path).
    pub specifier: String,
    /// OXC-transformed ESM source.
    pub source: String,
}

impl SsrIsolate {
    /// Load modules into the V8 context using the native ESM module system.
    ///
    /// 1. Evaluate dep IIFE as a script (sets globals)
    /// 2. Create synthetic modules for React, jsx-runtime, etc.
    /// 3. Compile all source modules into the ESM registry
    /// 4. Compile + instantiate + evaluate the entry module
    /// 5. Re-extract function handles from globalThis
    ///
    /// The `dep_config` provides the pre-bundled dependency IIFE and synthetic
    /// module definitions. `source_modules` are OXC-transformed user files.
    /// `entry_source` is the generated entry that imports everything.
    pub fn load_esm_modules(
        &mut self,
        dep_config: &DepModuleConfig,
        source_modules: &[EsmSourceModule],
        entry_specifier: &str,
        entry_source: &str,
    ) -> Result<()> {
        let mut registry = EsmModuleRegistry::new();

        v8::scope_with_context!(scope, &mut self.isolate, &self.context);

        // 1. Evaluate dep IIFE as a script (sets globals like __rex_deps)
        v8_eval!(scope, &dep_config.iife_js, "dep-bundle.js")
            .context("Failed to evaluate dep IIFE")?;

        // 2. Create synthetic modules for each dependency
        for def in &dep_config.synthetic_modules {
            let export_names: Vec<&str> = def.export_names.iter().map(|s| s.as_str()).collect();
            registry
                .create_synthetic_module(scope, &def.specifier, &export_names, &def.globals_expr)
                .with_context(|| format!("Failed to create synthetic module: {}", def.specifier))?;
        }

        // 3. Compile all source modules
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

    /// Invalidate a single ESM module for HMR.
    ///
    /// V8 doesn't support re-instantiating modules, so we:
    /// 1. Clear the page registry
    /// 2. Create a fresh module registry with the updated source
    /// 3. Recompile all modules + a new entry
    /// 4. Instantiate + evaluate
    /// 5. Re-extract function handles
    ///
    /// This avoids re-evaluating the dep IIFE (React stays loaded).
    pub fn invalidate_esm_module(
        &mut self,
        dep_config: &DepModuleConfig,
        source_modules: &[EsmSourceModule],
        entry_specifier: &str,
        entry_source: &str,
    ) -> Result<()> {
        let mut registry = EsmModuleRegistry::new();

        v8::scope_with_context!(scope, &mut self.isolate, &self.context);

        // Clear page registry so the new entry can re-register
        v8_eval!(scope, "globalThis.__rex_pages = {};", "<invalidate>")?;

        // Recreate synthetic modules (they reference globals that are still alive)
        for def in &dep_config.synthetic_modules {
            let export_names: Vec<&str> = def.export_names.iter().map(|s| s.as_str()).collect();
            registry
                .create_synthetic_module(scope, &def.specifier, &export_names, &def.globals_expr)
                .with_context(|| {
                    format!("Failed to recreate synthetic module: {}", def.specifier)
                })?;
        }

        // Recompile all source modules (including the changed one)
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
