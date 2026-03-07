use crate::build_utils::{extract_middleware_matchers, generate_build_id, runtime_server_dir};
use crate::client_bundle::build_client_bundles;
use crate::css_modules::process_css_modules;
use crate::manifest::AssetManifest;
use crate::server_bundle::build_server_bundle;
use crate::tailwind::process_tailwind_css;
use anyhow::Result;
use rex_core::{ProjectConfig, RexConfig};
use rex_router::ScanResult;
use std::fs;
use std::path::Path;
use tracing::{debug, info, info_span, Instrument};

// Re-exports for crate::rsc_bundler compatibility
pub(crate) use crate::build_utils::runtime_client_dir;
pub use crate::server_bundle::V8_POLYFILLS;

// Re-exports for public API (via lib.rs)
pub use crate::tailwind::{
    collect_all_css_import_paths, find_tailwind_bin, needs_tailwind,
    process_tailwind_css as process_tailwind_css_pub,
};

/// Compute the `modules` resolve dirs for rolldown.
///
/// If the project has no `package.json`, extract the embedded React packages
/// into the project's `node_modules/` first (zero-config mode). Either way
/// rolldown resolves from the standard `node_modules/` path.
pub(crate) fn resolve_modules_dirs(config: &RexConfig) -> Result<Vec<String>> {
    if !crate::builtin_modules::has_package_json(&config.project_root) {
        crate::builtin_modules::ensure_builtin_modules(&config.project_root)?;
        info!(
            "Using built-in React {}",
            crate::builtin_modules::EMBEDDED_REACT_VERSION
        );
    }
    Ok(vec![
        config
            .project_root
            .join("node_modules")
            .to_string_lossy()
            .to_string(),
        "node_modules".to_string(),
    ])
}

/// Build result containing paths to generated bundles
#[derive(Debug, Clone)]
pub struct BuildResult {
    pub build_id: String,
    pub server_bundle_path: std::path::PathBuf,
    pub manifest: AssetManifest,
}

