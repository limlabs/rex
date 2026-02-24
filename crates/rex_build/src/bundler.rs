use crate::entries::generate_build_id;
use crate::manifest::AssetManifest;
use crate::transform::{TransformOptions, transform_file};
use anyhow::Result;
use rex_core::RexConfig;
use rex_router::ScanResult;
use std::fs;
use std::path::Path;
use tracing::info;

/// Build result containing paths to generated bundles
#[derive(Debug, Clone)]
pub struct BuildResult {
    pub build_id: String,
    pub server_bundle_path: std::path::PathBuf,
    pub manifest: AssetManifest,
}

/// Build both server and client bundles
pub fn build_bundles(config: &RexConfig, scan: &ScanResult) -> Result<BuildResult> {
    let build_id = generate_build_id();
    let server_dir = config.server_build_dir();
    let client_dir = config.client_build_dir();

    fs::create_dir_all(&server_dir)?;
    fs::create_dir_all(&client_dir)?;

    info!("Building server bundle...");
    let server_bundle_path = build_server_bundle(config, scan, &server_dir)?;

    info!("Building client bundles...");
    let manifest = build_client_bundles(config, scan, &client_dir, &build_id)?;

    // Save manifest
    manifest.save(&config.manifest_path())?;

    Ok(BuildResult {
        build_id,
        server_bundle_path,
        manifest,
    })
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
        typescript: true,
        jsx: true,
        ..Default::default()
    };

    // require() shim for CJS modules — maps known packages to V8 globals
    bundle.push_str(
        r#"var require = function(name) {
    if (name === 'react') return { default: globalThis.__React, createElement: globalThis.__React.createElement };
    return {};
};
"#,
    );

    // Transform and include each page as a module in a registry
    bundle.push_str("globalThis.__rex_pages = globalThis.__rex_pages || {};\n\n");

    // Transform all route pages (SWC CJS transform handles imports/exports)
    for route in &scan.routes {
        let source = fs::read_to_string(&route.abs_path)?;
        let transformed =
            transform_file(&source, &route.abs_path.to_string_lossy(), &server_opts)?;
        let module_name = route.module_name();

        bundle.push_str(&format!("// Page: {}\n", module_name));
        bundle.push_str(&format!(
            "globalThis.__rex_pages['{}'] = (function() {{\n  var exports = {{}};\n  var module = {{ exports: exports }};\n",
            module_name
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

    // Transform _app if present
    if let Some(app) = &scan.app {
        let source = fs::read_to_string(&app.abs_path)?;
        let transformed =
            transform_file(&source, &app.abs_path.to_string_lossy(), &server_opts)?;
        bundle.push_str("// _app\n");
        bundle.push_str("globalThis.__rex_app = (function() {\n  var exports = {};\n  var module = { exports: exports };\n");
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

    // SSR functions
    bundle.push_str(
        r#"
// SSR render function
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

    return ReactDOMServer.renderToString(element);
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
"#,
    );

    let bundle_path = output_dir.join("server-bundle.js");
    fs::write(&bundle_path, &bundle)?;
    info!(path = %bundle_path.display(), "Server bundle written");

    Ok(bundle_path)
}

/// Build client-side bundles: one per page, plus vendor scripts
fn build_client_bundles(
    config: &RexConfig,
    scan: &ScanResult,
    output_dir: &Path,
    build_id: &str,
) -> Result<AssetManifest> {
    let mut manifest = AssetManifest::new(build_id.to_string());

    // Build vendor scripts (React runtime for the browser)
    manifest.vendor_scripts = build_vendor_scripts(config, output_dir, build_id)?;

    let client_opts = TransformOptions {
        server: false,
        typescript: true,
        jsx: true,
        fast_refresh: config.dev,
    };

    for route in &scan.routes {
        let source = fs::read_to_string(&route.abs_path)?;
        let transformed =
            transform_file(&source, &route.abs_path.to_string_lossy(), &client_opts)?;
        let module_name = route.module_name();

        // Generate a filename from the route
        let chunk_name = module_name.replace('/', "-");
        let chunk_name = if chunk_name.is_empty() {
            "index".to_string()
        } else {
            chunk_name
        };
        let filename = format!("{chunk_name}-{}.js", &build_id[..8]);

        // For the prototype, write the transformed page as a standalone script
        // that expects React to be globally available
        let mut client_js = String::new();
        client_js.push_str("// Rex Client Chunk - Auto-generated\n");
        client_js.push_str("(function() {\n");
        client_js.push_str("'use strict';\n");
        client_js.push_str(&transformed);
        client_js.push_str("\n");

        // Hydration bootstrap
        client_js.push_str(&format!(
            r#"
  var React = window.React;
  var ReactDOM = window.ReactDOM;
  if (typeof exports !== 'undefined' && exports.default) {{
    var dataEl = document.getElementById('__REX_DATA__');
    var pageProps = dataEl ? JSON.parse(dataEl.textContent) : {{}};
    var container = document.getElementById('__rex');
    if (container && ReactDOM.hydrateRoot) {{
      var element = React.createElement(exports.default, pageProps);
      window.__REX_ROOT__ = ReactDOM.hydrateRoot(container, element);
    }}
  }}
"#
        ));

        client_js.push_str("})();\n");

        let chunk_path = output_dir.join(&filename);
        fs::write(&chunk_path, &client_js)?;

        manifest.add_page(&route.pattern, &filename);
    }

    Ok(manifest)
}

/// Build vendor scripts (React runtime wrapped for browser use).
/// Reads React CJS/UMD from node_modules, wraps for global assignment,
/// and writes to the client output directory.
fn build_vendor_scripts(
    config: &RexConfig,
    output_dir: &Path,
    build_id: &str,
) -> Result<Vec<String>> {
    let nm = config.node_modules_dir();
    let mut vendor_files = Vec::new();
    let hash = &build_id[..8];

    // Try React 19 CJS first
    let react_cjs = nm.join("react/cjs/react.production.js");
    let react_dom_cjs = nm.join("react-dom/cjs/react-dom.production.js");

    if react_cjs.exists() && react_dom_cjs.exists() {
        // React CJS → window.React
        let react_src = fs::read_to_string(&react_cjs)?;
        let react_vendor = format!(
            "(function(){{\nvar exports={{}};var module={{exports:exports}};\n{react_src}\nwindow.React=module.exports;\n}})();\n"
        );
        let react_filename = format!("vendor-react-{hash}.js");
        fs::write(output_dir.join(&react_filename), &react_vendor)?;
        vendor_files.push(react_filename);

        // ReactDOM CJS → window.ReactDOM (requires React)
        let react_dom_src = fs::read_to_string(&react_dom_cjs)?;
        let react_dom_vendor = format!(
            "(function(){{\nvar exports={{}};var module={{exports:exports}};\nvar require=function(n){{if(n==='react')return window.React;return {{}}}};\n{react_dom_src}\nwindow.ReactDOM=module.exports;\n}})();\n"
        );
        let react_dom_filename = format!("vendor-react-dom-{hash}.js");
        fs::write(output_dir.join(&react_dom_filename), &react_dom_vendor)?;
        vendor_files.push(react_dom_filename);

        return Ok(vendor_files);
    }

    // Fallback: React 18 UMD
    let react_umd = nm.join("react/umd/react.production.min.js");
    let react_dom_umd = nm.join("react-dom/umd/react-dom.production.min.js");

    if react_umd.exists() && react_dom_umd.exists() {
        let react_src = fs::read_to_string(&react_umd)?;
        let react_filename = format!("vendor-react-{hash}.js");
        fs::write(output_dir.join(&react_filename), &react_src)?;
        vendor_files.push(react_filename);

        let react_dom_src = fs::read_to_string(&react_dom_umd)?;
        let react_dom_filename = format!("vendor-react-dom-{hash}.js");
        fs::write(output_dir.join(&react_dom_filename), &react_dom_src)?;
        vendor_files.push(react_dom_filename);

        return Ok(vendor_files);
    }

    // No React found — hydration won't work, but SSR stub may still function
    Ok(vendor_files)
}
