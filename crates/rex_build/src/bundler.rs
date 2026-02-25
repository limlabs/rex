use crate::entries::generate_build_id;
use crate::manifest::AssetManifest;
use anyhow::Result;
use rex_core::RexConfig;
use rex_router::ScanResult;
use rolldown_common::Output;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::info;

/// Build result containing paths to generated bundles
#[derive(Debug, Clone)]
pub struct BuildResult {
    pub build_id: String,
    pub server_bundle_path: std::path::PathBuf,
    pub manifest: AssetManifest,
}

/// Build both server and client bundles
pub async fn build_bundles(config: &RexConfig, scan: &ScanResult) -> Result<BuildResult> {
    let build_id = generate_build_id();
    let server_dir = config.server_build_dir();
    let client_dir = config.client_build_dir();

    // Clean output directories to remove stale artifacts from previous builds
    if server_dir.exists() {
        fs::remove_dir_all(&server_dir)?;
    }
    if client_dir.exists() {
        fs::remove_dir_all(&client_dir)?;
    }
    fs::create_dir_all(&server_dir)?;
    fs::create_dir_all(&client_dir)?;

    // Pre-process CSS modules (generates scoped CSS + JS proxy files)
    let css_modules = process_css_modules(scan, &client_dir, &build_id)?;

    info!("Building server bundle...");
    let server_bundle_path =
        build_server_bundle(config, scan, &server_dir, &css_modules.page_overrides).await?;

    info!("Building client bundles...");
    let manifest =
        build_client_bundles(config, scan, &client_dir, &build_id, &css_modules).await?;

    // Save manifest
    manifest.save(&config.manifest_path())?;

    // Clean up CSS module temp dir
    let css_modules_dir = client_dir.join("_css_modules");
    let _ = fs::remove_dir_all(&css_modules_dir);

    Ok(BuildResult {
        build_id,
        server_bundle_path,
        manifest,
    })
}

/// V8 polyfills for bare V8 environment (React 19 needs these).
/// Injected as a rolldown banner so they run before any bundled code.
const V8_POLYFILLS: &str = r#"
if (typeof globalThis.process === 'undefined') {
    globalThis.process = { env: { NODE_ENV: 'production' } };
}
if (typeof globalThis.setTimeout === 'undefined') {
    globalThis.setTimeout = function(fn) { fn(); return 0; };
    globalThis.clearTimeout = function() {};
}
if (typeof globalThis.queueMicrotask === 'undefined') {
    globalThis.queueMicrotask = function(fn) { fn(); };
}
if (typeof globalThis.MessageChannel === 'undefined') {
    globalThis.MessageChannel = function() {
        var cb = null;
        this.port1 = {};
        this.port2 = { postMessage: function() { if (cb) cb({ data: undefined }); } };
        Object.defineProperty(this.port1, 'onmessage', {
            set: function(fn) { cb = fn; }, get: function() { return cb; }
        });
    };
}
if (typeof globalThis.TextEncoder === 'undefined') {
    globalThis.TextEncoder = function() {};
    globalThis.TextEncoder.prototype.encode = function(str) {
        var arr = []; for (var i = 0; i < str.length; i++) arr.push(str.charCodeAt(i));
        return new Uint8Array(arr);
    };
}
if (typeof globalThis.TextDecoder === 'undefined') {
    globalThis.TextDecoder = function() {};
    globalThis.TextDecoder.prototype.decode = function(buf) {
        return String.fromCharCode.apply(null, new Uint8Array(buf));
    };
}
if (typeof globalThis.performance === 'undefined') {
    globalThis.performance = { now: function() { return Date.now(); } };
}
"#;

/// SSR runtime functions appended to the virtual entry.
/// These are bundled into the IIFE by rolldown alongside React and page code.
const SSR_RUNTIME: &str = r#"
// SSR render function — returns JSON { body, head }
globalThis.__rex_render_page = function(routeKey, propsJson) {
    var page = globalThis.__rex_pages[routeKey];
    if (!page) throw new Error('Page not found in registry: ' + routeKey);
    var Component = page.default;
    if (!Component) throw new Error('Page has no default export: ' + routeKey);

    var props = JSON.parse(propsJson);
    var element = __rex_createElement(Component, props);

    if (globalThis.__rex_app && globalThis.__rex_app.default) {
        element = __rex_createElement(globalThis.__rex_app.default, {
            Component: Component, pageProps: props
        });
    }

    globalThis.__rex_head_elements = [];
    var bodyHtml = __rex_renderToString(element);

    var headHtml = '';
    for (var i = 0; i < globalThis.__rex_head_elements.length; i++) {
        headHtml += __rex_renderToString(globalThis.__rex_head_elements[i]);
    }

    return JSON.stringify({ body: bodyHtml, head: headHtml });
};

globalThis.__rex_gssp_resolved = null;
globalThis.__rex_gssp_rejected = null;

globalThis.__rex_get_server_side_props = function(routeKey, contextJson) {
    var page = globalThis.__rex_pages[routeKey];
    if (!page || !page.getServerSideProps) return JSON.stringify({ props: {} });

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
    throw new Error('getServerSideProps promise did not resolve after microtask checkpoint');
};

globalThis.__rex_call_api_handler = function(routeKey, reqJson) {
    var handlers = globalThis.__rex_api_handlers;
    if (!handlers) throw new Error('No API handlers registered');
    var handler = handlers[routeKey];
    if (!handler) throw new Error('API handler not found: ' + routeKey);
    var handlerFn = handler.default;
    if (!handlerFn) throw new Error('No default export for API route: ' + routeKey);

    var reqData = JSON.parse(reqJson);
    var res = {
        _statusCode: 200, _headers: {}, _body: '',
        status: function(code) { this._statusCode = code; return this; },
        setHeader: function(name, value) { this._headers[name.toLowerCase()] = value; return this; },
        json: function(data) { this._headers['content-type'] = 'application/json'; this._body = JSON.stringify(data); return this; },
        send: function(body) { if (typeof body === 'object' && !this._headers['content-type']) return this.json(body); this._body = typeof body === 'string' ? body : String(body); return this; },
        end: function(body) { if (body !== undefined) this._body = String(body); return this; },
        redirect: function(statusOrUrl, maybeUrl) { if (typeof statusOrUrl === 'string') { this._statusCode = 307; this._headers['location'] = statusOrUrl; } else { this._statusCode = statusOrUrl; this._headers['location'] = maybeUrl; } return this; }
    };
    var req = { method: reqData.method, url: reqData.url, headers: reqData.headers || {}, query: reqData.query || {}, body: reqData.body, cookies: reqData.cookies || {} };

    var result = handlerFn(req, res);
    if (result && typeof result.then === 'function') {
        globalThis.__rex_api_resolved = null;
        globalThis.__rex_api_rejected = null;
        result.then(function() { globalThis.__rex_api_resolved = { statusCode: res._statusCode, headers: res._headers, body: res._body }; }, function(e) { globalThis.__rex_api_rejected = e; });
        return '__REX_API_ASYNC__';
    }
    return JSON.stringify({ statusCode: res._statusCode, headers: res._headers, body: res._body });
};