/// Build both server and client bundles
pub async fn build_bundles(
    config: &RexConfig,
    scan: &ScanResult,
    project_config: &ProjectConfig,
) -> Result<BuildResult> {
    let build_id = generate_build_id();
    let server_dir = config.server_build_dir();
    let client_dir = config.client_build_dir();

    // Clean output directories to remove stale artifacts from previous builds.
    // Use let _ to ignore errors — on macOS, remove_dir_all can race with
    // Spotlight/fsevents and fail with ENOTEMPTY (os error 66).
    let _ = fs::remove_dir_all(&server_dir);
    let _ = fs::remove_dir_all(&client_dir);
    fs::create_dir_all(&server_dir)?;
    fs::create_dir_all(&client_dir)?;

    // Pre-process CSS modules (generates scoped CSS + JS proxy files)
    let css_modules = process_css_modules(scan, &client_dir, &build_id)?;

    // Pre-process Tailwind CSS files (compile with tailwindcss CLI)
    let tailwind_outputs = process_tailwind_css(config, scan, &client_dir)?;

    // Replace process.env.NODE_ENV so React/scheduler resolve to production builds
    let node_env = if config.dev {
        "\"development\""
    } else {
        "\"production\""
    };
    let define = vec![("process.env.NODE_ENV".to_string(), node_env.to_string())];

    // Resolve module directories once for all bundle steps
    let module_dirs = resolve_modules_dirs(config)?;

    let has_pages = !scan.routes.is_empty() || scan.app.is_some();

    let (server_bundle_path, mut manifest) = if has_pages {
        // Build server and client bundles in parallel
        let server_fut = build_server_bundle(
            config,
            scan,
            &server_dir,
            &css_modules.page_overrides,
            &define,
            project_config,
            &module_dirs,
        )
        .instrument(info_span!("build_server_bundle"));
        let client_fut = build_client_bundles(
            config,
            scan,
            &client_dir,
            &build_id,
            &css_modules,
            &define,
            &tailwind_outputs,
            project_config,
            &module_dirs,
        )
        .instrument(info_span!("build_client_bundles"));

        tokio::try_join!(server_fut, client_fut)?
    } else {
        // App-only project: create a minimal server bundle with V8 polyfills + React + stubs
        build_minimal_server_bundle(
            config,
            scan,
            &server_dir,
            &define,
            &module_dirs,
            &build_id,
            project_config,
        )
        .await?
    };

    // Set middleware matchers on manifest (if middleware exists)
    if let Some(mw_path) = &scan.middleware {
        let source = fs::read_to_string(mw_path)?;
        manifest.middleware_matchers = Some(extract_middleware_matchers(&source));
    }

    // Build RSC bundles if app/ scan is present
    if let Some(app_scan) = &scan.app_scan {
        let rsc_result =
            crate::rsc_bundler::build_rsc_bundles(config, app_scan, &build_id, &define).await?;

        // Populate app_routes in manifest
        for route in &app_scan.routes {
            manifest.app_routes.insert(
                route.pattern.clone(),
                crate::manifest::AppRouteAssets {
                    client_chunks: rsc_result.client_chunks.clone(),
                    layout_chain: route
                        .layout_chain
                        .iter()
                        .map(|p| p.to_string_lossy().to_string())
                        .collect(),
                },
            );
        }

        manifest.client_reference_manifest = Some(rsc_result.client_manifest);
        manifest.rsc_server_bundle =
            Some(rsc_result.server_bundle_path.to_string_lossy().to_string());
        manifest.rsc_ssr_bundle = Some(rsc_result.ssr_bundle_path.to_string_lossy().to_string());

        debug!(app_routes = manifest.app_routes.len(), "RSC bundles built");
    }

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

/// Build a minimal server bundle for app-only projects (no pages/ routes).
async fn build_minimal_server_bundle(
    config: &RexConfig,
    scan: &ScanResult,
    server_dir: &Path,
    define: &[(String, String)],
    module_dirs: &[String],
    build_id: &str,
    project_config: &ProjectConfig,
) -> Result<(std::path::PathBuf, AssetManifest)> {
    debug!("No pages/ routes — creating minimal server bundle");
    let entry_dir = server_dir.join("_server_entry");
    fs::create_dir_all(&entry_dir)?;

    let mut entry = String::from(
        r#"import { createElement } from 'react';
import { renderToString } from 'react-dom/server';
globalThis.__rex_pages = {};
var __rex_createElement = createElement;
var __rex_renderToString = renderToString;

// Stub render functions for V8 isolate compatibility (app-only project)
globalThis.__rex_render_page = function() { return JSON.stringify({ body: '', head: '' }); };
globalThis.__rex_get_server_side_props = function() { return JSON.stringify({ props: {} }); };
globalThis.__rex_get_static_props = function() { return JSON.stringify({ props: {} }); };
globalThis.__rex_render_document = function() { return JSON.stringify({ html: '', head: '' }); };
"#,
    );

    // Include API routes in the minimal bundle (pages/api/ can coexist with app/)
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
        // Add the API handler runtime functions (same as in build_server_bundle)
        entry.push_str(
            r#"
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
"#,
        );
    }
    let entry_path = entry_dir.join("server-entry.js");
    fs::write(&entry_path, entry)?;

    let runtime_dir = runtime_server_dir()?;
    let mut module_types = rustc_hash::FxHashMap::default();
    module_types.insert(".css".to_string(), rolldown::ModuleType::Empty);

    let options = rolldown::BundlerOptions {
        input: Some(vec![rolldown::InputItem {
            name: Some("server-bundle".to_string()),
            import: entry_path.to_string_lossy().to_string(),
        }]),
        cwd: Some(config.project_root.clone()),
        format: Some(rolldown::OutputFormat::Iife),
        dir: Some(server_dir.to_string_lossy().to_string()),
        entry_filenames: Some("server-bundle.js".to_string().into()),
        platform: Some(rolldown::Platform::Browser),
        module_types: Some(module_types),
        define: Some(define.iter().cloned().collect()),
        banner: Some(rolldown::AddonOutputOption::String(Some(
            V8_POLYFILLS.to_string(),
        ))),
        tsconfig: Some(rolldown_common::TsConfig::Auto(false)),
        treeshake: crate::rsc_build_config::react_treeshake_options(),
        resolve: Some(rolldown::ResolveOptions {
            extensions: Some(vec![
                ".tsx".to_string(),
                ".ts".to_string(),
                ".jsx".to_string(),
                ".js".to_string(),
            ]),
            modules: Some(module_dirs.to_vec()),
            alias: Some(vec![
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
            ]),
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
        .map_err(|e| anyhow::anyhow!("Failed to create server bundler: {e}"))?;
    bundler
        .write()
        .await
        .map_err(|e| anyhow::anyhow!("Server bundle failed: {e:?}"))?;

    let _ = fs::remove_dir_all(&entry_dir);

    let bundle_path = server_dir.join("server-bundle.js");
    let manifest = AssetManifest::new(build_id.to_string());
    Ok((bundle_path, manifest))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::build_utils::detect_data_strategy_from_source;
    use crate::css_modules::{generate_css_module_proxy, parse_css_classes, scope_css};
    use crate::tailwind::find_tailwind_bin;
    use rex_core::{DataStrategy, PageType, Route};
    use std::collections::HashMap;
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
            middleware: None,
            app_scan: None,
            mcp_tools: vec![],
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
        let result = build_bundles(&config, &scan, &ProjectConfig::default())
            .await
            .unwrap();
        let bundle = fs::read_to_string(&result.server_bundle_path).unwrap();

        // V8 polyfills (injected as banner)
        assert!(
            bundle.contains("globalThis.process"),
            "should have process polyfill"
        );
        assert!(
            bundle.contains("MessageChannel"),
            "should have MessageChannel polyfill"
        );
        assert!(
            bundle.contains("globalThis.Buffer"),
            "should have Buffer polyfill"
        );

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
        let result = build_bundles(&config, &scan, &ProjectConfig::default())
            .await
            .unwrap();
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
        let result = build_bundles(&config, &scan, &ProjectConfig::default())
            .await
            .unwrap();
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
            bundle.contains("__rex_pages[\"blog/[slug]\"]")
                || bundle.contains("__rex_pages['blog/[slug]']"),
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
        let result = build_bundles(&config, &scan, &ProjectConfig::default())
            .await
            .unwrap();
        let bundle = fs::read_to_string(&result.server_bundle_path).unwrap();

        assert!(bundle.contains("__rex_app"), "should register _app");
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
        let result = build_bundles(&config, &scan, &ProjectConfig::default())
            .await
            .unwrap();
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
        let result = build_bundles(&config, &scan, &ProjectConfig::default())
            .await
            .unwrap();

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
    fn setup_mock_node_modules(root: &std::path::Path) {
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
            "export function createElement(type, props, ...children) { return { type, props, children }; }\nexport const Suspense = Symbol.for('react.suspense');\nexport default { createElement, Suspense };\n",
        )
        .unwrap();
        fs::write(
            react_dir.join("jsx-runtime.js"),
            "export function jsx(type, props) { return { type, props }; }\nexport function jsxs(type, props) { return { type, props }; }\nexport const Fragment = 'Fragment';\nexport const Suspense = Symbol.for('react.suspense');\n",
        )
        .unwrap();
        fs::write(
            react_dir.join("jsx-dev-runtime.js"),
            "export function jsxDEV(type, props) { return { type, props }; }\nexport const Fragment = 'Fragment';\nexport const Suspense = Symbol.for('react.suspense');\n",
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
            r#"
var _Suspense = Symbol.for('react.suspense');
function renderEl(el) {
  if (el == null || el === false) return '';
  if (typeof el === 'string') return el;
  if (typeof el === 'number') return '' + el;
  if (Array.isArray(el)) return el.map(renderEl).join('');
  if (typeof el === 'object' && el.type) {
    if (el.type === _Suspense) {
      try {
        var inner = '';
        if (el.children && el.children.length) inner += el.children.map(renderEl).join('');
        var pr = el.props || {};
        if (pr.children != null && !(el.children && el.children.length)) inner += renderEl(pr.children);
        return inner;
      } catch (e) {
        if (e && typeof e.then === 'function') {
          return renderEl(el.props && el.props.fallback);
        }
        throw e;
      }
    }
    if (typeof el.type === 'function') {
      var p = Object.assign({}, el.props || {});
      if (el.children && el.children.length) p.children = el.children.length === 1 ? el.children[0] : el.children;
      return renderEl(el.type(p));
    }
    var tag = el.type;
    var attrs = '';
    var pr = el.props || {};
    for (var k in pr) {
      if (k === 'children' || k === 'key' || k === 'ref') continue;
      if (typeof pr[k] === 'function' || typeof pr[k] === 'object') continue;
      if (k === 'className') attrs += ' class="' + pr[k] + '"';
      else attrs += ' ' + k + '="' + pr[k] + '"';
    }
    var ch = '';
    if (el.children && el.children.length) ch += el.children.map(renderEl).join('');
    if (pr.children != null && !(el.children && el.children.length)) ch += renderEl(pr.children);
    return '<' + tag + attrs + '>' + ch + '</' + tag + '>';
  }
  return '' + el;
}
export function renderToString(el) { return renderEl(el); }
"#,
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
        let result = build_bundles(&config, &scan, &ProjectConfig::default())
            .await
            .unwrap();
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
            middleware: None,
            app_scan: None,
            mcp_tools: vec![],
        };

        let result = build_bundles(&config, &scan, &ProjectConfig::default())
            .await
            .unwrap();

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
        let result = build_bundles(&config, &scan, &ProjectConfig::default())
            .await
            .unwrap();

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
        let index_js =
            fs::read_to_string(client_dir.join(result.manifest.pages["/"].js.clone())).unwrap();
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
        let result = build_bundles(&config, &scan, &ProjectConfig::default())
            .await
            .unwrap();

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
            middleware: None,
            app_scan: None,
            mcp_tools: vec![],
        };

        let result = build_bundles(&config, &scan, &ProjectConfig::default())
            .await
            .unwrap();

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
            .filter(|e| e.path().to_string_lossy().contains("Home.module-"))
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

    // --- Integration tests: build → V8 SSR ---

    /// Build a project and load the server bundle into V8 for SSR testing.
    async fn build_and_load(
        config: &RexConfig,
        scan: &ScanResult,
    ) -> (BuildResult, rex_v8::IsolatePool) {
        let result = build_bundles(config, scan, &ProjectConfig::default())
            .await
            .expect("build failed");
        let bundle = fs::read_to_string(&result.server_bundle_path).expect("read bundle");
        rex_v8::init_v8();
        let pool =
            rex_v8::IsolatePool::new(1, std::sync::Arc::new(bundle), None).expect("create pool");
        (result, pool)
    }

    /// Build and load with fs polyfill enabled (project_root passed to isolate pool).
    async fn build_and_load_with_root(
        config: &RexConfig,
        scan: &ScanResult,
    ) -> (BuildResult, rex_v8::IsolatePool) {
        let result = build_bundles(config, scan, &ProjectConfig::default())
            .await
            .expect("build failed");
        let bundle = fs::read_to_string(&result.server_bundle_path).expect("read bundle");
        rex_v8::init_v8();
        let root_str = config.project_root.to_string_lossy().to_string();
        let pool = rex_v8::IsolatePool::new(
            1,
            std::sync::Arc::new(bundle),
            Some(std::sync::Arc::new(root_str)),
        )
        .expect("create pool");
        (result, pool)
    }

    #[tokio::test]
    async fn test_integration_basic_ssr() {
        let (_tmp, config, scan) = setup_test_project(
            &[(
                "index.tsx",
                r#"
                export default function Home() {
                    return <div><h1>Hello Rex</h1><p>Welcome</p></div>;
                }
                "#,
            )],
            None,
        );

        let (_result, pool) = build_and_load(&config, &scan).await;

        let render = pool
            .execute(|iso| iso.render_page("index", "{}"))
            .await
            .expect("pool execute")
            .expect("render_page");

        assert!(
            render.body.contains("Hello Rex"),
            "SSR should render heading: {}",
            render.body
        );
        assert!(
            render.body.contains("Welcome"),
            "SSR should render paragraph: {}",
            render.body
        );
        assert!(
            render.body.contains("<div>"),
            "SSR should produce HTML tags: {}",
            render.body
        );
    }

    #[tokio::test]
    async fn test_integration_ssr_with_props() {
        let (_tmp, config, scan) = setup_test_project(
            &[(
                "index.tsx",
                r#"
                export default function Home({ message }) {
                    return <div><h1>{message}</h1></div>;
                }
                export function getServerSideProps() {
                    return { props: { message: "Dynamic content" } };
                }
                "#,
            )],
            None,
        );

        let (_result, pool) = build_and_load(&config, &scan).await;

        // Test GSSP
        let gssp_json = pool
            .execute(|iso| iso.get_server_side_props("index", "{\"params\":{},\"query\":{}}"))
            .await
            .expect("pool execute")
            .expect("gssp");

        let gssp: serde_json::Value = serde_json::from_str(&gssp_json).unwrap();
        assert_eq!(
            gssp["props"]["message"].as_str(),
            Some("Dynamic content"),
            "GSSP should return props"
        );

        // Test SSR with those props
        let render = pool
            .execute(|iso| iso.render_page("index", "{\"message\":\"Dynamic content\"}"))
            .await
            .expect("pool execute")
            .expect("render_page");

        assert!(
            render.body.contains("Dynamic content"),
            "SSR should render GSSP props: {}",
            render.body
        );
    }

    #[tokio::test]
    async fn test_integration_multiple_pages() {
        let (_tmp, config, scan) = setup_test_project(
            &[
                (
                    "index.tsx",
                    r#"
                    export default function Home() {
                        return <div><h1>Home Page</h1></div>;
                    }
                    "#,
                ),
                (
                    "about.tsx",
                    r#"
                    export default function About() {
                        return <div><h1>About Page</h1></div>;
                    }
                    "#,
                ),
            ],
            None,
        );

        let (_result, pool) = build_and_load(&config, &scan).await;

        // Render home
        let home = pool
            .execute(|iso| iso.render_page("index", "{}"))
            .await
            .unwrap()
            .unwrap();
        assert!(home.body.contains("Home Page"), "home: {}", home.body);

        // Render about
        let about = pool
            .execute(|iso| iso.render_page("about", "{}"))
            .await
            .unwrap()
            .unwrap();
        assert!(about.body.contains("About Page"), "about: {}", about.body);
    }

    #[tokio::test]
    async fn test_integration_css_module_in_ssr() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();

        setup_mock_node_modules(&root);

        let pages_dir = root.join("pages");
        let styles_dir = root.join("styles");
        fs::create_dir_all(&pages_dir).unwrap();
        fs::create_dir_all(&styles_dir).unwrap();

        fs::write(
            styles_dir.join("Home.module.css"),
            ".wrapper { padding: 20px; }\n.heading { color: blue; }\n",
        )
        .unwrap();

        let index_path = pages_dir.join("index.tsx");
        fs::write(
            &index_path,
            r#"import styles from '../styles/Home.module.css';
export default function Home() {
    return <div className={styles.wrapper}><h1 className={styles.heading}>Styled</h1></div>;
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
            middleware: None,
            app_scan: None,
            mcp_tools: vec![],
        };

        let (_result, pool) = build_and_load(&config, &scan).await;

        let render = pool
            .execute(|iso| iso.render_page("index", "{}"))
            .await
            .unwrap()
            .unwrap();

        assert!(
            render.body.contains("Styled"),
            "should render page content: {}",
            render.body
        );
        // Scoped class names should appear in the HTML
        assert!(
            render.body.contains("Home_wrapper_"),
            "should have scoped class name for wrapper: {}",
            render.body
        );
        assert!(
            render.body.contains("Home_heading_"),
            "should have scoped class name for heading: {}",
            render.body
        );
    }

    #[tokio::test]
    async fn test_integration_suspense_ssr() {
        let (_tmp, config, scan) = setup_test_project(
            &[(
                "index.tsx",
                r#"
                import { Suspense } from 'react';
                export default function Home() {
                    return (
                        <Suspense fallback={<div>Loading...</div>}>
                            <div><h1>Suspense Content</h1></div>
                        </Suspense>
                    );
                }
                "#,
            )],
            None,
        );

        let (_result, pool) = build_and_load(&config, &scan).await;

        let render = pool
            .execute(|iso| iso.render_page("index", "{}"))
            .await
            .unwrap()
            .unwrap();

        assert!(
            render.body.contains("Suspense Content"),
            "SSR should render Suspense children: {}",
            render.body
        );
        assert!(
            !render.body.contains("Loading..."),
            "SSR should NOT render fallback when children render normally: {}",
            render.body
        );
    }

    #[tokio::test]
    async fn test_integration_fs_polyfill() {
        // Create a page that imports fs and uses readFileSync in GSSP
        let (_tmp, config, scan) = setup_test_project(
            &[(
                "index.tsx",
                r#"
                import fs from 'fs';
                export default function Home({ content }) {
                    return <div><h1>{content}</h1></div>;
                }
                export function getServerSideProps() {
                    const content = fs.readFileSync('data/message.txt', 'utf8');
                    return { props: { content } };
                }
                "#,
            )],
            None,
        );

        // Write the data file the page will read
        let data_dir = config.project_root.join("data");
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(data_dir.join("message.txt"), "hello from file").unwrap();

        let (_result, pool) = build_and_load_with_root(&config, &scan).await;

        // Test GSSP reads the file
        let gssp_json = pool
            .execute(|iso| iso.get_server_side_props("index", "{\"params\":{},\"query\":{}}"))
            .await
            .expect("pool execute")
            .expect("gssp");

        let gssp: serde_json::Value = serde_json::from_str(&gssp_json).unwrap();
        assert_eq!(
            gssp["props"]["content"].as_str(),
            Some("hello from file"),
            "GSSP should read file content via fs polyfill: {gssp_json}"
        );

        // Test SSR renders the file content
        let render = pool
            .execute(|iso| iso.render_page("index", "{\"content\":\"hello from file\"}"))
            .await
            .expect("pool execute")
            .expect("render_page");

        assert!(
            render.body.contains("hello from file"),
            "SSR should render file content: {}",
            render.body
        );
    }

    #[tokio::test]
    async fn test_integration_fs_promises_polyfill() {
        // Test the fs/promises shim with async getServerSideProps
        let (_tmp, config, scan) = setup_test_project(
            &[(
                "index.tsx",
                r#"
                import { readFile } from 'fs/promises';
                export default function Home({ content }) {
                    return <div><h1>{content}</h1></div>;
                }
                export async function getServerSideProps() {
                    const content = await readFile('data/async.txt', 'utf8');
                    return { props: { content } };
                }
                "#,
            )],
            None,
        );

        // Write the data file
        let data_dir = config.project_root.join("data");
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(data_dir.join("async.txt"), "async file content").unwrap();

        let (_result, pool) = build_and_load_with_root(&config, &scan).await;

        // Test async GSSP reads via fs/promises
        let gssp_json = pool
            .execute(|iso| iso.get_server_side_props("index", "{\"params\":{},\"query\":{}}"))
            .await
            .expect("pool execute")
            .expect("gssp");

        let gssp: serde_json::Value = serde_json::from_str(&gssp_json).unwrap();
        assert_eq!(
            gssp["props"]["content"].as_str(),
            Some("async file content"),
            "Async GSSP should read file content via fs/promises polyfill: {gssp_json}"
        );
    }

    #[tokio::test]
    async fn test_integration_fs_path_traversal_blocked() {
        // Verify that path traversal is blocked through the full pipeline
        let (_tmp, config, scan) = setup_test_project(
            &[(
                "index.tsx",
                r#"
                import fs from 'fs';
                export default function Home({ error }) {
                    return <div><h1>{error}</h1></div>;
                }
                export function getServerSideProps() {
                    try {
                        fs.readFileSync('../../etc/passwd', 'utf8');
                        return { props: { error: 'should have thrown' } };
                    } catch (e) {
                        return { props: { error: e.code || e.message } };
                    }
                }
                "#,
            )],
            None,
        );

        let (_result, pool) = build_and_load_with_root(&config, &scan).await;

        let gssp_json = pool
            .execute(|iso| iso.get_server_side_props("index", "{\"params\":{},\"query\":{}}"))
            .await
            .expect("pool execute")
            .expect("gssp");

        let gssp: serde_json::Value = serde_json::from_str(&gssp_json).unwrap();
        assert_eq!(
            gssp["props"]["error"].as_str(),
            Some("EACCES"),
            "Path traversal should be blocked: {gssp_json}"
        );
    }

    // ── path polyfill integration tests ────────────────────────

    #[tokio::test]
    async fn test_integration_path_polyfill() {
        // Test path.join, path.basename, path.dirname, path.extname via GSSP
        let (_tmp, config, scan) = setup_test_project(
            &[(
                "index.tsx",
                r#"
                import path from 'path';
                export default function Home(props) {
                    return <div>{JSON.stringify(props)}</div>;
                }
                export function getServerSideProps() {
                    return { props: {
                        joined: path.join('a', 'b', 'c'),
                        base: path.basename('/foo/bar.txt'),
                        baseExt: path.basename('/foo/bar.txt', '.txt'),
                        dir: path.dirname('/foo/bar.txt'),
                        ext: path.extname('/foo/bar.txt'),
                        normalized: path.join('a', '..', 'b', '.', 'c'),
                    }};
                }
                "#,
            )],
            None,
        );

        let (_result, pool) = build_and_load(&config, &scan).await;

        let gssp_json = pool
            .execute(|iso| iso.get_server_side_props("index", "{\"params\":{},\"query\":{}}"))
            .await
            .expect("pool execute")
            .expect("gssp");

        let gssp: serde_json::Value = serde_json::from_str(&gssp_json).unwrap();
        assert_eq!(gssp["props"]["joined"], "a/b/c", "path.join: {gssp_json}");
        assert_eq!(
            gssp["props"]["base"], "bar.txt",
            "path.basename: {gssp_json}"
        );
        assert_eq!(
            gssp["props"]["baseExt"], "bar",
            "path.basename with ext: {gssp_json}"
        );
        assert_eq!(gssp["props"]["dir"], "/foo", "path.dirname: {gssp_json}");
        assert_eq!(gssp["props"]["ext"], ".txt", "path.extname: {gssp_json}");
        assert_eq!(
            gssp["props"]["normalized"], "b/c",
            "path.join normalize: {gssp_json}"
        );
    }

    #[tokio::test]
    async fn test_integration_path_node_prefix() {
        // Verify `import path from 'node:path'` also resolves
        let (_tmp, config, scan) = setup_test_project(
            &[(
                "index.tsx",
                r#"
                import path from 'node:path';
                export default function Home(props) {
                    return <div>{props.joined}</div>;
                }
                export function getServerSideProps() {
                    return { props: { joined: path.join('x', 'y') }};
                }
                "#,
            )],
            None,
        );

        let (_result, pool) = build_and_load(&config, &scan).await;

        let gssp_json = pool
            .execute(|iso| iso.get_server_side_props("index", "{\"params\":{},\"query\":{}}"))
            .await
            .expect("pool execute")
            .expect("gssp");

        let gssp: serde_json::Value = serde_json::from_str(&gssp_json).unwrap();
        assert_eq!(
            gssp["props"]["joined"], "x/y",
            "node:path should work: {gssp_json}"
        );
    }

    #[tokio::test]
    async fn test_integration_path_named_imports() {
        // Verify named imports work: import { join, resolve } from 'path'
        let (_tmp, config, scan) = setup_test_project(
            &[(
                "index.tsx",
                r#"
                import { join, basename, dirname, extname, isAbsolute } from 'path';
                export default function Home(props) {
                    return <div>{JSON.stringify(props)}</div>;
                }
                export function getServerSideProps() {
                    return { props: {
                        joined: join('a', 'b'),
                        base: basename('/x/y.js'),
                        dir: dirname('/x/y.js'),
                        ext: extname('file.tar.gz'),
                        abs: isAbsolute('/foo'),
                        rel: isAbsolute('foo'),
                    }};
                }
                "#,
            )],
            None,
        );

        let (_result, pool) = build_and_load(&config, &scan).await;

        let gssp_json = pool
            .execute(|iso| iso.get_server_side_props("index", "{\"params\":{},\"query\":{}}"))
            .await
            .expect("pool execute")
            .expect("gssp");

        let gssp: serde_json::Value = serde_json::from_str(&gssp_json).unwrap();
        assert_eq!(gssp["props"]["joined"], "a/b");
        assert_eq!(gssp["props"]["base"], "y.js");
        assert_eq!(gssp["props"]["dir"], "/x");
        assert_eq!(gssp["props"]["ext"], ".gz");
        assert_eq!(gssp["props"]["abs"], true);
        assert_eq!(gssp["props"]["rel"], false);
    }

    // ── buffer polyfill integration tests ────────────────────────

    #[tokio::test]
    async fn test_integration_buffer_polyfill_global() {
        // Test Buffer global: from/toString with utf8, base64, hex
        let (_tmp, config, scan) = setup_test_project(
            &[(
                "index.tsx",
                r#"
                export default function Home(props) {
                    return <div>{JSON.stringify(props)}</div>;
                }
                export function getServerSideProps() {
                    var buf = Buffer.from('hello world');
                    return { props: {
                        utf8: buf.toString('utf8'),
                        base64: buf.toString('base64'),
                        hex: buf.toString('hex'),
                        roundtrip: Buffer.from(buf.toString('base64'), 'base64').toString('utf8'),
                        isBuffer: Buffer.isBuffer(buf),
                        notBuffer: Buffer.isBuffer('nope'),
                        len: buf.length,
                    }};
                }
                "#,
            )],
            None,
        );

        let (_result, pool) = build_and_load(&config, &scan).await;

        let gssp_json = pool
            .execute(|iso| iso.get_server_side_props("index", "{\"params\":{},\"query\":{}}"))
            .await
            .expect("pool execute")
            .expect("gssp");

        let gssp: serde_json::Value = serde_json::from_str(&gssp_json).unwrap();
        assert_eq!(gssp["props"]["utf8"], "hello world", "utf8: {gssp_json}");
        assert_eq!(
            gssp["props"]["base64"], "aGVsbG8gd29ybGQ=",
            "base64: {gssp_json}"
        );
        assert_eq!(
            gssp["props"]["hex"], "68656c6c6f20776f726c64",
            "hex: {gssp_json}"
        );
        assert_eq!(
            gssp["props"]["roundtrip"], "hello world",
            "base64 roundtrip: {gssp_json}"
        );
        assert_eq!(gssp["props"]["isBuffer"], true, "isBuffer: {gssp_json}");
        assert_eq!(gssp["props"]["notBuffer"], false, "notBuffer: {gssp_json}");
        assert_eq!(gssp["props"]["len"], 11, "length: {gssp_json}");
    }

    #[tokio::test]
    async fn test_integration_buffer_polyfill_import() {
        // Test importing Buffer from 'buffer' module
        let (_tmp, config, scan) = setup_test_project(
            &[(
                "index.tsx",
                r#"
                import { Buffer } from 'buffer';
                export default function Home(props) {
                    return <div>{props.result}</div>;
                }
                export function getServerSideProps() {
                    var buf = Buffer.from('SGVsbG8=', 'base64');
                    return { props: { result: buf.toString('utf8') }};
                }
                "#,
            )],
            None,
        );

        let (_result, pool) = build_and_load(&config, &scan).await;

        let gssp_json = pool
            .execute(|iso| iso.get_server_side_props("index", "{\"params\":{},\"query\":{}}"))
            .await
            .expect("pool execute")
            .expect("gssp");

        let gssp: serde_json::Value = serde_json::from_str(&gssp_json).unwrap();
        assert_eq!(
            gssp["props"]["result"], "Hello",
            "import from 'buffer' should work: {gssp_json}"
        );
    }

    #[tokio::test]
    async fn test_integration_buffer_alloc_concat() {
        // Test Buffer.alloc, Buffer.concat, and integer read/write methods
        let (_tmp, config, scan) = setup_test_project(
            &[(
                "index.tsx",
                r#"
                export default function Home(props) {
                    return <div>{JSON.stringify(props)}</div>;
                }
                export function getServerSideProps() {
                    var a = Buffer.from('foo');
                    var b = Buffer.from('bar');
                    var c = Buffer.concat([a, b]);

                    var alloc = Buffer.alloc(4);
                    alloc.writeUInt32BE(0xDEADBEEF, 0);
                    var readBack = alloc.readUInt32BE(0);

                    return { props: {
                        concat: c.toString('utf8'),
                        allocLen: alloc.length,
                        readBack: readBack,
                        byteLen: Buffer.byteLength('hello', 'utf8'),
                    }};
                }
                "#,
            )],
            None,
        );

        let (_result, pool) = build_and_load(&config, &scan).await;

        let gssp_json = pool
            .execute(|iso| iso.get_server_side_props("index", "{\"params\":{},\"query\":{}}"))
            .await
            .expect("pool execute")
            .expect("gssp");

        let gssp: serde_json::Value = serde_json::from_str(&gssp_json).unwrap();
        assert_eq!(gssp["props"]["concat"], "foobar", "concat: {gssp_json}");
        assert_eq!(gssp["props"]["allocLen"], 4, "alloc length: {gssp_json}");
        assert_eq!(
            gssp["props"]["readBack"], 0xDEADBEEFu64,
            "readBack: {gssp_json}"
        );
        assert_eq!(gssp["props"]["byteLen"], 5, "byteLength: {gssp_json}");
    }

    // ── detect_data_strategy_from_source tests ──────────────────

    #[test]
    fn test_detect_strategy_gssp() {
        let source = r#"
            import React from 'react';
            export default function Page() { return <div/>; }
            export function getServerSideProps(ctx) { return { props: {} }; }
        "#;
        assert_eq!(
            detect_data_strategy_from_source(source).unwrap(),
            DataStrategy::GetServerSideProps,
        );
    }

    #[test]
    fn test_detect_strategy_gssp_async() {
        let source = r#"
            export default function Page() { return <div/>; }
            export async function getServerSideProps(ctx) { return { props: {} }; }
        "#;
        assert_eq!(
            detect_data_strategy_from_source(source).unwrap(),
            DataStrategy::GetServerSideProps,
        );
    }

    #[test]
    fn test_detect_strategy_gsp() {
        let source = r#"
            export default function Page() { return <div/>; }
            export function getStaticProps() { return { props: {} }; }
        "#;
        assert_eq!(
            detect_data_strategy_from_source(source).unwrap(),
            DataStrategy::GetStaticProps,
        );
    }

    #[test]
    fn test_detect_strategy_none() {
        let source = r#"
            import React from 'react';
            export default function Page() { return <div>Static</div>; }
        "#;
        assert_eq!(
            detect_data_strategy_from_source(source).unwrap(),
            DataStrategy::None,
        );
    }

    #[test]
    fn test_detect_strategy_both_errors() {
        let source = r#"
            export default function Page() { return <div/>; }
            export function getServerSideProps() { return { props: {} }; }
            export function getStaticProps() { return { props: {} }; }
        "#;
        let err = detect_data_strategy_from_source(source).unwrap_err();
        assert!(
            err.to_string()
                .contains("both getStaticProps and getServerSideProps"),
            "expected dual-export error, got: {err}"
        );
    }

    #[test]
    fn test_detect_strategy_reexport_syntax() {
        let source = r#"
            export default function Page() { return <div/>; }
            export{ getServerSideProps } from './data';
        "#;
        assert_eq!(
            detect_data_strategy_from_source(source).unwrap(),
            DataStrategy::GetServerSideProps,
        );
    }

    // --- Tailwind CSS detection tests ---

    #[test]
    fn test_needs_tailwind_v4() {
        assert!(needs_tailwind("@import \"tailwindcss\";\n"));
        assert!(needs_tailwind("  @import \"tailwindcss\";\n"));
        assert!(needs_tailwind("@import 'tailwindcss';\n"));
    }

    #[test]
    fn test_needs_tailwind_v3() {
        assert!(needs_tailwind(
            "@tailwind base;\n@tailwind components;\n@tailwind utilities;\n"
        ));
        assert!(needs_tailwind("  @tailwind utilities;\n"));
    }

    #[test]
    fn test_needs_tailwind_negative() {
        assert!(!needs_tailwind("body { margin: 0; }\n"));
        assert!(!needs_tailwind(".container { max-width: 1200px; }\n"));
        assert!(!needs_tailwind("/* @import \"tailwindcss\" */\nbody {}\n"));
        assert!(!needs_tailwind(""));
    }

    #[test]
    #[ignore] // Requires tailwindcss CLI installed
    fn test_tailwind_processing() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();

        // Create styles dir
        let styles_dir = root.join("styles");
        fs::create_dir_all(&styles_dir).unwrap();

        // Write a Tailwind CSS file
        fs::write(styles_dir.join("globals.css"), "@import \"tailwindcss\";\n").unwrap();

        // Create pages with CSS import
        let pages_dir = root.join("pages");
        fs::create_dir_all(&pages_dir).unwrap();
        fs::write(
            pages_dir.join("_app.tsx"),
            "import '../styles/globals.css';\nexport default function App({ Component, pageProps }) { return <Component {...pageProps} />; }\n",
        )
        .unwrap();
        fs::write(
            pages_dir.join("index.tsx"),
            "export default function Home() { return <div className=\"p-4\">Hello</div>; }\n",
        )
        .unwrap();

        // Must have tailwindcss installed
        let bin = find_tailwind_bin(&root);
        if bin.is_none() {
            eprintln!("tailwindcss not found, skipping integration test");
            return;
        }

        let config = RexConfig::new(root).with_dev(false);
        let scan = rex_router::scan_pages(&config.pages_dir).unwrap();
        let output_dir = tmp.path().join("output");
        fs::create_dir_all(&output_dir).unwrap();

        let mappings = process_tailwind_css(&config, &scan, &output_dir).unwrap();
        assert!(
            !mappings.is_empty(),
            "should have processed at least one Tailwind file"
        );

        // The output file should exist and contain actual CSS (not just the directive)
        for output in mappings.values() {
            assert!(output.exists(), "Tailwind output file should exist");
            let content = fs::read_to_string(output).unwrap();
            assert!(
                !content.contains("@import \"tailwindcss\""),
                "should be compiled"
            );
            assert!(!content.is_empty(), "compiled CSS should not be empty");
        }
    }

    #[test]
    fn test_extract_middleware_matchers_array() {
        let source = r#"
export function middleware(request) {}

export const config = {
    matcher: ['/dashboard/:path*', '/api/admin/:path*']
}
"#;
        let matchers = extract_middleware_matchers(source);
        assert_eq!(matchers, vec!["/dashboard/:path*", "/api/admin/:path*"]);
    }

    #[test]
    fn test_extract_middleware_matchers_single_line() {
        let source = r#"
export function middleware(req) { return NextResponse.next(); }
export const config = { matcher: ['/protected'] }
"#;
        let matchers = extract_middleware_matchers(source);
        assert_eq!(matchers, vec!["/protected"]);
    }

    #[test]
    fn test_extract_middleware_matchers_no_config() {
        let source = r#"
export function middleware(request) {
    return NextResponse.next();
}
"#;
        let matchers = extract_middleware_matchers(source);
        assert!(matchers.is_empty());
    }

    #[test]
    fn test_extract_middleware_matchers_no_matcher() {
        let source = r#"
export function middleware(request) {}
export const config = { runtime: 'edge' }
"#;
        let matchers = extract_middleware_matchers(source);
        assert!(matchers.is_empty());
    }

    /// Test that a project with no package.json and no node_modules can still
    /// build using the embedded React packages (zero-config mode).
    #[tokio::test]
    async fn test_build_without_package_json() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();

        // NO setup_mock_node_modules, NO package.json — pure zero-config
        let pages_dir = root.join("pages");
        fs::create_dir_all(&pages_dir).unwrap();

        let index_path = pages_dir.join("index.tsx");
        fs::write(
            &index_path,
            "export default function Home() { return <div>Hello Zero Config</div>; }",
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
            middleware: None,
            app_scan: None,
            mcp_tools: vec![],
        };

        let result = build_bundles(&config, &scan, &ProjectConfig::default())
            .await
            .expect("build should succeed without package.json");

        // Server bundle should exist and contain React
        let bundle = fs::read_to_string(&result.server_bundle_path).unwrap();
        assert!(
            bundle.contains("__rex_render_page"),
            "should have render function"
        );
        assert!(bundle.contains("__rex_pages"), "should init page registry");

        // Client bundles should exist
        assert!(
            !result.manifest.pages.is_empty(),
            "should have page entries in manifest"
        );
    }
}
