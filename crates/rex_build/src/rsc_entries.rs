//! Pure entry code generation for RSC bundles.
//!
//! Each function produces a JavaScript/TypeScript entry source string that can be
//! written to a file and fed to rolldown. No I/O, no rolldown — fully unit-testable.

use crate::client_manifest::{client_reference_id, ClientReferenceManifest};
use crate::rsc_graph::ModuleGraph;
use crate::server_action_manifest::ServerActionManifest;
use rex_core::app_route::{AppRoute, AppScanResult};
use std::path::Path;

/// Generate the RSC server flight bundle entry source.
///
/// Includes: React imports, layout/page registration on `globalThis`,
/// webpack bundler config, server action imports + dispatch table,
/// and the flight runtime.
pub(crate) fn generate_server_entry(
    app_scan: &AppScanResult,
    client_manifest: &ClientReferenceManifest,
    server_action_manifest: &ServerActionManifest,
    project_root: &Path,
) -> String {
    let mut entry = String::new();

    // React imports — resolved with react-server condition
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

    // Import pages as namespace imports to capture metadata/generateMetadata
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

    // Register layout chains per route (using .default from namespace imports)
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

    // Register metadata sources per route for the Metadata API.
    // Each route gets an array of module references (layouts + page) that may
    // export `metadata` (static object) or `generateMetadata` (async function).
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

    // Server-side webpack bundler config for renderToReadableStream
    let bundler_config_json = serde_json::to_string(&client_manifest.to_server_webpack_config())
        .unwrap_or_else(|_| "{}".to_string());
    entry.push_str(&format!(
        "\nglobalThis.__rex_webpack_bundler_config = {bundler_config_json};\n"
    ));

    // Server actions: import "use server" modules and build dispatch table
    if !server_action_manifest.actions.is_empty() {
        entry.push_str("\n// --- Server Actions Registration ---\n");
        entry.push_str(
            "import { registerServerReference, decodeReply, decodeAction } from 'react-server-dom-webpack/server';\n",
        );
        entry.push_str("globalThis.__rex_decodeReply = decodeReply;\n");
        entry.push_str("globalThis.__rex_decodeAction = decodeAction;\n");

        // Group actions by module_path to deduplicate imports
        let mut modules_by_path: std::collections::HashMap<&str, Vec<(&str, &str)>> =
            std::collections::HashMap::new();
        for (action_id, action_entry) in &server_action_manifest.actions {
            modules_by_path
                .entry(&action_entry.module_path)
                .or_default()
                .push((action_id.as_str(), action_entry.export_name.as_str()));
        }

        let project_root_str = project_root.to_string_lossy().to_string();

        entry.push_str("globalThis.__rex_server_actions = {};\n");

        for (i, (module_path, actions)) in modules_by_path.iter().enumerate() {
            let abs_path = format!("{}/{}", project_root_str.trim_end_matches('/'), module_path);
            let import_var = format!("__sa_{i}");
            entry.push_str(&format!("import * as {import_var} from '{abs_path}';\n"));

            for (action_id, export_name) in actions {
                // Register with React's server reference system
                entry.push_str(&format!(
                    "registerServerReference({import_var}.{export_name}, \"{action_id}\", \"{export_name}\");\n"
                ));
                // Build dispatch table for direct invocation
                entry.push_str(&format!(
                    "globalThis.__rex_server_actions[\"{action_id}\"] = {import_var}.{export_name};\n"
                ));
            }
        }

        // Expose dispatch table as the server action manifest for decodeReply/decodeAction
        entry.push_str(
            "globalThis.__rex_server_action_manifest = globalThis.__rex_server_actions;\n",
        );
    }

    // App router route handlers (app/**/route.ts) — registered here so the
    // RSC server bundle includes them (they need the same react-server condition
    // and polyfills). For projects with pages/, the full server bundle handles
    // these instead.
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
        let app_route_runtime = include_str!(concat!(env!("OUT_DIR"), "/app-route-runtime.js"));
        entry.push_str(app_route_runtime);
    }

    // Metadata runtime: resolveMetadata + metadataToHtml
    let metadata_runtime = include_str!("../../../runtime/server/metadata.ts");
    entry.push_str("\n// --- Metadata Runtime ---\n");
    entry.push_str(metadata_runtime);

    // RSC runtime: flight protocol using React's renderToReadableStream
    let flight_runtime = include_str!("../../../runtime/rsc/flight.ts");
    entry.push_str("\n// --- RSC Flight Runtime ---\n");
    entry.push_str(flight_runtime);

    entry
}