globalThis.__rex_resolve_api = function() {
    if (globalThis.__rex_api_rejected) throw globalThis.__rex_api_rejected;
    if (globalThis.__rex_api_resolved !== null) return JSON.stringify(globalThis.__rex_api_resolved);
    throw new Error('API handler promise did not resolve');
};

// Detect data strategy for a page
globalThis.__rex_detect_data_strategy = function(routeKey) {
    var page = globalThis.__rex_pages[routeKey];
    if (!page) return 'none';
    if (page.getStaticProps && page.getServerSideProps) {
        throw new Error('Page "' + routeKey + '" exports both getStaticProps and getServerSideProps. Use one or the other.');
    }
    if (page.getStaticProps) return 'getStaticProps';
    if (page.getServerSideProps) return 'getServerSideProps';
    return 'none';
};

// getStaticProps execution (parallel structure to GSSP)
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
    throw new Error('getStaticProps promise did not resolve after microtask checkpoint');
};
"#;

/// Build the server bundle using rolldown.
///
/// Produces a self-contained IIFE that includes React, all pages, and SSR
/// runtime functions. Runs in bare V8 with no module loader.
async fn build_server_bundle(
    config: &RexConfig,
    scan: &ScanResult,
    output_dir: &Path,
    page_overrides: &HashMap<PathBuf, PathBuf>,
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
            let page_path = route.abs_path.to_string_lossy().replace('\\', "/");
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

    // _app
    if let Some(app) = &scan.app {
        let effective_app = page_overrides
            .get(&app.abs_path)
            .unwrap_or(&app.abs_path);
        let app_path = effective_app.to_string_lossy().replace('\\', "/");
        entry.push_str(&format!("\nimport * as __app from '{app_path}';\n"));
        entry.push_str("globalThis.__rex_app = __app;\n");
    }

    // _document (imports rex/document which sets up __rex_render_document)
    if let Some(doc) = &scan.document {
        entry.push_str("\nimport 'rex/document';\n");
        let doc_path = doc.abs_path.to_string_lossy().replace('\\', "/");
        entry.push_str(&format!("import * as __doc from '{doc_path}';\n"));
        entry.push_str("globalThis.__rex_document = __doc;\n");
    }

    // SSR runtime functions
    entry.push_str(SSR_RUNTIME);

    let entry_path = entries_dir.join("server-entry.js");
    fs::write(&entry_path, &entry)?;

    // CSS → empty module (server doesn't need CSS)
    let mut module_types = rustc_hash::FxHashMap::default();
    module_types.insert(".css".to_string(), rolldown::ModuleType::Empty);

    // Enable minification for production builds
    let minify = if !config.dev {
        Some(rolldown_common::RawMinifyOptions::Bool(true))
    } else {
        None
    };

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
        banner: Some(rolldown::AddonOutputOption::String(Some(
            V8_POLYFILLS.to_string(),
        ))),
        resolve: Some(rolldown::ResolveOptions {
            alias: Some(vec![
                (
                    "rex/head".to_string(),
                    vec![Some(
                        runtime_dir.join("head.js").to_string_lossy().to_string(),
                    )],
                ),
                (
                    "rex/link".to_string(),
                    vec![Some(
                        runtime_dir.join("link.js").to_string_lossy().to_string(),
                    )],
                ),
                (
                    "rex/router".to_string(),
                    vec![Some(
                        runtime_dir.join("router.js").to_string_lossy().to_string(),
                    )],
                ),
                (
                    "rex/document".to_string(),
                    vec![Some(
                        runtime_dir
                            .join("document.js")
                            .to_string_lossy()
                            .to_string(),
                    )],
                ),
                // Next.js compatibility shims
                (
                    "next/head".to_string(),
                    vec![Some(
                        runtime_dir.join("head.js").to_string_lossy().to_string(),
                    )],
                ),
                (
                    "next/link".to_string(),
                    vec![Some(
                        runtime_dir.join("link.js").to_string_lossy().to_string(),
                    )],
                ),
                (
                    "next/router".to_string(),
                    vec![Some(
                        runtime_dir.join("router.js").to_string_lossy().to_string(),
                    )],
                ),
                (
                    "next/document".to_string(),
                    vec![Some(
                        runtime_dir
                            .join("document.js")
                            .to_string_lossy()
                            .to_string(),
                    )],
                ),
            ]),
            extensions: Some(vec![
                ".tsx".to_string(),
                ".ts".to_string(),
                ".jsx".to_string(),
                ".js".to_string(),
            ]),
            // Ensure runtime stubs (outside project tree) can resolve 'react'
            // from the project's node_modules
            modules: Some(vec![
                config
                    .project_root
                    .join("node_modules")
                    .to_string_lossy()
                    .to_string(),
                "node_modules".to_string(),
            ]),
            ..Default::default()
        }),
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
    info!(path = %bundle_path.display(), "Server bundle written");
    Ok(bundle_path)
}

