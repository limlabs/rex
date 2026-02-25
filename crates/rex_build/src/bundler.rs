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
        append_page_iife(&mut bundle, &format!("Page: {module_name}"), &format!("globalThis.__rex_pages['{module_name}']"), &transformed);
    }

    // Include special pages (404, _error) in the registry so they can be SSR'd
    for (label, route_opt) in [("404", &scan.not_found), ("_error", &scan.error)] {
        if let Some(route) = route_opt {
            let source = fs::read_to_string(&route.abs_path)?;
            let transformed =
                transform_file(&source, &route.abs_path.to_string_lossy(), &server_opts)?;
            append_page_iife(&mut bundle, label, &format!("globalThis.__rex_pages['{label}']"), &transformed);
        }
    }

    // Transform _app if present
    if let Some(app) = &scan.app {
        let source = fs::read_to_string(&app.abs_path)?;
        let transformed =
            transform_file(&source, &app.abs_path.to_string_lossy(), &server_opts)?;
        append_page_iife(&mut bundle, "_app", "globalThis.__rex_app", &transformed);
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

#[cfg(test)]
mod tests {
    use super::*;
    use rex_core::{PageType, Route};
    use std::path::PathBuf;

    /// Create a temp project directory with page files, returning (config, scan)
    fn setup_test_project(
        pages: &[(&str, &str)],
        app_source: Option<&str>,
    ) -> (tempfile::TempDir, RexConfig, ScanResult) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();

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

        let config = RexConfig::new(root);
        let scan = ScanResult {
            routes,
            app,
            document: None,
            error: None,
            not_found: None,
        };

        (tmp, config, scan)
    }

    #[test]
    fn test_server_bundle_structure() {
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
        let result = build_bundles(&config, &scan).unwrap();
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

    #[test]
    fn test_server_bundle_cjs_format() {
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
        let result = build_bundles(&config, &scan).unwrap();
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

    #[test]
    fn test_server_bundle_multiple_pages() {
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
        let result = build_bundles(&config, &scan).unwrap();
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

    #[test]
    fn test_server_bundle_with_app() {
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
        let result = build_bundles(&config, &scan).unwrap();
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

    #[test]
    fn test_client_bundles_per_page() {
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
        let result = build_bundles(&config, &scan).unwrap();
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

    #[test]
    fn test_manifest_contents() {
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
        let result = build_bundles(&config, &scan).unwrap();

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

    /// Helper to create fake node_modules with React CJS or UMD files
    fn setup_fake_node_modules(root: &Path, layout: &str) {
        let nm = root.join("node_modules");
        match layout {
            "cjs" => {
                // React 19 CJS layout
                let react_cjs_dir = nm.join("react/cjs");
                let react_dom_cjs_dir = nm.join("react-dom/cjs");
                fs::create_dir_all(&react_cjs_dir).unwrap();
                fs::create_dir_all(&react_dom_cjs_dir).unwrap();
                fs::write(
                    react_cjs_dir.join("react.production.js"),
                    "module.exports = { createElement: function() { return {}; } };",
                )
                .unwrap();
                fs::write(
                    react_dom_cjs_dir.join("react-dom.production.js"),
                    "module.exports = { createRoot: function() {} };",
                )
                .unwrap();
            }
            "umd" => {
                // React 18 UMD layout
                let react_umd_dir = nm.join("react/umd");
                let react_dom_umd_dir = nm.join("react-dom/umd");
                fs::create_dir_all(&react_umd_dir).unwrap();
                fs::create_dir_all(&react_dom_umd_dir).unwrap();
                fs::write(
                    react_umd_dir.join("react.production.min.js"),
                    "window.React = { createElement: function() { return {}; } };",
                )
                .unwrap();
                fs::write(
                    react_dom_umd_dir.join("react-dom.production.min.js"),
                    "window.ReactDOM = { createRoot: function() {} };",
                )
                .unwrap();
            }
            _ => {} // "none" — no node_modules
        }
    }

    #[test]
    fn test_vendor_scripts_react19_cjs() {
        let (_tmp, config, scan) = setup_test_project(
            &[(
                "index.tsx",
                "export default function Home() { return <div>Home</div>; }",
            )],
            None,
        );
        setup_fake_node_modules(&config.project_root, "cjs");

        let result = build_bundles(&config, &scan).unwrap();

        assert_eq!(
            result.manifest.vendor_scripts.len(),
            2,
            "should have react + react-dom vendor scripts"
        );
        assert!(
            result.manifest.vendor_scripts[0].starts_with("vendor-react-"),
            "first vendor script should be react"
        );
        assert!(
            result.manifest.vendor_scripts[1].starts_with("vendor-react-dom-"),
            "second vendor script should be react-dom"
        );

        // Verify files exist and contain CJS wrapper
        let client_dir = config.client_build_dir();
        let react_vendor = fs::read_to_string(
            client_dir.join(&result.manifest.vendor_scripts[0]),
        )
        .unwrap();
        assert!(
            react_vendor.contains("window.React=module.exports"),
            "CJS react vendor should assign window.React"
        );

        let react_dom_vendor = fs::read_to_string(
            client_dir.join(&result.manifest.vendor_scripts[1]),
        )
        .unwrap();
        assert!(
            react_dom_vendor.contains("window.ReactDOM=module.exports"),
            "CJS react-dom vendor should assign window.ReactDOM"
        );
        assert!(
            react_dom_vendor.contains("require"),
            "react-dom vendor should have require shim"
        );
    }

    #[test]
    fn test_vendor_scripts_react18_umd() {
        let (_tmp, config, scan) = setup_test_project(
            &[(
                "index.tsx",
                "export default function Home() { return <div>Home</div>; }",
            )],
            None,
        );
        setup_fake_node_modules(&config.project_root, "umd");

        let result = build_bundles(&config, &scan).unwrap();

        assert_eq!(
            result.manifest.vendor_scripts.len(),
            2,
            "should have react + react-dom vendor scripts"
        );

        // UMD scripts are written as-is (no wrapper needed)
        let client_dir = config.client_build_dir();
        let react_vendor = fs::read_to_string(
            client_dir.join(&result.manifest.vendor_scripts[0]),
        )
        .unwrap();
        assert!(
            react_vendor.contains("window.React"),
            "UMD react should set window.React"
        );
    }

    #[test]
    fn test_vendor_scripts_no_react() {
        let (_tmp, config, scan) = setup_test_project(
            &[(
                "index.tsx",
                "export default function Home() { return <div>Home</div>; }",
            )],
            None,
        );
        // No setup_fake_node_modules — no React available

        let result = build_bundles(&config, &scan).unwrap();

        assert!(
            result.manifest.vendor_scripts.is_empty(),
            "should have no vendor scripts when React not found"
        );
    }
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
