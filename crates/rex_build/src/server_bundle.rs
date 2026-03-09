use crate::build_utils::runtime_server_dir;
use anyhow::Result;
use rex_core::{ProjectConfig, RexConfig};
use rex_router::ScanResult;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::debug;

/// V8 polyfills for bare V8 environment (React 19 needs these).
/// Injected as a rolldown banner so they run before any bundled code.
/// Compiled from TypeScript at build time by build.rs.
/// Compiled V8 polyfills JS, concatenated from `runtime/server/polyfills/*.ts`.
/// Public so `rex_v8` tests can use the real polyfills instead of duplicating them.
pub const V8_POLYFILLS: &str = include_str!(concat!(env!("OUT_DIR"), "/v8-polyfills.js"));

/// SSR runtime functions appended to the virtual entry.
/// These are bundled into the IIFE by rolldown alongside React and page code.
const SSR_RUNTIME: &str = include_str!(concat!(env!("OUT_DIR"), "/ssr-runtime.js"));

/// MCP tool runtime functions appended to the virtual entry when mcp/ tools exist.
/// Defines __rex_list_mcp_tools() and __rex_call_mcp_tool(name, paramsJson).
const MCP_RUNTIME: &str = include_str!(concat!(env!("OUT_DIR"), "/mcp-runtime.js"));

/// Middleware runtime functions appended to the virtual entry when middleware.ts exists.
/// Defines __rex_run_middleware(reqJson) and __rex_resolve_middleware().
const MIDDLEWARE_RUNTIME: &str = include_str!(concat!(env!("OUT_DIR"), "/middleware-runtime.js"));

/// App route handler runtime for route.ts dispatch.
/// Defines __rex_call_app_route_handler(routePattern, reqJson) and __rex_resolve_app_route().
const APP_ROUTE_RUNTIME: &str = include_str!(concat!(env!("OUT_DIR"), "/app-route-runtime.js"));