/// Build client-side bundles using rolldown.
///
/// Rolldown handles the full pipeline: parsing TSX/JSX, resolving imports from
/// node_modules (including React), transforming, and code-splitting shared
/// dependencies into separate chunks. Output is ESM.
async fn build_client_bundles(
    config: &RexConfig,
    scan: &ScanResult,
    output_dir: &Path,
    build_id: &str,
    css_modules: &CssModuleProcessing,
) -> Result<AssetManifest> {
    let mut manifest = AssetManifest::new(build_id.to_string());
    let hash = &build_id[..8];

    // Collect and copy CSS files referenced by source (rolldown doesn't bundle CSS)
    collect_css_files(config, scan, output_dir, build_id, &mut manifest)?;

    // Add CSS module files to manifest
    for css_file in &css_modules.global_css {
        manifest.global_css.push(css_file.clone());
    }
    for (pattern, css_files) in &css_modules.route_css {
        if let Some(existing) = manifest.pages.get_mut(pattern) {
            existing.css.extend(css_files.iter().cloned());
        } else {
            // Page entry will be registered below when rolldown output is processed.
            // Store CSS temporarily; we merge after rolldown processing.
        }
    }

    // Runtime files for rex/link, rex/head aliases
    let runtime_dir = runtime_client_dir()?;

    // Generate virtual entry files for rolldown
    let entries_dir = output_dir.join("_entries");
    fs::create_dir_all(&entries_dir)?;

    let mut inputs = Vec::new();

    // _app entry
    if let Some(app) = &scan.app {
        let effective_app = css_modules
            .page_overrides
            .get(&app.abs_path)
            .unwrap_or(&app.abs_path);
        let page_path = effective_app.to_string_lossy().replace('\\', "/");
        let entry_code = format!(
            "import App from '{page_path}';\nwindow.__REX_APP__ = App;\n"
        );
        let entry_path = entries_dir.join("_app.js");
        fs::write(&entry_path, &entry_code)?;
        inputs.push(rolldown::InputItem {
            name: Some("_app".to_string()),
            import: entry_path.to_string_lossy().to_string(),
        });
    }

    // Page entries
    for route in &scan.routes {
        let chunk_name = route_to_chunk_name(route);
        let effective_path = css_modules
            .page_overrides
            .get(&route.abs_path)
            .unwrap_or(&route.abs_path);
        let page_path = effective_path.to_string_lossy().replace('\\', "/");
        let entry_code = format!(
            r#"import {{ createElement }} from 'react';
import {{ hydrateRoot }} from 'react-dom/client';
import Page from '{page_path}';

window.__REX_PAGES = window.__REX_PAGES || {{}};
window.__REX_PAGES['{route_pattern}'] = {{ default: Page }};

// Expose render function for client-side navigation (used by router.js)
if (!window.__REX_RENDER__) {{
  window.__REX_RENDER__ = function(Component, props) {{
    var element;
    if (window.__REX_APP__) {{
      element = createElement(window.__REX_APP__, {{ Component: Component, pageProps: props }});
    }} else {{
      element = createElement(Component, props);
    }}
    if (window.__REX_ROOT__) {{
      window.__REX_ROOT__.render(element);
    }}
  }};
}}

if (!window.__REX_NAVIGATING__) {{
  var dataEl = document.getElementById('__REX_DATA__');
  var pageProps = dataEl ? JSON.parse(dataEl.textContent) : {{}};
  var container = document.getElementById('__rex');
  if (container) {{
    var element;
    if (window.__REX_APP__) {{
      element = createElement(window.__REX_APP__, {{ Component: Page, pageProps: pageProps }});
    }} else {{
      element = createElement(Page, pageProps);
    }}
    window.__REX_ROOT__ = hydrateRoot(container, element);
  }}
}}
"#,
            route_pattern = route.pattern,
        );
        let entry_path = entries_dir.join(format!("{chunk_name}.js"));
        fs::write(&entry_path, &entry_code)?;
        inputs.push(rolldown::InputItem {
            name: Some(chunk_name),
            import: entry_path.to_string_lossy().to_string(),
        });
    }

    // CSS imports → empty modules (rolldown removed CSS bundling support)
    let mut module_types = rustc_hash::FxHashMap::default();
    module_types.insert(".css".to_string(), rolldown::ModuleType::Empty);

    // Enable minification for production builds
    let minify = if !config.dev {
        Some(rolldown_common::RawMinifyOptions::Bool(true))
    } else {
        None
    };

    let options = rolldown::BundlerOptions {
        input: Some(inputs),
        cwd: Some(config.project_root.clone()),
        format: Some(rolldown::OutputFormat::Esm),
        dir: Some(output_dir.to_string_lossy().to_string()),
        entry_filenames: Some(format!("[name]-{hash}.js").into()),
        chunk_filenames: Some(format!("chunk-[name]-{hash}.js").into()),
        asset_filenames: Some(format!("[name]-{hash}.[ext]").into()),
        platform: Some(rolldown::Platform::Browser),
        module_types: Some(module_types),
        minify,
        resolve: Some(rolldown::ResolveOptions {
            alias: Some(vec![
                (
                    "rex/link".to_string(),
                    vec![Some(
                        runtime_dir.join("link.js").to_string_lossy().to_string(),
                    )],
                ),
                (
                    "rex/head".to_string(),
                    vec![Some(
                        runtime_dir.join("head.js").to_string_lossy().to_string(),
                    )],
                ),
                (
                    "rex/router".to_string(),
                    vec![Some(
                        runtime_dir
                            .join("use-router.js")
                            .to_string_lossy()
                            .to_string(),
                    )],
                ),
                // Next.js compatibility shims
                (
                    "next/link".to_string(),
                    vec![Some(
                        runtime_dir.join("link.js").to_string_lossy().to_string(),
                    )],
                ),
                (
                    "next/head".to_string(),
                    vec![Some(
                        runtime_dir.join("head.js").to_string_lossy().to_string(),
                    )],
                ),
                (
                    "next/router".to_string(),
                    vec![Some(
                        runtime_dir
                            .join("use-router.js")
                            .to_string_lossy()
                            .to_string(),
                    )],
                ),
            ]),
            extensions: Some(vec![
                ".tsx".to_string(),
                ".ts".to_string(),
                ".jsx".to_string(),
                ".js".to_string(),
            ]),
            // Ensure runtime stubs (outside project tree) can resolve 'react'
            // from the project's node_modules
            modules: Some(vec![
                config
                    .project_root
                    .join("node_modules")
                    .to_string_lossy()
                    .to_string(),
                "node_modules".to_string(),
            ]),
            ..Default::default()
        }),
        ..Default::default()
    };

    let mut bundler = rolldown::Bundler::new(options)
        .map_err(|e| anyhow::anyhow!("Failed to create rolldown bundler: {e}"))?;

    let output = bundler
        .write()
        .await
        .map_err(|e| anyhow::anyhow!("Rolldown bundle failed: {e:?}"))?;

    // Process rolldown output: register entry chunks in the manifest
    for item in &output.assets {
        if let Output::Chunk(chunk) = item {
            if !chunk.is_entry {
                continue;
            }
            let name = chunk.name.to_string();
            let filename = chunk.filename.to_string();

            if name == "_app" {
                manifest.app_script = Some(filename);
            } else if let Some(route) = find_route_for_chunk(&name, &scan.routes) {
                // Check if this route has CSS module files to include
                if let Some(css_files) = css_modules.route_css.get(&route.pattern) {
                    manifest.add_page_with_css(
                        &route.pattern,
                        &filename,
                        css_files,
                    );
                } else {
                    manifest.add_page(&route.pattern, &filename);
                }
            }
        }
    }

    let _ = fs::remove_dir_all(&entries_dir);

    info!(
        pages = scan.routes.len(),
        "Client bundles built with rolldown"
    );
    Ok(manifest)
}

/// Map a route to a chunk name for rolldown entry naming.
fn route_to_chunk_name(route: &rex_core::Route) -> String {
    let module_name = route.module_name();
    let cn = module_name
        .replace('/', "-")
        .replace('[', "_")
        .replace(']', "_");
    if cn.is_empty() {
        "index".to_string()
    } else {
        cn
    }
}

/// Find the route that matches a given chunk name.
fn find_route_for_chunk<'a>(
    chunk_name: &str,
    routes: &'a [rex_core::Route],
) -> Option<&'a rex_core::Route> {
    routes.iter().find(|r| route_to_chunk_name(r) == chunk_name)
}