/// Generate the core RSC server bundle entry.
///
/// Contains: React imports (exported to globalThis), webpack config, server
/// actions, API route handlers, metadata runtime, flight runtime.
/// Does NOT include layout/page imports — those go in per-group entries.
pub fn generate_core_entry(
    app_scan: &AppScanResult,
    client_manifest: &ClientReferenceManifest,
    server_action_manifest: &ServerActionManifest,
    project_root: &Path,
) -> String {
    let mut entry = String::new();

    // React imports — resolved with react-server condition.
    // Export the React namespace to globalThis so group bundles can share it
    // via the react-group-shim alias (avoiding duplicate React instances).
    entry.push_str("import * as __react_ns from 'react';\n");
    entry.push_str("globalThis.__rex_react_ns = __react_ns;\n");
    entry.push_str("import { renderToReadableStream } from 'react-server-dom-webpack/server';\n");
    entry.push_str("var __rex_createElement = __react_ns.createElement;\n");
    entry.push_str("var __rex_renderToReadableStream = renderToReadableStream;\n\n");

    // Initialize empty registries — group entries will extend these.
    entry.push_str("globalThis.__rex_app_layouts = globalThis.__rex_app_layouts || {};\n");
    entry.push_str("globalThis.__rex_app_pages = globalThis.__rex_app_pages || {};\n");
    entry.push_str(
        "globalThis.__rex_app_layout_chains = globalThis.__rex_app_layout_chains || {};\n",
    );
    entry.push_str(
        "globalThis.__rex_app_metadata_sources = globalThis.__rex_app_metadata_sources || {};\n",
    );

    // Server-side webpack bundler config for renderToReadableStream
    let bundler_config_json = serde_json::to_string(&client_manifest.to_server_webpack_config())
        .unwrap_or_else(|_| "{}".to_string());
    entry.push_str(&format!(
        "\nglobalThis.__rex_webpack_bundler_config = {bundler_config_json};\n"
    ));

    // Server actions: import "use server" modules and build dispatch table
    if !server_action_manifest.actions.is_empty() {
        entry.push_str("\n// --- Server Actions Registration ---\n");
        entry.push_str(
            "import { registerServerReference, decodeReply, decodeAction } from 'react-server-dom-webpack/server';\n",
        );
        entry.push_str("globalThis.__rex_decodeReply = decodeReply;\n");
        entry.push_str("globalThis.__rex_decodeAction = decodeAction;\n");

        let mut modules_by_path: std::collections::HashMap<&str, Vec<(&str, &str)>> =
            std::collections::HashMap::new();
        for (action_id, action_entry) in &server_action_manifest.actions {
            modules_by_path
                .entry(&action_entry.module_path)
                .or_default()
                .push((action_id.as_str(), action_entry.export_name.as_str()));
        }

        let project_root_str = project_root.to_string_lossy().to_string();
        entry.push_str("globalThis.__rex_server_actions = {};\n");

        for (i, (module_path, actions)) in modules_by_path.iter().enumerate() {
            let abs_path = format!("{}/{}", project_root_str.trim_end_matches('/'), module_path);
            let import_var = format!("__sa_{i}");
            entry.push_str(&format!("import * as {import_var} from '{abs_path}';\n"));

            for (action_id, export_name) in actions {
                entry.push_str(&format!(
                    "registerServerReference({import_var}.{export_name}, \"{action_id}\", \"{export_name}\");\n"
                ));
                entry.push_str(&format!(
                    "globalThis.__rex_server_actions[\"{action_id}\"] = {import_var}.{export_name};\n"
                ));
            }
        }

        entry.push_str(
            "globalThis.__rex_server_action_manifest = globalThis.__rex_server_actions;\n",
        );
    }

    // App router route handlers (route.ts)
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
        let app_route_runtime = include_str!(concat!(env!("OUT_DIR"), "/app-route-runtime.js"));
        entry.push_str(app_route_runtime);
    }

    // Metadata runtime
    let metadata_runtime = include_str!("../../../runtime/server/metadata.ts");
    entry.push_str("\n// --- Metadata Runtime ---\n");
    entry.push_str(metadata_runtime);

    // RSC runtime: flight protocol
    let flight_runtime = include_str!("../../../runtime/rsc/flight.ts");
    entry.push_str("\n// --- RSC Flight Runtime ---\n");
    entry.push_str(flight_runtime);

    entry
}

