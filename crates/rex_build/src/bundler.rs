use crate::entries::generate_build_id;
use crate::manifest::AssetManifest;
use crate::transform::{TransformOptions, transform_file};
use anyhow::Result;
use rex_core::RexConfig;
use rex_router::ScanResult;
use rolldown_common::Output;
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

    fs::create_dir_all(&server_dir)?;
    fs::create_dir_all(&client_dir)?;

    info!("Building server bundle...");
    let server_bundle_path = build_server_bundle(config, scan, &server_dir)?;

    info!("Building client bundles...");
    let manifest = build_client_bundles(config, scan, &client_dir, &build_id).await?;

    // Save manifest
    manifest.save(&config.manifest_path())?;

    Ok(BuildResult {
        build_id,
        server_bundle_path,
        manifest,
    })
}

/// Append a CJS module IIFE to the bundle, assigning the result to `target`.
fn append_page_iife(bundle: &mut String, comment: &str, target: &str, transformed: &str) {
    bundle.push_str(&format!("// {comment}\n"));
    bundle.push_str(&format!(
        "{target} = (function() {{\n  var exports = {{}};\n  var module = {{ exports: exports }};\n"
    ));
    bundle.push_str("  (function(exports, module, require) {\n");
    for line in transformed.lines() {
        bundle.push_str("    ");
        bundle.push_str(line);
        bundle.push('\n');
    }
    bundle.push_str("  })(exports, module, require);\n");
    bundle.push_str("  return module.exports;\n");
    bundle.push_str("})();\n\n");
}