/// Scan source files for CSS imports and copy them to the output directory.
/// Registers global CSS (from _app) and per-page CSS in the manifest.
fn collect_css_files(
    _config: &RexConfig,
    scan: &ScanResult,
    output_dir: &Path,
    build_id: &str,
    manifest: &mut AssetManifest,
) -> Result<()> {
    let hash = &build_id[..8];

    // Collect CSS from _app (global styles)
    if let Some(app) = &scan.app {
        let css_paths = extract_css_imports(&app.abs_path)?;
        for css_path in css_paths {
            if css_path.exists() {
                let stem = css_path.file_stem().unwrap_or_default().to_string_lossy();
                let filename = format!("{stem}-{hash}.css");
                fs::copy(&css_path, output_dir.join(&filename))?;
                manifest.global_css.push(filename);
            }
        }
    }

    // Collect CSS from individual pages
    for route in &scan.routes {
        let css_paths = extract_css_imports(&route.abs_path)?;
        if css_paths.is_empty() {
            continue;
        }
        let mut page_css = Vec::new();
        for css_path in css_paths {
            if css_path.exists() {
                let stem = css_path.file_stem().unwrap_or_default().to_string_lossy();
                let filename = format!("{stem}-{hash}.css");
                fs::copy(&css_path, output_dir.join(&filename))?;
                page_css.push(filename);
            }
        }
        if !page_css.is_empty() {
            let chunk_name = route_to_chunk_name(route);
            let js_filename = format!("{chunk_name}-{hash}.js");
            manifest.add_page_with_css(&route.pattern, &js_filename, &page_css);
        }
    }

    Ok(())
}

/// Parse a source file and extract CSS import paths (resolved relative to the file).
fn extract_css_imports(source_path: &Path) -> Result<Vec<PathBuf>> {
    let source = fs::read_to_string(source_path)?;
    let parent = source_path.parent().unwrap_or(Path::new("."));
    let mut css_paths = Vec::new();

    for line in source.lines() {
        let trimmed = line.trim();
        // Match: import 'path.css' or import "path.css"
        if trimmed.starts_with("import ") || trimmed.starts_with("import'")
            || trimmed.starts_with("import\"")
        {
            if let Some(path) = extract_string_literal(trimmed) {
                // Skip .module.css — handled separately by process_css_modules
                if path.ends_with(".css") && !path.ends_with(".module.css") {
                    css_paths.push(parent.join(path));
                }
            }
        }
    }

    Ok(css_paths)
}

/// Extract the string literal from an import statement.
/// E.g. `import '../styles/globals.css';` → `../styles/globals.css`
fn extract_string_literal(line: &str) -> Option<&str> {
    // Find first quote character
    let single = line.find('\'');
    let double = line.find('"');
    let (quote_char, start) = match (single, double) {
        (Some(s), Some(d)) => {
            if s < d {
                ('\'', s)
            } else {
                ('"', d)
            }
        }
        (Some(s), None) => ('\'', s),
        (None, Some(d)) => ('"', d),
        (None, None) => return None,
    };
    let rest = &line[start + 1..];
    let end = rest.find(quote_char)?;
    Some(&rest[..end])
}

/// Get the path to the client runtime files.
/// These are embedded in the source tree at runtime/client/.
fn runtime_client_dir() -> Result<PathBuf> {
    // In dev: relative to the crate source
    // The runtime files are at the workspace root under runtime/client/
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let runtime_dir = manifest_dir.join("../../runtime/client");
    if runtime_dir.exists() {
        return Ok(runtime_dir.canonicalize()?);
    }
    // Fallback: look relative to current dir
    let cwd_runtime = PathBuf::from("runtime/client");
    if cwd_runtime.exists() {
        return Ok(cwd_runtime.canonicalize()?);
    }
    anyhow::bail!("Could not find runtime/client directory")
}

/// Get the path to the server runtime files.
/// These are embedded in the source tree at runtime/server/.
fn runtime_server_dir() -> Result<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let runtime_dir = manifest_dir.join("../../runtime/server");
    if runtime_dir.exists() {
        return Ok(runtime_dir.canonicalize()?);
    }
    let cwd_runtime = PathBuf::from("runtime/server");
    if cwd_runtime.exists() {
        return Ok(cwd_runtime.canonicalize()?);
    }
    anyhow::bail!("Could not find runtime/server directory")
}

// --- CSS Modules ---

/// Result of CSS module pre-processing.
struct CssModuleProcessing {
    /// Map of original page abs_path → modified page path (with CSS module imports rewritten)
    page_overrides: HashMap<PathBuf, PathBuf>,
    /// Scoped CSS files per route pattern
    route_css: HashMap<String, Vec<String>>,
    /// Scoped CSS files from _app (global)
    global_css: Vec<String>,
}

/// Pre-process CSS modules before rolldown bundling.
///
/// For each page that imports `.module.css` files:
/// 1. Parse the CSS to extract class names
/// 2. Generate scoped class names and write scoped CSS to output
/// 3. Generate a JS proxy that exports the class name mapping
/// 4. Create a modified page source with CSS module imports rewritten to proxy imports
fn process_css_modules(
    scan: &ScanResult,
    output_dir: &Path,
    build_id: &str,
) -> Result<CssModuleProcessing> {
    let hash_prefix = &build_id[..8];
    let temp_dir = output_dir.join("_css_modules");
    fs::create_dir_all(&temp_dir)?;

    let mut page_overrides = HashMap::new();
    let mut route_css: HashMap<String, Vec<String>> = HashMap::new();
    let mut global_css = Vec::new();

    // Track processed CSS module files to avoid duplicating work
    let mut processed_css: HashMap<PathBuf, (String, HashMap<String, String>)> = HashMap::new();

    // Collect all source files to scan: (abs_path, route_pattern or None for _app)
    let mut sources: Vec<(&PathBuf, Option<&str>)> = Vec::new();
    for route in &scan.routes {
        sources.push((&route.abs_path, Some(&route.pattern)));
    }
    if let Some(app) = &scan.app {
        sources.push((&app.abs_path, None));
    }

    for (source_path, route_pattern) in &sources {
        let css_module_imports = find_css_module_imports(source_path)?;
        if css_module_imports.is_empty() {
            continue;
        }

        let source_dir = source_path.parent().unwrap_or(Path::new("."));
        let mut source_content = fs::read_to_string(source_path)?;
        let mut page_css_files = Vec::new();

        for (import_specifier, css_abs_path) in &css_module_imports {
            // Process each CSS module file (reuse if already processed)
            let (css_filename, class_map) = if let Some(cached) = processed_css.get(css_abs_path) {
                cached.clone()
            } else {
                let css_content = fs::read_to_string(css_abs_path)?;
                let classes = parse_css_classes(&css_content);
                let file_hash = css_module_hash(css_abs_path);
                let stem = css_module_stem(css_abs_path);

                let mut class_map = HashMap::new();
                for class in &classes {
                    let scoped = format!("{stem}_{class}_{file_hash}");
                    class_map.insert(class.clone(), scoped);
                }

                // Write scoped CSS to output
                let scoped_css = scope_css(&css_content, &class_map);
                let css_filename = format!("{stem}.module-{hash_prefix}.css");
                fs::write(output_dir.join(&css_filename), &scoped_css)?;

                processed_css
                    .insert(css_abs_path.clone(), (css_filename.clone(), class_map.clone()));
                (css_filename, class_map)
            };

            page_css_files.push(css_filename);

            // Generate proxy JS file
            let proxy_content = generate_css_module_proxy(&class_map);
            let proxy_name = format!(
                "{}.js",
                css_abs_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            );
            let proxy_path = temp_dir.join(&proxy_name);
            fs::write(&proxy_path, &proxy_content)?;

            // Replace the CSS module import specifier with the absolute proxy path
            let proxy_abs = proxy_path.to_string_lossy().replace('\\', "/");
            source_content = source_content.replace(import_specifier, &proxy_abs);
        }

        // Absolutize remaining relative imports so the file works from the temp dir
        source_content = absolutize_relative_imports(&source_content, source_dir);

        // Write modified page source to temp dir
        let modified_name = source_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        // Use a unique name to avoid collisions between pages in different dirs
        let unique_name = format!(
            "{}_{}",
            css_module_hash(source_path),
            modified_name
        );
        let modified_path = temp_dir.join(&unique_name);
        fs::write(&modified_path, &source_content)?;

        page_overrides.insert((*source_path).clone(), modified_path);

        // Track CSS files
        if let Some(pattern) = route_pattern {
            route_css
                .entry(pattern.to_string())
                .or_default()
                .extend(page_css_files);
        } else {
            global_css.extend(page_css_files);
        }
    }

    Ok(CssModuleProcessing {
        page_overrides,
        route_css,
        global_css,
    })
}

