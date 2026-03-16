//! RSC entry generation for ESM module loading.
//!
//! Generates ESM entry source that registers app/ layouts and pages on
//! `globalThis`, using ESM `import` syntax instead of being fed to rolldown.
//! This replaces `rsc_entries::generate_server_entry()` for the ESM path.

use rex_core::app_route::AppScanResult;
use std::path::Path;

/// Generate ESM entry source for the RSC flight module.
///
/// This entry is compiled as a V8 ESM module. It imports layouts and pages
/// using absolute paths (resolved by the ESM module registry), registers
/// them on `globalThis`, and appends the flight runtime.
///
/// Unlike `generate_server_entry()` in `rsc_entries.rs`, this does NOT
/// import React or render APIs — those are provided by the dep IIFE
/// and wrapped as synthetic modules.
pub fn generate_rsc_esm_entry(
    app_scan: &AppScanResult,
    project_root: &Path,
    webpack_config_json: &str,
    server_actions_js: &str,
    flight_runtime_js: &str,
    metadata_runtime_js: &str,
) -> String {
    let mut entry = String::new();

    // Import React APIs from synthetic modules (provided by dep IIFE)
    entry.push_str("import { createElement } from 'react';\n");
    entry.push_str("import { renderToReadableStream } from 'react-server-dom-webpack/server';\n");
    entry.push_str("var __rex_createElement = createElement;\n");
    entry.push_str("var __rex_renderToReadableStream = renderToReadableStream;\n\n");

    // Import layouts as namespace imports to capture metadata/generateMetadata
    entry.push_str("globalThis.__rex_app_layouts = {};\n");
    for (i, route) in app_scan.routes.iter().enumerate() {
        for (j, layout) in route.layout_chain.iter().enumerate() {
            let layout_path = layout.to_string_lossy().replace('\\', "/");
            let mod_var = format!("__layout_mod_{i}_{j}");
            entry.push_str(&format!("import * as {mod_var} from '{layout_path}';\n"));
        }
    }

    // Import pages as namespace imports
    entry.push_str("\nglobalThis.__rex_app_pages = {};\n");
    for (i, route) in app_scan.routes.iter().enumerate() {
        let page_path = route.page_path.to_string_lossy().replace('\\', "/");
        let mod_var = format!("__page_mod_{i}");
        entry.push_str(&format!("import * as {mod_var} from '{page_path}';\n"));
        let pattern = &route.pattern;
        entry.push_str(&format!(
            "globalThis.__rex_app_pages[\"{pattern}\"] = {mod_var}.default;\n"
        ));
    }

    // Register layout chains per route
    entry.push_str("\nglobalThis.__rex_app_layout_chains = {};\n");
    for (i, route) in app_scan.routes.iter().enumerate() {
        let layout_vars: Vec<String> = (0..route.layout_chain.len())
            .map(|j| format!("__layout_mod_{i}_{j}.default"))
            .collect();
        let array = format!("[{}]", layout_vars.join(", "));
        let pattern = &route.pattern;
        entry.push_str(&format!(
            "globalThis.__rex_app_layout_chains[\"{pattern}\"] = {array};\n"
        ));
    }

    // Register metadata sources per route
    entry.push_str("\nglobalThis.__rex_app_metadata_sources = {};\n");
    for (i, route) in app_scan.routes.iter().enumerate() {
        let mut source_vars: Vec<String> = (0..route.layout_chain.len())
            .map(|j| format!("__layout_mod_{i}_{j}"))
            .collect();
        source_vars.push(format!("__page_mod_{i}"));
        let array = format!("[{}]", source_vars.join(", "));
        let pattern = &route.pattern;
        entry.push_str(&format!(
            "globalThis.__rex_app_metadata_sources[\"{pattern}\"] = {array};\n"
        ));
    }

    // Webpack bundler config
    entry.push_str(&format!(
        "\nglobalThis.__rex_webpack_bundler_config = {webpack_config_json};\n"
    ));

    // Server actions (if any)
    if !server_actions_js.is_empty() {
        entry.push_str("\n// --- Server Actions ---\n");
        entry.push_str(server_actions_js);
    }

    // App router route handlers
    if !app_scan.api_routes.is_empty() {
        entry.push_str("\n// --- App Route Handlers ---\n");
        entry.push_str("globalThis.__rex_app_route_handlers = {};\n");
        for (i, route) in app_scan.api_routes.iter().enumerate() {
            let handler_path = route.handler_path.to_string_lossy().replace('\\', "/");
            let pattern = &route.pattern;
            entry.push_str(&format!(
                "import * as __app_route{i} from '{handler_path}';\n"
            ));
            entry.push_str(&format!(
                "globalThis.__rex_app_route_handlers['{pattern}'] = __app_route{i};\n"
            ));
        }
    }

    // Metadata runtime
    entry.push_str("\n// --- Metadata Runtime ---\n");
    entry.push_str(metadata_runtime_js);

    // Flight runtime
    entry.push_str("\n// --- RSC Flight Runtime ---\n");
    entry.push_str(flight_runtime_js);

    let _ = project_root; // Used for logging context if needed
    entry
}

/// Generate ESM entry source for the pages router.
///
/// Imports all pages using absolute paths and registers them on
/// `globalThis.__rex_pages`. SSR runtime is appended inline.
pub fn generate_pages_esm_entry(
    page_sources: &[(String, std::path::PathBuf)],
    ssr_runtime_js: &str,
) -> String {
    let mut entry = String::new();

    // Import React from synthetic module
    entry.push_str("import { createElement } from 'react';\n");
    entry.push_str("import { renderToString } from 'react-dom/server';\n");
    entry.push_str("var __rex_createElement = createElement;\n");
    entry.push_str("var __rex_renderToString = renderToString;\n\n");

    // Import server-side head runtime
    entry.push_str("import 'rex/head';\n\n");

    // Import and register pages
    entry.push_str("globalThis.__rex_pages = {};\n");
    for (i, (module_name, abs_path)) in page_sources.iter().enumerate() {
        let page_path = abs_path.to_string_lossy().replace('\\', "/");
        entry.push_str(&format!("import * as __page{i} from '{page_path}';\n"));
        entry.push_str(&format!(
            "globalThis.__rex_pages['{module_name}'] = __page{i};\n"
        ));
    }

    // SSR runtime
    entry.push_str("\n// --- SSR Runtime ---\n");
    entry.push_str(ssr_runtime_js);

    entry
}