/// Build the server bundle: transform all pages and concatenate with React SSR runtime
fn build_server_bundle(
    _config: &RexConfig,
    scan: &ScanResult,
    output_dir: &Path,
) -> Result<std::path::PathBuf> {
    let mut bundle = String::new();

    // Preamble: we'll load React/ReactDOMServer separately in V8
    bundle.push_str("// Rex Server Bundle - Auto-generated\n");
    bundle.push_str("'use strict';\n\n");

    let server_opts = TransformOptions {
        server: true,
        cjs: true,
        classic_jsx: true,
        typescript: true,
        jsx: true,
        ..Default::default()
    };

    // require() shim for CJS modules — maps known packages to V8 globals
    bundle.push_str(
        r#"var require = function(name) {
    if (name === 'react') return { default: globalThis.__React, createElement: globalThis.__React.createElement };
    if (name === 'rex/head') return { default: globalThis.__rex_head_component, __esModule: true };
    if (name === 'rex/link') return { default: globalThis.__rex_link_component, __esModule: true };
    if (name === 'rex/document') return { default: (globalThis.__rex_document_components || {}).Html, Html: (globalThis.__rex_document_components || {}).Html, Head: (globalThis.__rex_document_components || {}).Head, Main: (globalThis.__rex_document_components || {}).Main, NextScript: (globalThis.__rex_document_components || {}).NextScript, __esModule: true };
    return {};
};
"#,
    );

    // Transform and include each page as a module in a registry
    bundle.push_str("globalThis.__rex_pages = globalThis.__rex_pages || {};\n\n");

    // Transform all route pages (SWC CJS transform handles imports/exports)
    for route in &scan.routes {
        let source = fs::read_to_string(&route.abs_path)?;
        let result =
            transform_file(&source, &route.abs_path.to_string_lossy(), &server_opts)?;
        let module_name = route.module_name();
        append_page_iife(&mut bundle, &format!("Page: {module_name}"), &format!("globalThis.__rex_pages['{module_name}']"), &result.code);
    }

    // Include special pages (404, _error) in the registry so they can be SSR'd
    for (label, route_opt) in [("404", &scan.not_found), ("_error", &scan.error)] {
        if let Some(route) = route_opt {
            let source = fs::read_to_string(&route.abs_path)?;
            let result =
                transform_file(&source, &route.abs_path.to_string_lossy(), &server_opts)?;
            append_page_iife(&mut bundle, label, &format!("globalThis.__rex_pages['{label}']"), &result.code);
        }
    }

    // Transform API route handlers
    if !scan.api_routes.is_empty() {
        bundle.push_str("globalThis.__rex_api_handlers = globalThis.__rex_api_handlers || {};\n\n");
        for route in &scan.api_routes {
            let source = fs::read_to_string(&route.abs_path)?;
            let result =
                transform_file(&source, &route.abs_path.to_string_lossy(), &server_opts)?;
            let module_name = route.module_name();
            append_page_iife(
                &mut bundle,
                &format!("API: {module_name}"),
                &format!("globalThis.__rex_api_handlers['{module_name}']"),
                &result.code,
            );
        }
    }

    // Transform _app if present
    if let Some(app) = &scan.app {
        let source = fs::read_to_string(&app.abs_path)?;
        let result =
            transform_file(&source, &app.abs_path.to_string_lossy(), &server_opts)?;
        append_page_iife(&mut bundle, "_app", "globalThis.__rex_app", &result.code);
    }

    // Transform _document if present
    if let Some(doc) = &scan.document {
        let source = fs::read_to_string(&doc.abs_path)?;
        let result =
            transform_file(&source, &doc.abs_path.to_string_lossy(), &server_opts)?;
        append_page_iife(&mut bundle, "_document", "globalThis.__rex_document", &result.code);
    }

    // SSR functions
    bundle.push_str(
        r#"
// SSR render function — returns JSON { body, head }
globalThis.__rex_render_page = function(routeKey, propsJson) {
    var React = globalThis.__React;
    var ReactDOMServer = globalThis.__ReactDOMServer;
    if (!React || !ReactDOMServer) {
        throw new Error('React/ReactDOMServer not loaded. Ensure react runtime is evaluated first.');
    }

    var page = globalThis.__rex_pages[routeKey];
    if (!page) {
        throw new Error('Page not found in registry: ' + routeKey);
    }

    var Component = page.default;
    if (!Component) {
        throw new Error('Page has no default export: ' + routeKey);
    }

    var props = JSON.parse(propsJson);
    var element = React.createElement(Component, props);

    // Wrap with _app if present
    if (globalThis.__rex_app && globalThis.__rex_app.default) {
        var App = globalThis.__rex_app.default;
        element = React.createElement(App, { Component: Component, pageProps: props });
    }

    // Reset head collector, render, then collect head elements
    globalThis.__rex_head_elements = [];
    var bodyHtml = ReactDOMServer.renderToString(element);

    // Render collected head elements to HTML
    var headHtml = '';
    for (var i = 0; i < globalThis.__rex_head_elements.length; i++) {
        headHtml += ReactDOMServer.renderToString(globalThis.__rex_head_elements[i]);
    }

    return JSON.stringify({ body: bodyHtml, head: headHtml });
};

// getServerSideProps executor
globalThis.__rex_gssp_resolved = null;
globalThis.__rex_gssp_rejected = null;

globalThis.__rex_get_server_side_props = function(routeKey, contextJson) {
    var page = globalThis.__rex_pages[routeKey];
    if (!page || !page.getServerSideProps) {
        return JSON.stringify({ props: {} });
    }

    var context = JSON.parse(contextJson);
    var result = page.getServerSideProps(context);

    // Handle sync result or immediately-resolved promise
    if (result && typeof result.then === 'function') {
        globalThis.__rex_gssp_resolved = null;
        globalThis.__rex_gssp_rejected = null;
        result.then(
            function(v) { globalThis.__rex_gssp_resolved = v; },
            function(e) { globalThis.__rex_gssp_rejected = e; }
        );
        // Return sentinel — Rust will pump the microtask queue and call the resolver
        return '__REX_ASYNC__';
    }

    return JSON.stringify(result);
};

globalThis.__rex_resolve_gssp = function() {
    if (globalThis.__rex_gssp_rejected) {
        throw globalThis.__rex_gssp_rejected;
    }
    if (globalThis.__rex_gssp_resolved !== null) {
        return JSON.stringify(globalThis.__rex_gssp_resolved);
    }
    throw new Error('getServerSideProps promise did not resolve after microtask checkpoint');
};

// API route handler executor
globalThis.__rex_call_api_handler = function(routeKey, reqJson) {
    var handlers = globalThis.__rex_api_handlers;
    if (!handlers) throw new Error('No API handlers registered');
    var handler = handlers[routeKey];
    if (!handler) throw new Error('API handler not found: ' + routeKey);
    var handlerFn = handler.default;
    if (!handlerFn) throw new Error('No default export for API route: ' + routeKey);

    var reqData = JSON.parse(reqJson);
    var res = {
        _statusCode: 200,
        _headers: {},
        _body: '',
        status: function(code) { this._statusCode = code; return this; },
        setHeader: function(name, value) { this._headers[name.toLowerCase()] = value; return this; },
        json: function(data) {
            this._headers['content-type'] = 'application/json';
            this._body = JSON.stringify(data);
            return this;
        },
        send: function(body) {
            if (typeof body === 'object' && !this._headers['content-type']) {
                return this.json(body);
            }
            this._body = typeof body === 'string' ? body : String(body);
            return this;
        },
        end: function(body) {
            if (body !== undefined) this._body = String(body);
            return this;
        },
        redirect: function(statusOrUrl, maybeUrl) {
            if (typeof statusOrUrl === 'string') {
                this._statusCode = 307;
                this._headers['location'] = statusOrUrl;
            } else {
                this._statusCode = statusOrUrl;
                this._headers['location'] = maybeUrl;
            }
            return this;
        }
    };
    var req = {
        method: reqData.method,
        url: reqData.url,
        headers: reqData.headers || {},
        query: reqData.query || {},
        body: reqData.body,
        cookies: reqData.cookies || {}
    };

    var result = handlerFn(req, res);

    if (result && typeof result.then === 'function') {
        globalThis.__rex_api_resolved = null;
        globalThis.__rex_api_rejected = null;
        result.then(
            function() {
                globalThis.__rex_api_resolved = {
                    statusCode: res._statusCode,
                    headers: res._headers,
                    body: res._body
                };
            },
            function(e) { globalThis.__rex_api_rejected = e; }
        );
        return '__REX_API_ASYNC__';
    }

    return JSON.stringify({
        statusCode: res._statusCode,
        headers: res._headers,
        body: res._body
    });
};

globalThis.__rex_resolve_api = function() {
    if (globalThis.__rex_api_rejected) throw globalThis.__rex_api_rejected;
    if (globalThis.__rex_api_resolved !== null) return JSON.stringify(globalThis.__rex_api_resolved);
    throw new Error('API handler promise did not resolve');
};
"#,
    );

    let bundle_path = output_dir.join("server-bundle.js");
    fs::write(&bundle_path, &bundle)?;
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
) -> Result<AssetManifest> {
    let mut manifest = AssetManifest::new(build_id.to_string());
    let hash = &build_id[..8];

    // Collect and copy CSS files referenced by source (rolldown doesn't bundle CSS)
    collect_css_files(config, scan, output_dir, build_id, &mut manifest)?;

    // Runtime files for rex/link, rex/head aliases
    let runtime_dir = runtime_client_dir()?;

    // Generate virtual entry files for rolldown
    let entries_dir = output_dir.join("_entries");
    fs::create_dir_all(&entries_dir)?;

    let mut inputs = Vec::new();

    // _app entry
    if let Some(app) = &scan.app {
        let page_path = app.abs_path.to_string_lossy().replace('\\', "/");
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
        let page_path = route.abs_path.to_string_lossy().replace('\\', "/");
        let entry_code = format!(
            r#"import {{ createElement }} from 'react';
import {{ hydrateRoot }} from 'react-dom/client';
import Page from '{page_path}';

window.__REX_PAGES = window.__REX_PAGES || {{}};
window.__REX_PAGES['{route_pattern}'] = {{ default: Page }};

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
            ]),
            extensions: Some(vec![
                ".tsx".to_string(),
                ".ts".to_string(),
                ".jsx".to_string(),
                ".js".to_string(),
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
                manifest.add_page(&route.pattern, &filename);
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
    let cn = module_name.replace('/', "-");
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
                if path.ends_with(".css") {
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

        let config = RexConfig::new(root);
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

        // Preamble
        assert!(bundle.starts_with("// Rex Server Bundle"), "should start with preamble");
        assert!(bundle.contains("'use strict'"), "should have strict mode");

        // Page registry
        assert!(bundle.contains("globalThis.__rex_pages"), "should init page registry");
        assert!(
            bundle.contains("globalThis.__rex_pages['index']"),
            "should register index page"
        );

        // SSR runtime functions
        assert!(
            bundle.contains("globalThis.__rex_render_page"),
            "should have render function"
        );
        assert!(
            bundle.contains("globalThis.__rex_get_server_side_props"),
            "should have GSSP executor"
        );
        assert!(
            bundle.contains("globalThis.__rex_resolve_gssp"),
            "should have GSSP resolver"
        );
        assert!(
            bundle.contains("__REX_ASYNC__"),
            "should have async sentinel"
        );
    }

    #[tokio::test]
    async fn test_server_bundle_cjs_format() {
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

        // Should use CJS, not ESM
        // The page code is inside IIFEs, check it doesn't have raw ESM
        assert!(
            !bundle.contains("export default"),
            "should not have ESM export default"
        );
        assert!(
            !bundle.contains("import React"),
            "should not have ESM import"
        );

        // Should have require() shim
        assert!(
            bundle.contains("var require = function(name)"),
            "should have require shim"
        );
        assert!(
            bundle.contains("globalThis.__React"),
            "require shim should reference React global"
        );

        // CJS module wrapper
        assert!(
            bundle.contains("var exports = {}"),
            "should have CJS exports"
        );
        assert!(
            bundle.contains("var module = { exports: exports }"),
            "should have CJS module"
        );
        assert!(
            bundle.contains("return module.exports"),
            "should return module.exports"
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
            bundle.contains("globalThis.__rex_pages['index']"),
            "should have index page"
        );
        assert!(
            bundle.contains("globalThis.__rex_pages['about']"),
            "should have about page"
        );
        assert!(
            bundle.contains("globalThis.__rex_pages['blog/[slug]']"),
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
            bundle.contains("globalThis.__rex_app"),
            "should register _app"
        );
        assert!(
            bundle.contains("// _app"),
            "should have _app comment marker"
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
            r#"{"name":"react-dom","version":"19.0.0","main":"index.js","exports":{".":{"default":"./index.js"},"./client":{"default":"./client.js"}}}"#,
        )
        .unwrap();
        fs::write(react_dom_dir.join("index.js"), "export default {};\n").unwrap();
        fs::write(
            react_dom_dir.join("client.js"),
            "export function hydrateRoot() {}\nexport function createRoot() {}\n",
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
            bundle.contains("globalThis.__rex_document"),
            "should register _document"
        );
        assert!(
            bundle.contains("// _document"),
            "should have _document comment marker"
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

        let config = RexConfig::new(root);
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
}