/// Find `.module.css` imports in a source file.
/// Returns: Vec of (import_specifier, resolved_absolute_path).
fn find_css_module_imports(source_path: &Path) -> Result<Vec<(String, PathBuf)>> {
    let source = fs::read_to_string(source_path)?;
    let parent = source_path.parent().unwrap_or(Path::new("."));
    let mut results = Vec::new();

    for line in source.lines() {
        let trimmed = line.trim();
        // Match: import X from './path.module.css'
        if trimmed.starts_with("import ") {
            if let Some(specifier) = extract_import_from_specifier(trimmed) {
                if specifier.ends_with(".module.css") {
                    let abs_path = parent.join(specifier);
                    if abs_path.exists() {
                        results.push((specifier.to_string(), abs_path));
                    }
                }
            }
        }
    }

    Ok(results)
}

/// Extract the `from` specifier from an import statement.
/// E.g. `import styles from './Button.module.css';` → `./Button.module.css`
fn extract_import_from_specifier(line: &str) -> Option<&str> {
    // Look for `from '...'` or `from "..."`
    let from_pos = line.find("from ")?;
    let after_from = &line[from_pos + 5..];
    let trimmed = after_from.trim();
    let quote_char = trimmed.chars().next()?;
    if quote_char != '\'' && quote_char != '"' {
        return None;
    }
    let inner = &trimmed[1..];
    let end = inner.find(quote_char)?;
    Some(&inner[..end])
}

/// Parse CSS source to extract class names from selectors.
fn parse_css_classes(css: &str) -> Vec<String> {
    let mut classes = Vec::new();
    let bytes = css.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        // Skip CSS comments
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            if let Some(end) = css[i + 2..].find("*/") {
                i += end + 4;
                continue;
            }
        }

        if bytes[i] == b'.' {
            let start = i + 1;
            if start < bytes.len()
                && (bytes[start].is_ascii_alphabetic() || bytes[start] == b'_')
            {
                let mut end = start;
                while end < bytes.len()
                    && (bytes[end].is_ascii_alphanumeric()
                        || bytes[end] == b'_'
                        || bytes[end] == b'-')
                {
                    end += 1;
                }
                let class = &css[start..end];
                if !classes.contains(&class.to_string()) {
                    classes.push(class.to_string());
                }
                i = end;
                continue;
            }
        }
        i += 1;
    }

    classes
}

/// Generate a short hash for CSS module scoping based on the file path.
fn css_module_hash(file_path: &Path) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(file_path.to_string_lossy().as_bytes());
    hex::encode(&hasher.finalize()[..4])
}

/// Extract the stem from a CSS module filename (e.g., `Button.module.css` → `Button`).
fn css_module_stem(file_path: &Path) -> String {
    file_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("module")
        .trim_end_matches(".module")
        .to_string()
}

/// Rewrite CSS with scoped class names.
fn scope_css(css: &str, class_map: &HashMap<String, String>) -> String {
    let mut result = css.to_string();
    // Sort by length descending to avoid partial replacements (e.g., `.btn` before `.btn-primary`)
    let mut entries: Vec<_> = class_map.iter().collect();
    entries.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    for (original, scoped) in entries {
        result = result.replace(&format!(".{original}"), &format!(".{scoped}"));
    }
    result
}

/// Generate JS proxy file content for a CSS module.
fn generate_css_module_proxy(class_map: &HashMap<String, String>) -> String {
    let mut entries: Vec<_> = class_map.iter().collect();
    entries.sort_by_key(|(k, _)| (*k).clone());

    let pairs: Vec<String> = entries
        .iter()
        .map(|(orig, scoped)| format!("  \"{orig}\": \"{scoped}\""))
        .collect();

    format!(
        "var __css_module = {{\n{}\n}};\nexport default __css_module;\n",
        pairs.join(",\n")
    )
}