/// Generate a per-route-group RSC entry that registers layouts and pages.
///
/// This produces a small entry that imports only the layouts and pages for
/// routes in a single route group. Each group IIFE runs after the core IIFE
/// and extends the shared `globalThis.__rex_app_*` registries.
///
/// React is NOT imported — group bundles use rolldown aliases to resolve
/// `react` to a shim that reads from `globalThis.__rex_react_ns` (set by
/// the core bundle).
pub fn generate_group_entry(routes: &[&AppRoute]) -> String {
    let mut entry = String::new();

    // Import layouts
    for (i, route) in routes.iter().enumerate() {
        for (j, layout) in route.layout_chain.iter().enumerate() {
            let layout_path = layout.to_string_lossy().replace('\\', "/");
            let mod_var = format!("__layout_mod_{i}_{j}");
            entry.push_str(&format!("import * as {mod_var} from '{layout_path}';\n"));
        }
    }

    // Import pages and register on globalThis
    for (i, route) in routes.iter().enumerate() {
        let page_path = route.page_path.to_string_lossy().replace('\\', "/");
        let mod_var = format!("__page_mod_{i}");
        entry.push_str(&format!("import * as {mod_var} from '{page_path}';\n"));
        let pattern = &route.pattern;
        entry.push_str(&format!(
            "globalThis.__rex_app_pages[\"{pattern}\"] = {mod_var}.default;\n"
        ));
    }

    // Register layout chains
    for (i, route) in routes.iter().enumerate() {
        let layout_vars: Vec<String> = (0..route.layout_chain.len())
            .map(|j| format!("__layout_mod_{i}_{j}.default"))
            .collect();
        let array = format!("[{}]", layout_vars.join(", "));
        let pattern = &route.pattern;
        entry.push_str(&format!(
            "globalThis.__rex_app_layout_chains[\"{pattern}\"] = {array};\n"
        ));
    }

    // Register metadata sources
    for (i, route) in routes.iter().enumerate() {
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

    entry
}

/// Generate the SSR pass bundle entry source.
///
/// Includes: React imports, all `"use client"` component imports for SSR,
/// `__rex_ssr_modules__` registration, webpack SSR manifest, and the SSR pass runtime.
pub(crate) fn generate_ssr_entry(
    graph: &ModuleGraph,
    client_manifest: &ClientReferenceManifest,
    project_root: &Path,
    build_id: &str,
) -> String {
    let mut entry = String::new();

    // React imports — standard (non-react-server) conditions
    // Use renderToReadableStream (streaming, Suspense-aware) instead of renderToString
    entry.push_str("import { createElement } from 'react';\n");
    entry.push_str("import { renderToReadableStream } from 'react-dom/server';\n");
    entry.push_str("import { createFromReadableStream } from 'react-server-dom-webpack/client';\n");
    entry.push_str("var __rex_createElement = createElement;\n");
    entry.push_str("var __rex_renderToReadableStream_ssr = renderToReadableStream;\n");
    entry.push_str("var __rex_createFromReadableStream = createFromReadableStream;\n\n");

    // Import all "use client" components for SSR rendering
    let client_boundaries = graph.client_boundary_modules();
    for (i, module) in client_boundaries.iter().enumerate() {
        let module_path = module.path.to_string_lossy().replace('\\', "/");
        entry.push_str(&format!(
            "import * as __ssr_client_{i} from '{module_path}';\n"
        ));
    }

    // Register client modules in __rex_ssr_modules__ for __webpack_require__
    entry.push_str("\nglobalThis.__rex_ssr_modules__ = globalThis.__rex_ssr_modules__ || {};\n");
    for (i, module) in client_boundaries.iter().enumerate() {
        let rel_path = module
            .path
            .strip_prefix(project_root)
            .unwrap_or(&module.path)
            .to_string_lossy()
            .replace('\\', "/");

        for export in &module.exports {
            let ref_id = client_reference_id(&rel_path, export, build_id);
            entry.push_str(&format!(
                "globalThis.__rex_ssr_modules__[\"{ref_id}\"] = __ssr_client_{i};\n"
            ));
        }
    }

    // SSR webpack manifest for createFromReadableStream
    let ssr_manifest_json = serde_json::to_string(&client_manifest.to_ssr_webpack_manifest())
        .unwrap_or_else(|_| "{}".to_string());
    entry.push_str(&format!(
        "\nglobalThis.__rex_webpack_ssr_manifest = {ssr_manifest_json};\n"
    ));

    // SSR pass runtime
    let ssr_runtime = include_str!("../../../runtime/rsc/ssr-pass.ts");
    entry.push_str("\n// --- RSC SSR Pass Runtime ---\n");
    entry.push_str(ssr_runtime);

    entry
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::client_manifest::ClientReferenceManifest;
    use crate::rsc_graph::{ModuleGraph, ModuleInfo};
    use crate::server_action_manifest::{ServerActionEntry, ServerActionManifest};
    use rex_core::app_route::{AppRoute, AppScanResult, AppSegment};
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn make_basic_app_scan() -> AppScanResult {
        let layout_path = PathBuf::from("/project/app/layout.tsx");
        let page_path = PathBuf::from("/project/app/page.tsx");
        AppScanResult {
            root: AppSegment {
                segment: String::new(),
                layout: Some(layout_path.clone()),
                page: Some(page_path.clone()),
                route: None,
                loading: None,
                error_boundary: None,
                not_found: None,
                children: vec![],
            },
            routes: vec![AppRoute {
                pattern: "/".to_string(),
                page_path: page_path.clone(),
                layout_chain: vec![layout_path.clone()],
                loading_chain: vec![None],
                error_chain: vec![None],
                dynamic_segments: vec![],
                specificity: 10,
                route_group: None,
            }],
            api_routes: vec![],
            root_layout: Some(layout_path),
        }
    }

    #[test]
    fn server_entry_contains_react_imports() {
        let scan = make_basic_app_scan();
        let manifest = ClientReferenceManifest::new();
        let sa_manifest = ServerActionManifest::new();

        let entry = generate_server_entry(&scan, &manifest, &sa_manifest, Path::new("/project"));

        assert!(entry.contains("import { createElement } from 'react'"));
        assert!(entry.contains("import { renderToReadableStream }"));
    }

    #[test]
    fn server_entry_registers_pages() {
        let scan = make_basic_app_scan();
        let manifest = ClientReferenceManifest::new();
        let sa_manifest = ServerActionManifest::new();

        let entry = generate_server_entry(&scan, &manifest, &sa_manifest, Path::new("/project"));

        assert!(entry.contains("globalThis.__rex_app_pages"));
        assert!(entry.contains("globalThis.__rex_app_pages[\"/\"]"));
    }

    #[test]
    fn server_entry_registers_layout_chains() {
        let scan = make_basic_app_scan();
        let manifest = ClientReferenceManifest::new();
        let sa_manifest = ServerActionManifest::new();

        let entry = generate_server_entry(&scan, &manifest, &sa_manifest, Path::new("/project"));

        assert!(entry.contains("globalThis.__rex_app_layout_chains"));
        assert!(entry.contains("globalThis.__rex_app_layout_chains[\"/\"]"));
    }

    #[test]
    fn server_entry_embeds_webpack_config() {
        let scan = make_basic_app_scan();
        let mut manifest = ClientReferenceManifest::new();
        manifest.add("ref1", "/Counter.js".to_string(), "default".to_string());
        let sa_manifest = ServerActionManifest::new();

        let entry = generate_server_entry(&scan, &manifest, &sa_manifest, Path::new("/project"));

        assert!(entry.contains("__rex_webpack_bundler_config"));
        assert!(entry.contains("ref1"));
    }

    #[test]
    fn server_entry_includes_flight_runtime() {
        let scan = make_basic_app_scan();
        let manifest = ClientReferenceManifest::new();
        let sa_manifest = ServerActionManifest::new();

        let entry = generate_server_entry(&scan, &manifest, &sa_manifest, Path::new("/project"));

        assert!(entry.contains("// --- RSC Flight Runtime ---"));
    }

    #[test]
    fn server_entry_includes_metadata_runtime() {
        let scan = make_basic_app_scan();
        let manifest = ClientReferenceManifest::new();
        let sa_manifest = ServerActionManifest::new();

        let entry = generate_server_entry(&scan, &manifest, &sa_manifest, Path::new("/project"));

        assert!(entry.contains("// --- Metadata Runtime ---"));
        assert!(entry.contains("metadataToHtml"));
    }

    #[test]
    fn server_entry_registers_metadata_sources() {
        let scan = make_basic_app_scan();
        let manifest = ClientReferenceManifest::new();
        let sa_manifest = ServerActionManifest::new();

        let entry = generate_server_entry(&scan, &manifest, &sa_manifest, Path::new("/project"));

        assert!(entry.contains("globalThis.__rex_app_metadata_sources"));
        assert!(entry.contains("globalThis.__rex_app_metadata_sources[\"/\"]"));
        // Should contain both the layout module and the page module
        assert!(entry.contains("__layout_mod_0_0"));
        assert!(entry.contains("__page_mod_0"));
    }

    #[test]
    fn server_entry_with_server_actions() {
        let scan = make_basic_app_scan();
        let manifest = ClientReferenceManifest::new();
        let mut sa_manifest = ServerActionManifest::new();
        sa_manifest.actions.insert(
            "action_123".to_string(),
            ServerActionEntry {
                module_path: "app/actions.ts".to_string(),
                export_name: "increment".to_string(),
            },
        );

        let entry = generate_server_entry(&scan, &manifest, &sa_manifest, Path::new("/project"));

        assert!(entry.contains("registerServerReference"));
        assert!(entry.contains("globalThis.__rex_server_actions"));
        assert!(entry.contains("action_123"));
        assert!(entry.contains("globalThis.__rex_decodeReply = decodeReply"));
        assert!(entry.contains("globalThis.__rex_decodeAction = decodeAction"));
        assert!(entry
            .contains("globalThis.__rex_server_action_manifest = globalThis.__rex_server_actions"));
    }

    #[test]
    fn server_entry_without_server_actions_omits_registration() {
        let scan = make_basic_app_scan();
        let manifest = ClientReferenceManifest::new();
        let sa_manifest = ServerActionManifest::new();

        let entry = generate_server_entry(&scan, &manifest, &sa_manifest, Path::new("/project"));

        // The registration block should not appear (the flight runtime itself
        // references __rex_server_actions for dispatch, so only check the
        // registration marker).
        assert!(!entry.contains("// --- Server Actions Registration ---"));
        assert!(!entry.contains("registerServerReference"));
    }

    #[test]
    fn server_entry_multiple_routes() {
        let layout_path = PathBuf::from("/project/app/layout.tsx");
        let page1 = PathBuf::from("/project/app/page.tsx");
        let page2 = PathBuf::from("/project/app/about/page.tsx");
        let scan = AppScanResult {
            root: AppSegment {
                segment: String::new(),
                layout: Some(layout_path.clone()),
                page: Some(page1.clone()),
                route: None,
                loading: None,
                error_boundary: None,
                not_found: None,
                children: vec![],
            },
            routes: vec![
                AppRoute {
                    pattern: "/".to_string(),
                    page_path: page1,
                    layout_chain: vec![layout_path.clone()],
                    loading_chain: vec![None],
                    error_chain: vec![None],
                    dynamic_segments: vec![],
                    specificity: 10,
                    route_group: None,
                },
                AppRoute {
                    pattern: "/about".to_string(),
                    page_path: page2,
                    layout_chain: vec![layout_path.clone()],
                    loading_chain: vec![None],
                    error_chain: vec![None],
                    dynamic_segments: vec![],
                    specificity: 10,
                    route_group: None,
                },
            ],
            api_routes: vec![],
            root_layout: Some(layout_path),
        };

        let manifest = ClientReferenceManifest::new();
        let sa_manifest = ServerActionManifest::new();
        let entry = generate_server_entry(&scan, &manifest, &sa_manifest, Path::new("/project"));

        assert!(entry.contains("globalThis.__rex_app_pages[\"/\"]"));
        assert!(entry.contains("globalThis.__rex_app_pages[\"/about\"]"));
    }

    fn make_graph_with_client_boundary() -> ModuleGraph {
        let mut modules = HashMap::new();
        modules.insert(
            PathBuf::from("/project/components/Counter.tsx"),
            ModuleInfo {
                path: PathBuf::from("/project/components/Counter.tsx"),
                is_client: true,
                is_server: false,
                uses_dynamic_functions: false,
                imports: vec![],
                exports: vec!["default".to_string()],
                server_functions: vec![],
            },
        );
        ModuleGraph { modules }
    }

    #[test]
    fn ssr_entry_contains_react_imports() {
        let graph = ModuleGraph::default();
        let manifest = ClientReferenceManifest::new();
        let entry = generate_ssr_entry(&graph, &manifest, Path::new("/project"), "build1");

        assert!(entry.contains("import { createElement } from 'react'"));
        assert!(entry.contains("import { renderToReadableStream } from 'react-dom/server'"));
        assert!(entry.contains("import { createFromReadableStream }"));
    }

    #[test]
    fn ssr_entry_imports_client_components() {
        let graph = make_graph_with_client_boundary();
        let manifest = ClientReferenceManifest::new();
        let entry = generate_ssr_entry(&graph, &manifest, Path::new("/project"), "build1");

        assert!(entry.contains("import * as __ssr_client_0"));
        assert!(entry.contains("Counter.tsx"));
    }

    #[test]
    fn ssr_entry_registers_modules() {
        let graph = make_graph_with_client_boundary();
        let manifest = ClientReferenceManifest::new();
        let entry = generate_ssr_entry(&graph, &manifest, Path::new("/project"), "build1");

        assert!(entry.contains("globalThis.__rex_ssr_modules__"));
    }

    #[test]
    fn ssr_entry_embeds_webpack_manifest() {
        let graph = ModuleGraph::default();
        let mut manifest = ClientReferenceManifest::new();
        manifest.add("ref1", "/chunk.js".to_string(), "default".to_string());
        let entry = generate_ssr_entry(&graph, &manifest, Path::new("/project"), "build1");

        assert!(entry.contains("__rex_webpack_ssr_manifest"));
        assert!(entry.contains("ref1"));
    }

    #[test]
    fn ssr_entry_includes_runtime() {
        let graph = ModuleGraph::default();
        let manifest = ClientReferenceManifest::new();
        let entry = generate_ssr_entry(&graph, &manifest, Path::new("/project"), "build1");

        assert!(entry.contains("// --- RSC SSR Pass Runtime ---"));
    }
}