/// Build the server bundle using rolldown.
///
/// Produces a self-contained IIFE that includes React, all pages, and SSR
/// runtime functions. Runs in bare V8 with no module loader.
pub async fn build_server_bundle(
    config: &RexConfig,
    scan: &ScanResult,
    output_dir: &Path,
    page_overrides: &HashMap<PathBuf, PathBuf>,
    define: &[(String, String)],
    project_config: &ProjectConfig,
    module_dirs: &[String],
) -> Result<PathBuf> {
    let runtime_dir = runtime_server_dir()?;

    // Generate virtual entry that imports everything and registers on globalThis
    let entries_dir = output_dir.join("_server_entry");
    fs::create_dir_all(&entries_dir)?;

    let mut entry = String::new();

    // Import React (resolved from node_modules by rolldown)
    entry.push_str("import { createElement } from 'react';\n");
    entry.push_str("import { renderToString } from 'react-dom/server';\n");
    // Make these available to runtime functions via globals
    entry.push_str("var __rex_createElement = createElement;\n");
    entry.push_str("var __rex_renderToString = renderToString;\n\n");

    // Import server-side head runtime (side effect: sets up globalThis.__rex_head_elements)
    entry.push_str("import 'rex/head';\n\n");

    // Import and register pages
    entry.push_str("globalThis.__rex_pages = {};\n");
    for (i, route) in scan.routes.iter().enumerate() {
        let effective_path = page_overrides
            .get(&route.abs_path)
            .unwrap_or(&route.abs_path);
        let page_path = effective_path.to_string_lossy().replace('\\', "/");
        let module_name = route.module_name();
        entry.push_str(&format!("import * as __page{i} from '{page_path}';\n"));
        entry.push_str(&format!(
            "globalThis.__rex_pages['{module_name}'] = __page{i};\n"
        ));
    }

    // Special pages (404, _error)
    for (label, route_opt) in [("404", &scan.not_found), ("_error", &scan.error)] {
        if let Some(route) = route_opt {
            let effective_path = page_overrides
                .get(&route.abs_path)
                .unwrap_or(&route.abs_path);
            let page_path = effective_path.to_string_lossy().replace('\\', "/");
            entry.push_str(&format!("import * as __page_{label} from '{page_path}';\n"));
            entry.push_str(&format!(
                "globalThis.__rex_pages['{label}'] = __page_{label};\n"
            ));
        }
    }

    // API routes
    if !scan.api_routes.is_empty() {
        entry.push_str("\nglobalThis.__rex_api_handlers = {};\n");
        for (i, route) in scan.api_routes.iter().enumerate() {
            let api_path = route.abs_path.to_string_lossy().replace('\\', "/");
            let module_name = route.module_name();
            entry.push_str(&format!("import * as __api{i} from '{api_path}';\n"));
            entry.push_str(&format!(
                "globalThis.__rex_api_handlers['{module_name}'] = __api{i};\n"
            ));
        }
    }

    // App router route handlers (app/**/route.ts)
    if let Some(app_scan) = &scan.app_scan {
        if !app_scan.api_routes.is_empty() {
            entry.push_str("\nglobalThis.__rex_app_route_handlers = {};\n");
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
    }

    // _app
    if let Some(app) = &scan.app {
        let effective_app = page_overrides.get(&app.abs_path).unwrap_or(&app.abs_path);
        let app_path = effective_app.to_string_lossy().replace('\\', "/");
        entry.push_str(&format!("\nimport * as __app from '{app_path}';\n"));
        entry.push_str("globalThis.__rex_app = __app;\n");
    }

    // _document (imports rex/document which sets up __rex_render_document)
    if let Some(doc) = &scan.document {
        entry.push_str("\nimport 'rex/document';\n");
        let effective_doc = page_overrides.get(&doc.abs_path).unwrap_or(&doc.abs_path);
        let doc_path = effective_doc.to_string_lossy().replace('\\', "/");
        entry.push_str(&format!("import * as __doc from '{doc_path}';\n"));
        entry.push_str("globalThis.__rex_document = __doc;\n");
    }

    // Middleware (if middleware.ts exists at project root)
    if let Some(mw_path) = &scan.middleware {
        let path = mw_path.to_string_lossy().replace('\\', "/");
        entry.push_str(&format!("\nimport * as __middleware from '{path}';\n"));
        entry.push_str("globalThis.__rex_middleware = __middleware;\n");
    }

    // MCP tools (if mcp/ directory has tool files)
    if !scan.mcp_tools.is_empty() {
        entry.push_str("\nglobalThis.__rex_mcp_tools = {};\n");
        for (i, tool) in scan.mcp_tools.iter().enumerate() {
            let tool_path = tool.abs_path.to_string_lossy().replace('\\', "/");
            let tool_name = &tool.name;
            entry.push_str(&format!("import * as __mcp{i} from '{tool_path}';\n"));
            entry.push_str(&format!(
                "globalThis.__rex_mcp_tools['{tool_name}'] = __mcp{i};\n"
            ));
        }
    }

    // SSR runtime functions
    entry.push_str(SSR_RUNTIME);

    // App route handler runtime (only when app route.ts files exist)
    if scan
        .app_scan
        .as_ref()
        .is_some_and(|a| !a.api_routes.is_empty())
    {
        entry.push_str(APP_ROUTE_RUNTIME);
    }

    // Middleware runtime (only when middleware exists)
    if scan.middleware.is_some() {
        entry.push_str(MIDDLEWARE_RUNTIME);
    }

    // MCP runtime (only when mcp/ tools exist)
    if !scan.mcp_tools.is_empty() {
        entry.push_str(MCP_RUNTIME);
    }

    let entry_path = entries_dir.join("server-entry.js");
    fs::write(&entry_path, &entry)?;

    // Non-JS assets → empty/binary modules (server doesn't need these)
    let mut module_types = rustc_hash::FxHashMap::default();
    for ext in &[".css", ".scss", ".sass", ".less", ".mdx", ".svg"] {
        module_types.insert((*ext).to_string(), rolldown::ModuleType::Empty);
    }
    for ext in &[
        ".png", ".jpg", ".jpeg", ".gif", ".webp", ".ico", ".woff", ".woff2", ".ttf", ".eot",
    ] {
        module_types.insert((*ext).to_string(), rolldown::ModuleType::Binary);
    }

    // Enable minification for production builds
    let minify = if !config.dev {
        Some(rolldown_common::RawMinifyOptions::Bool(true))
    } else {
        None
    };

    // Rex built-in aliases first, then user aliases (first match wins in rolldown)
    let mut aliases = vec![
        (
            "rex/head".to_string(),
            vec![Some(
                runtime_dir.join("head.ts").to_string_lossy().to_string(),
            )],
        ),
        (
            "rex/link".to_string(),
            vec![Some(
                runtime_dir.join("link.ts").to_string_lossy().to_string(),
            )],
        ),
        (
            "rex/router".to_string(),
            vec![Some(
                runtime_dir.join("router.ts").to_string_lossy().to_string(),
            )],
        ),
        (
            "rex/document".to_string(),
            vec![Some(
                runtime_dir
                    .join("document.ts")
                    .to_string_lossy()
                    .to_string(),
            )],
        ),
        (
            "rex/image".to_string(),
            vec![Some(
                runtime_dir.join("image.ts").to_string_lossy().to_string(),
            )],
        ),
        (
            "rex/middleware".to_string(),
            vec![Some(
                runtime_dir
                    .join("middleware.ts")
                    .to_string_lossy()
                    .to_string(),
            )],
        ),
        // Next.js compatibility shims
        (
            "next/head".to_string(),
            vec![Some(
                runtime_dir.join("head.ts").to_string_lossy().to_string(),
            )],
        ),
        (
            "next/link".to_string(),
            vec![Some(
                runtime_dir.join("link.ts").to_string_lossy().to_string(),
            )],
        ),
        (
            "next/router".to_string(),
            vec![Some(
                runtime_dir.join("router.ts").to_string_lossy().to_string(),
            )],
        ),
        (
            "next/document".to_string(),
            vec![Some(
                runtime_dir
                    .join("document.ts")
                    .to_string_lossy()
                    .to_string(),
            )],
        ),
        (
            "next/image".to_string(),
            vec![Some(
                runtime_dir.join("image.ts").to_string_lossy().to_string(),
            )],
        ),
        // Node.js fs module polyfill (server-only)
        (
            "fs".to_string(),
            vec![Some(
                runtime_dir.join("fs.ts").to_string_lossy().to_string(),
            )],
        ),
        (
            "node:fs".to_string(),
            vec![Some(
                runtime_dir.join("fs.ts").to_string_lossy().to_string(),
            )],
        ),
        (
            "fs/promises".to_string(),
            vec![Some(
                runtime_dir
                    .join("fs-promises.ts")
                    .to_string_lossy()
                    .to_string(),
            )],
        ),
        (
            "node:fs/promises".to_string(),
            vec![Some(
                runtime_dir
                    .join("fs-promises.ts")
                    .to_string_lossy()
                    .to_string(),
            )],
        ),
        // Node.js path module polyfill (server-only)
        (
            "path".to_string(),
            vec![Some(
                runtime_dir.join("path.ts").to_string_lossy().to_string(),
            )],
        ),
        (
            "node:path".to_string(),
            vec![Some(
                runtime_dir.join("path.ts").to_string_lossy().to_string(),
            )],
        ),
        // Node.js buffer module polyfill (re-exports globalThis.Buffer from banner)
        (
            "buffer".to_string(),
            vec![Some(
                runtime_dir.join("buffer.ts").to_string_lossy().to_string(),
            )],
        ),
        (
            "node:buffer".to_string(),
            vec![Some(
                runtime_dir.join("buffer.ts").to_string_lossy().to_string(),
            )],
        ),
        // Node.js crypto module polyfill (re-exports globalThis.crypto from banner)
        (
            "crypto".to_string(),
            vec![Some(
                runtime_dir.join("crypto.ts").to_string_lossy().to_string(),
            )],
        ),
        (
            "node:crypto".to_string(),
            vec![Some(
                runtime_dir.join("crypto.ts").to_string_lossy().to_string(),
            )],
        ),
        // Node.js http/https module polyfill (wraps fetch)
        (
            "http".to_string(),
            vec![Some(
                runtime_dir.join("http.ts").to_string_lossy().to_string(),
            )],
        ),
        (
            "node:http".to_string(),
            vec![Some(
                runtime_dir.join("http.ts").to_string_lossy().to_string(),
            )],
        ),
        (
            "https".to_string(),
            vec![Some(
                runtime_dir.join("https.ts").to_string_lossy().to_string(),
            )],
        ),
        (
            "node:https".to_string(),
            vec![Some(
                runtime_dir.join("https.ts").to_string_lossy().to_string(),
            )],
        ),
        // Node.js querystring module polyfill
        (
            "querystring".to_string(),
            vec![Some(
                runtime_dir
                    .join("querystring.ts")
                    .to_string_lossy()
                    .to_string(),
            )],
        ),
        (
            "node:querystring".to_string(),
            vec![Some(
                runtime_dir
                    .join("querystring.ts")
                    .to_string_lossy()
                    .to_string(),
            )],
        ),
        // Node.js events module polyfill (EventEmitter)
        (
            "events".to_string(),
            vec![Some(
                runtime_dir.join("events.ts").to_string_lossy().to_string(),
            )],
        ),
        (
            "node:events".to_string(),
            vec![Some(
                runtime_dir.join("events.ts").to_string_lossy().to_string(),
            )],
        ),
        // Node.js net module stub (empty — triggers pg-cloudflare fallback)
        (
            "net".to_string(),
            vec![Some(
                runtime_dir.join("net.ts").to_string_lossy().to_string(),
            )],
        ),
        (
            "node:net".to_string(),
            vec![Some(
                runtime_dir.join("net.ts").to_string_lossy().to_string(),
            )],
        ),
        // Node.js tls module stub
        (
            "tls".to_string(),
            vec![Some(
                runtime_dir.join("tls.ts").to_string_lossy().to_string(),
            )],
        ),
        (
            "node:tls".to_string(),
            vec![Some(
                runtime_dir.join("tls.ts").to_string_lossy().to_string(),
            )],
        ),
        // Node.js dns module stub (hostname passthrough)
        (
            "dns".to_string(),
            vec![Some(
                runtime_dir.join("dns.ts").to_string_lossy().to_string(),
            )],
        ),
        (
            "node:dns".to_string(),
            vec![Some(
                runtime_dir.join("dns.ts").to_string_lossy().to_string(),
            )],
        ),
        // Node.js os module stub
        (
            "os".to_string(),
            vec![Some(
                runtime_dir.join("os.ts").to_string_lossy().to_string(),
            )],
        ),
        (
            "node:os".to_string(),
            vec![Some(
                runtime_dir.join("os.ts").to_string_lossy().to_string(),
            )],
        ),
        // Node.js stream module polyfill (EventEmitter-based)
        (
            "stream".to_string(),
            vec![Some(
                runtime_dir.join("stream.ts").to_string_lossy().to_string(),
            )],
        ),
        (
            "node:stream".to_string(),
            vec![Some(
                runtime_dir.join("stream.ts").to_string_lossy().to_string(),
            )],
        ),
        // Node.js string_decoder module stub
        (
            "string_decoder".to_string(),
            vec![Some(
                runtime_dir
                    .join("string_decoder.ts")
                    .to_string_lossy()
                    .to_string(),
            )],
        ),
        (
            "node:string_decoder".to_string(),
            vec![Some(
                runtime_dir
                    .join("string_decoder.ts")
                    .to_string_lossy()
                    .to_string(),
            )],
        ),
        // Node.js util module stub
        (
            "util".to_string(),
            vec![Some(
                runtime_dir.join("util.ts").to_string_lossy().to_string(),
            )],
        ),
        (
            "node:util".to_string(),
            vec![Some(
                runtime_dir.join("util.ts").to_string_lossy().to_string(),
            )],
        ),
        // Node.js url module polyfill (fileURLToPath, pathToFileURL)
        (
            "url".to_string(),
            vec![Some(
                runtime_dir
                    .join("url-module.ts")
                    .to_string_lossy()
                    .to_string(),
            )],
        ),
        (
            "node:url".to_string(),
            vec![Some(
                runtime_dir
                    .join("url-module.ts")
                    .to_string_lossy()
                    .to_string(),
            )],
        ),
        // Node.js stream/web polyfill (re-exports Web Streams from globalThis)
        (
            "stream/web".to_string(),
            vec![Some(
                runtime_dir
                    .join("stream-web.ts")
                    .to_string_lossy()
                    .to_string(),
            )],
        ),
        (
            "node:stream/web".to_string(),
            vec![Some(
                runtime_dir
                    .join("stream-web.ts")
                    .to_string_lossy()
                    .to_string(),
            )],
        ),
        // Cloudflare sockets polyfill (TCP via Rust callbacks, used by pg-cloudflare)
        (
            "cloudflare:sockets".to_string(),
            vec![Some(
                runtime_dir
                    .join("cloudflare-sockets.ts")
                    .to_string_lossy()
                    .to_string(),
            )],
        ),
        // file-type stub (Node.js-only APIs not available in V8)
        (
            "file-type".to_string(),
            vec![Some(
                runtime_dir
                    .join("file-type.ts")
                    .to_string_lossy()
                    .to_string(),
            )],
        ),
    ];
    // Append user-defined aliases from rex.config build.alias
    aliases.extend(project_config.build.resolved_aliases(&config.project_root));

    let options = rolldown::BundlerOptions {
        input: Some(vec![rolldown::InputItem {
            name: Some("server-bundle".to_string()),
            import: entry_path.to_string_lossy().to_string(),
        }]),
        cwd: Some(config.project_root.clone()),
        format: Some(rolldown::OutputFormat::Iife),
        dir: Some(output_dir.to_string_lossy().to_string()),
        entry_filenames: Some("server-bundle.js".to_string().into()),
        platform: Some(rolldown::Platform::Browser),
        module_types: Some(module_types),
        minify: minify.clone(),
        define: Some(define.iter().cloned().collect()),
        banner: Some(rolldown::AddonOutputOption::String(Some(
            V8_POLYFILLS.to_string(),
        ))),
        // Disable tsconfig path resolution for the server bundle — we provide
        // explicit resolve.alias entries for rex/* stubs.  tsconfig.json `paths`
        // (e.g. "rex/*" → "@limlabs/rex/src/*") would otherwise shadow the
        // server-safe stubs with the client-only package source, causing
        // "window is not defined" at SSR time.
        tsconfig: Some(rolldown_common::TsConfig::Auto(false)),
        treeshake: crate::rsc_build_config::react_treeshake_options(),
        resolve: Some(rolldown::ResolveOptions {
            alias: Some(aliases),
            extensions: Some(vec![
                ".tsx".to_string(),
                ".ts".to_string(),
                ".jsx".to_string(),
                ".js".to_string(),
            ]),
            modules: Some(module_dirs.to_vec()),
            ..Default::default()
        }),
        sourcemap: if project_config.build.sourcemap {
            Some(rolldown::SourceMapType::File)
        } else {
            None
        },
        ..Default::default()
    };

    let mut bundler = rolldown::Bundler::new(options)
        .map_err(|e| anyhow::anyhow!("Failed to create rolldown bundler: {e}"))?;

    bundler
        .write()
        .await
        .map_err(|e| anyhow::anyhow!("Server bundle failed: {e:?}"))?;

    let _ = fs::remove_dir_all(&entries_dir);

    let bundle_path = output_dir.join("server-bundle.js");
    debug!(path = %bundle_path.display(), "Server bundle written");
    Ok(bundle_path)
}