/// Absolutize relative imports in a source file so it can be moved to a temp directory.
fn absolutize_relative_imports(source: &str, source_dir: &Path) -> String {
    let mut result = String::new();

    for line in source.lines() {
        let trimmed = line.trim();

        // Handle: import X from './relative' or import X from '../relative'
        if trimmed.starts_with("import ") || trimmed.starts_with("export ") {
            if let Some(from_pos) = trimmed.find("from ") {
                let after_from = &trimmed[from_pos + 5..];
                let after_from_trimmed = after_from.trim();
                if let Some(quote_char) = after_from_trimmed.chars().next() {
                    if (quote_char == '\'' || quote_char == '"')
                        && after_from_trimmed.len() > 1
                    {
                        let inner = &after_from_trimmed[1..];
                        if let Some(end) = inner.find(quote_char) {
                            let specifier = &inner[..end];
                            if specifier.starts_with("./") || specifier.starts_with("../") {
                                let abs = source_dir.join(specifier);
                                let abs_str = abs.to_string_lossy().replace('\\', "/");
                                let new_line = format!(
                                    "{}{}{}{}{}",
                                    &trimmed[..from_pos + 5],
                                    quote_char,
                                    abs_str,
                                    quote_char,
                                    &inner[end + 1..]
                                );
                                result.push_str(&new_line);
                                result.push('\n');
                                continue;
                            }
                        }
                    }
                }
            }
            // Handle side-effect imports: import './foo.css'
            if trimmed.starts_with("import '") || trimmed.starts_with("import \"") {
                let quote_char = if trimmed.starts_with("import '") {
                    '\''
                } else {
                    '"'
                };
                let after_quote = &trimmed[8..]; // after `import '` or `import "`
                if let Some(end) = after_quote.find(quote_char) {
                    let specifier = &after_quote[..end];
                    if specifier.starts_with("./") || specifier.starts_with("../") {
                        let abs = source_dir.join(specifier);
                        let abs_str = abs.to_string_lossy().replace('\\', "/");
                        let new_line = format!(
                            "import {quote_char}{abs_str}{quote_char}{}",
                            &after_quote[end + 1..]
                        );
                        result.push_str(&new_line);
                        result.push('\n');
                        continue;
                    }
                }
            }
        }

        result.push_str(line);
        result.push('\n');
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use rex_core::{PageType, Route};
    use std::path::PathBuf;

    fn setup_test_project(
        pages: &[(&str, &str)],
        app_source: Option<&str>,
    ) -> (tempfile::TempDir, RexConfig, ScanResult) {
        setup_test_project_full(pages, app_source, None)
    }

    /// Create a temp project directory with page files, returning (config, scan)
    fn setup_test_project_full(
        pages: &[(&str, &str)],
        app_source: Option<&str>,
        doc_source: Option<&str>,
    ) -> (tempfile::TempDir, RexConfig, ScanResult) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();

        // Create mock node_modules so rolldown can resolve React imports
        setup_mock_node_modules(&root);

        // Create pages directory
        let pages_dir = root.join("pages");
        fs::create_dir_all(&pages_dir).unwrap();

        let mut routes = Vec::new();
        for (rel_path, source) in pages {
            let abs = pages_dir.join(rel_path);
            if let Some(parent) = abs.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&abs, source).unwrap();

            let file_path = PathBuf::from(rel_path);
            let module_name = file_path
                .with_extension("")
                .to_string_lossy()
                .replace('\\', "/");
            let pattern = if module_name == "index" {
                "/".to_string()
            } else {
                format!("/{}", module_name.replace("[slug]", ":slug"))
            };

            routes.push(Route {
                pattern,
                file_path,
                abs_path: abs,
                dynamic_segments: vec![],
                page_type: PageType::Regular,
                specificity: 10,
            });
        }

        let app = app_source.map(|src| {
            let abs = pages_dir.join("_app.tsx");
            fs::write(&abs, src).unwrap();
            Route {
                pattern: String::new(),
                file_path: PathBuf::from("_app.tsx"),
                abs_path: abs,
                dynamic_segments: vec![],
                page_type: PageType::App,
                specificity: 0,
            }
        });

        let document = doc_source.map(|src| {
            let abs = pages_dir.join("_document.tsx");
            fs::write(&abs, src).unwrap();
            Route {
                pattern: String::new(),
                file_path: PathBuf::from("_document.tsx"),
                abs_path: abs,
                dynamic_segments: vec![],
                page_type: PageType::Document,
                specificity: 0,
            }
        });

        let config = RexConfig::new(root).with_dev(true);
        let scan = ScanResult {
            routes,
            api_routes: vec![],
            app,
            document,
            error: None,
            not_found: None,
        };

        (tmp, config, scan)
    }

    #[tokio::test]
    async fn test_server_bundle_structure() {
        let (_tmp, config, scan) = setup_test_project(
            &[(
                "index.tsx",
                r#"
                export default function Home() {
                    return <div>Hello</div>;
                }
                "#,
            )],
            None,
        );
        let result = build_bundles(&config, &scan).await.unwrap();
        let bundle = fs::read_to_string(&result.server_bundle_path).unwrap();

        // V8 polyfills (injected as banner)
        assert!(bundle.contains("globalThis.process"), "should have process polyfill");
        assert!(bundle.contains("MessageChannel"), "should have MessageChannel polyfill");

        // Page registry
        assert!(bundle.contains("__rex_pages"), "should init page registry");

        // SSR runtime functions
        assert!(
            bundle.contains("__rex_render_page"),
            "should have render function"
        );
        assert!(
            bundle.contains("__rex_get_server_side_props"),
            "should have GSSP executor"
        );
        assert!(
            bundle.contains("__rex_resolve_gssp"),
            "should have GSSP resolver"
        );
        assert!(
            bundle.contains("__REX_ASYNC__"),
            "should have async sentinel"
        );
    }

    #[tokio::test]
    async fn test_server_bundle_iife_format() {
        let (_tmp, config, scan) = setup_test_project(
            &[(
                "index.tsx",
                r#"
                import React from 'react';
                export default function Home() {
                    return <div>Hello</div>;
                }
                export async function getServerSideProps() {
                    return { props: {} };
                }
                "#,
            )],
            None,
        );
        let result = build_bundles(&config, &scan).await.unwrap();
        let bundle = fs::read_to_string(&result.server_bundle_path).unwrap();

        // Should be IIFE format — no raw ESM import/export at top level
        assert!(
            !bundle.contains("\nimport "),
            "should not have ESM import statements"
        );
        assert!(
            !bundle.contains("\nexport "),
            "should not have ESM export statements"
        );

        // Should be self-contained (React bundled in, not externalized)
        assert!(
            bundle.contains("createElement"),
            "should contain bundled React createElement"
        );
    }

    #[tokio::test]
    async fn test_server_bundle_multiple_pages() {
        let (_tmp, config, scan) = setup_test_project(
            &[
                (
                    "index.tsx",
                    "export default function Home() { return <div>Home</div>; }",
                ),
                (
                    "about.tsx",
                    "export default function About() { return <div>About</div>; }",
                ),
                (
                    "blog/[slug].tsx",
                    "export default function Post({ slug }) { return <div>{slug}</div>; }",
                ),
            ],
            None,
        );
        let result = build_bundles(&config, &scan).await.unwrap();
        let bundle = fs::read_to_string(&result.server_bundle_path).unwrap();

        assert!(
            bundle.contains("__rex_pages[\"index\"]") || bundle.contains("__rex_pages['index']"),
            "should have index page"
        );
        assert!(
            bundle.contains("__rex_pages[\"about\"]") || bundle.contains("__rex_pages['about']"),
            "should have about page"
        );
        assert!(
            bundle.contains("__rex_pages[\"blog/[slug]\"]") || bundle.contains("__rex_pages['blog/[slug]']"),
            "should have dynamic page"
        );
    }

    #[tokio::test]
    async fn test_server_bundle_with_app() {
        let (_tmp, config, scan) = setup_test_project(
            &[(
                "index.tsx",
                "export default function Home() { return <div>Home</div>; }",
            )],
            Some(
                r#"
                export default function App({ Component, pageProps }) {
                    return <Component {...pageProps} />;
                }
                "#,
            ),
        );
        let result = build_bundles(&config, &scan).await.unwrap();
        let bundle = fs::read_to_string(&result.server_bundle_path).unwrap();

        assert!(
            bundle.contains("__rex_app"),
            "should register _app"
        );
    }

    #[tokio::test]
    async fn test_client_bundles_per_page() {
        let (_tmp, config, scan) = setup_test_project(
            &[
                (
                    "index.tsx",
                    "export default function Home() { return <div>Home</div>; }",
                ),
                (
                    "about.tsx",
                    "export default function About() { return <div>About</div>; }",
                ),
            ],
            None,
        );
        let result = build_bundles(&config, &scan).await.unwrap();
        let client_dir = config.client_build_dir();
        let build_hash = &result.build_id[..8];

        // Each page should have its own client chunk
        let index_path = client_dir.join(format!("index-{build_hash}.js"));
        let about_path = client_dir.join(format!("about-{build_hash}.js"));
        assert!(index_path.exists(), "index client chunk should exist");
        assert!(about_path.exists(), "about client chunk should exist");

        // Client chunks should have hydration bootstrap
        let index_js = fs::read_to_string(&index_path).unwrap();
        assert!(
            index_js.contains("hydrateRoot"),
            "should have hydration code"
        );
        assert!(
            index_js.contains("__REX_DATA__"),
            "should reference data element"
        );

        // Client chunks should NOT have getServerSideProps
        assert!(
            !index_js.contains("getServerSideProps"),
            "client chunk should strip GSSP"
        );
    }

    #[tokio::test]
    async fn test_manifest_contents() {
        let (_tmp, config, scan) = setup_test_project(
            &[
                (
                    "index.tsx",
                    "export default function Home() { return <div>Home</div>; }",
                ),
                (
                    "about.tsx",
                    "export default function About() { return <div>About</div>; }",
                ),
            ],
            None,
        );
        let result = build_bundles(&config, &scan).await.unwrap();

        // Manifest should track both pages
        assert!(
            result.manifest.pages.contains_key("/"),
            "manifest should have index route"
        );
        assert!(
            result.manifest.pages.contains_key("/about"),
            "manifest should have about route"
        );

        // JS filenames should include build hash
        let hash = &result.build_id[..8];
        assert!(
            result.manifest.pages["/"].js.contains(hash),
            "JS filename should include build hash"
        );

        // Manifest should be saved to disk
        let manifest_path = config.manifest_path();
        assert!(manifest_path.exists(), "manifest.json should be written");

        let loaded = AssetManifest::load(&manifest_path).unwrap();
        assert_eq!(loaded.build_id, result.build_id);
        assert_eq!(loaded.pages.len(), 2);
    }

    /// Create mock node_modules with minimal React stubs so rolldown can resolve imports.
    fn setup_mock_node_modules(root: &Path) {
        let nm = root.join("node_modules");

        // react
        let react_dir = nm.join("react");
        fs::create_dir_all(&react_dir).unwrap();
        fs::write(
            react_dir.join("package.json"),
            r#"{"name":"react","version":"19.0.0","main":"index.js"}"#,
        )
        .unwrap();
        fs::write(
            react_dir.join("index.js"),
            "export function createElement(type, props, ...children) { return { type, props, children }; }\nexport default { createElement };\n",
        )
        .unwrap();
        fs::write(
            react_dir.join("jsx-runtime.js"),
            "export function jsx(type, props) { return { type, props }; }\nexport function jsxs(type, props) { return { type, props }; }\nexport const Fragment = 'Fragment';\n",
        )
        .unwrap();
        fs::write(
            react_dir.join("jsx-dev-runtime.js"),
            "export function jsxDEV(type, props) { return { type, props }; }\nexport const Fragment = 'Fragment';\n",
        )
        .unwrap();

        // react-dom
        let react_dom_dir = nm.join("react-dom");
        fs::create_dir_all(&react_dom_dir).unwrap();
        fs::write(
            react_dom_dir.join("package.json"),
            r#"{"name":"react-dom","version":"19.0.0","main":"index.js","exports":{".":{"default":"./index.js"},"./client":{"default":"./client.js"},"./server":{"default":"./server.js"}}}"#,
        )
        .unwrap();
        fs::write(react_dom_dir.join("index.js"), "export default {};\n").unwrap();
        fs::write(
            react_dom_dir.join("client.js"),
            "export function hydrateRoot() {}\nexport function createRoot() {}\n",
        )
        .unwrap();
        fs::write(
            react_dom_dir.join("server.js"),
            "export function renderToString(el) { return ''; }\n",
        )
        .unwrap();
    }

    #[tokio::test]
    async fn test_server_bundle_with_document() {
        let (_tmp, config, scan) = setup_test_project_full(
            &[(
                "index.tsx",
                "export default function Home() { return <div>Home</div>; }",
            )],
            None,
            Some(
                r#"
                import React from 'react';
                export default function Document() {
                    return React.createElement('html', { lang: 'en' });
                }
                "#,
            ),
        );
        let result = build_bundles(&config, &scan).await.unwrap();
        let bundle = fs::read_to_string(&result.server_bundle_path).unwrap();

        assert!(
            bundle.contains("__rex_document"),
            "should register _document"
        );
    }

    #[tokio::test]
    async fn test_global_css_from_app() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();

        setup_mock_node_modules(&root);

        let pages_dir = root.join("pages");
        let styles_dir = root.join("styles");
        fs::create_dir_all(&pages_dir).unwrap();
        fs::create_dir_all(&styles_dir).unwrap();

        // Create CSS file
        fs::write(styles_dir.join("globals.css"), "body { color: red; }").unwrap();

        // Create index page
        let index_path = pages_dir.join("index.tsx");
        fs::write(
            &index_path,
            "export default function Home() { return <div>Home</div>; }",
        )
        .unwrap();

        // Create _app that imports CSS
        let app_path = pages_dir.join("_app.tsx");
        fs::write(
            &app_path,
            "import '../styles/globals.css';\nexport default function App({ Component, pageProps }) { return <Component {...pageProps} />; }",
        )
        .unwrap();

        let config = RexConfig::new(root).with_dev(true);
        let scan = ScanResult {
            routes: vec![Route {
                pattern: "/".to_string(),
                file_path: PathBuf::from("index.tsx"),
                abs_path: index_path,
                dynamic_segments: vec![],
                page_type: PageType::Regular,
                specificity: 10,
            }],
            api_routes: vec![],
            app: Some(Route {
                pattern: String::new(),
                file_path: PathBuf::from("_app.tsx"),
                abs_path: app_path,
                dynamic_segments: vec![],
                page_type: PageType::App,
                specificity: 0,
            }),
            document: None,
            error: None,
            not_found: None,
        };

        let result = build_bundles(&config, &scan).await.unwrap();

        // Manifest should have global CSS
        assert_eq!(
            result.manifest.global_css.len(),
            1,
            "should have 1 global CSS file"
        );
        assert!(
            result.manifest.global_css[0].starts_with("globals-"),
            "CSS filename should be globals-*"
        );
        assert!(
            result.manifest.global_css[0].ends_with(".css"),
            "CSS filename should end in .css"
        );

        // CSS file should exist in client output
        let client_dir = config.client_build_dir();
        let css_path = client_dir.join(&result.manifest.global_css[0]);
        assert!(css_path.exists(), "CSS file should exist in client output");
        let css_content = fs::read_to_string(&css_path).unwrap();
        assert!(
            css_content.contains("color: red"),
            "CSS file should have original content"
        );

        // Manifest should be loadable and retain global_css
        let loaded = AssetManifest::load(&config.manifest_path()).unwrap();
        assert_eq!(loaded.global_css.len(), 1);
    }

    #[tokio::test]
    async fn test_client_bundle_app_wrapping() {
        let (_tmp, config, scan) = setup_test_project(
            &[(
                "index.tsx",
                "export default function Home() { return <div>Home</div>; }",
            )],
            Some(
                r#"
                export default function App({ Component, pageProps }) {
                    return <Component {...pageProps} />;
                }
                "#,
            ),
        );
        let result = build_bundles(&config, &scan).await.unwrap();

        // _app client chunk should exist
        assert!(
            result.manifest.app_script.is_some(),
            "should have app_script in manifest"
        );
        let app_script = result.manifest.app_script.as_ref().unwrap();
        assert!(
            app_script.starts_with("_app-"),
            "app script should be named _app-*"
        );

        // Client page chunk should have _app wrapping logic
        let client_dir = config.client_build_dir();
        let index_js = fs::read_to_string(
            client_dir.join(result.manifest.pages["/"].js.clone()),
        )
        .unwrap();
        assert!(
            index_js.contains("__REX_APP__"),
            "page hydration should check for __REX_APP__"
        );
    }

    #[tokio::test]
    async fn test_next_import_shims() {
        let (_tmp, config, scan) = setup_test_project(
            &[(
                "index.tsx",
                r#"
                import Head from 'next/head';
                import Link from 'next/link';
                export default function Home() {
                    return <div><Head><title>Test</title></Head><Link href="/about">About</Link></div>;
                }
                "#,
            )],
            None,
        );

        // Should build without errors — next/* aliases resolve to rex runtime stubs
        let result = build_bundles(&config, &scan).await.unwrap();

        // Server bundle should contain the page
        let bundle = fs::read_to_string(&result.server_bundle_path).unwrap();
        assert!(
            bundle.contains("__rex_pages"),
            "server bundle should register pages"
        );

        // Client bundle should exist for the page
        assert!(
            result.manifest.pages.contains_key("/"),
            "manifest should have index page"
        );
    }

    #[tokio::test]
    async fn test_css_modules() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();

        setup_mock_node_modules(&root);

        let pages_dir = root.join("pages");
        let styles_dir = root.join("styles");
        fs::create_dir_all(&pages_dir).unwrap();
        fs::create_dir_all(&styles_dir).unwrap();

        // Create a CSS module file
        fs::write(
            styles_dir.join("Home.module.css"),
            ".container { padding: 20px; }\n.title { font-size: 24px; color: blue; }\n",
        )
        .unwrap();

        // Create a page that imports the CSS module
        let index_path = pages_dir.join("index.tsx");
        fs::write(
            &index_path,
            r#"import styles from '../styles/Home.module.css';
export default function Home() {
    return <div className={styles.container}><h1 className={styles.title}>Hello</h1></div>;
}
"#,
        )
        .unwrap();

        let config = RexConfig::new(root).with_dev(true);
        let scan = ScanResult {
            routes: vec![Route {
                pattern: "/".to_string(),
                file_path: PathBuf::from("index.tsx"),
                abs_path: index_path,
                dynamic_segments: vec![],
                page_type: PageType::Regular,
                specificity: 10,
            }],
            api_routes: vec![],
            app: None,
            document: None,
            error: None,
            not_found: None,
        };

        let result = build_bundles(&config, &scan).await.unwrap();

        // Server bundle should contain the CSS module class mapping
        let bundle = fs::read_to_string(&result.server_bundle_path).unwrap();
        assert!(
            bundle.contains("Home_container_"),
            "server bundle should contain scoped class name for container"
        );
        assert!(
            bundle.contains("Home_title_"),
            "server bundle should contain scoped class name for title"
        );

        // Scoped CSS file should exist in client output
        let client_dir = config.client_build_dir();
        let css_files: Vec<_> = fs::read_dir(&client_dir)
            .unwrap()
            .flatten()
            .filter(|e| {
                e.path()
                    .to_string_lossy()
                    .contains("Home.module-")
            })
            .collect();
        assert_eq!(css_files.len(), 1, "should have 1 scoped CSS module file");

        let scoped_css = fs::read_to_string(css_files[0].path()).unwrap();
        assert!(
            scoped_css.contains("Home_container_"),
            "scoped CSS should have rewritten class names"
        );
        assert!(
            scoped_css.contains("padding: 20px"),
            "scoped CSS should preserve property values"
        );
        assert!(
            !scoped_css.contains(".container"),
            "scoped CSS should not have original class names"
        );

        // Manifest should track CSS module file for the page
        let page_assets = result.manifest.pages.get("/").expect("should have / page");
        assert!(
            !page_assets.css.is_empty(),
            "page should have CSS assets in manifest"
        );
        assert!(
            page_assets.css[0].contains("Home.module-"),
            "CSS asset should be the scoped module file"
        );
    }

    #[test]
    fn test_parse_css_classes() {
        let css = r#"
.container { padding: 20px; }
.title { font-size: 24px; }
.btn-primary { background: blue; }
.btn-primary:hover { background: darkblue; }
/* .commented { display: none; } */
"#;
        let classes = parse_css_classes(css);
        assert!(classes.contains(&"container".to_string()));
        assert!(classes.contains(&"title".to_string()));
        assert!(classes.contains(&"btn-primary".to_string()));
    }

    #[test]
    fn test_scope_css() {
        let css = ".container { padding: 20px; }\n.title { font-size: 24px; }\n";
        let mut class_map = HashMap::new();
        class_map.insert("container".to_string(), "Home_container_abc".to_string());
        class_map.insert("title".to_string(), "Home_title_abc".to_string());

        let scoped = scope_css(css, &class_map);
        assert!(scoped.contains(".Home_container_abc"));
        assert!(scoped.contains(".Home_title_abc"));
        assert!(!scoped.contains(".container"));
        assert!(!scoped.contains(".title"));
    }

    #[test]
    fn test_generate_css_module_proxy() {
        let mut class_map = HashMap::new();
        class_map.insert("container".to_string(), "Home_container_abc".to_string());
        class_map.insert("title".to_string(), "Home_title_abc".to_string());

        let proxy = generate_css_module_proxy(&class_map);
        assert!(proxy.contains("\"container\": \"Home_container_abc\""));
        assert!(proxy.contains("\"title\": \"Home_title_abc\""));
        assert!(proxy.contains("export default"));
    }
}

