use crate::build_utils::{extract_middleware_matchers, generate_build_id, runtime_server_dir};
use crate::client_bundle::build_client_bundles;
use crate::css_modules::process_css_modules;
use crate::font::process_fonts;
use crate::manifest::AssetManifest;
use crate::server_bundle::build_server_bundle;
use crate::tailwind::process_tailwind_css;
use anyhow::Result;
use rex_core::{ProjectConfig, RexConfig};
use rex_router::ScanResult;
use std::fs;
use std::path::Path;
use tracing::{debug, info, info_span, Instrument};

/// App route handler runtime — compiled from `runtime/server/app-route-runtime.ts` at build time.
/// Used in both full server bundles (via server_bundle.rs) and minimal bundles (app-only projects).
const APP_ROUTE_RUNTIME: &str = include_str!(concat!(env!("OUT_DIR"), "/app-route-runtime.js"));

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
pub fn resolve_modules_dirs(config: &RexConfig) -> Result<Vec<String>> {
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

    // Pre-process MDX pages and CSS modules (generates compiled JSX, scoped CSS + JS proxy files)
    let css_modules = process_css_modules(scan, &client_dir, &build_id, &config.project_root)?;

    // Pre-process font imports (downloads Google Fonts, generates @font-face CSS)
    let font_result = process_fonts(
        scan,
        &client_dir,
        &build_id,
        &config.project_root,
        &css_modules.page_overrides,
    )
    .await?;

    // Merge font page overrides into CSS module overrides
    let mut merged_page_overrides = css_modules.page_overrides.clone();
    merged_page_overrides.extend(font_result.page_overrides);

    // Create a merged CssModuleProcessing with font overrides folded in
    let css_modules_merged = crate::css_modules::CssModuleProcessing {
        page_overrides: merged_page_overrides.clone(),
        route_css: css_modules.route_css.clone(),
        global_css: css_modules.global_css.clone(),
    };

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
            &merged_page_overrides,
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
            &css_modules_merged,
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

    // Add font CSS and preloads to manifest
    if !font_result.font_css.is_empty() {
        let font_css_filename = format!("fonts-{}.css", &build_id[..8]);
        let font_css_path = client_dir.join(&font_css_filename);
        fs::write(&font_css_path, &font_result.font_css)?;
        manifest
            .css_contents
            .insert(font_css_filename.clone(), font_result.font_css);
        manifest.global_css.insert(0, font_css_filename);
        manifest.font_preloads = font_result.font_preloads;
    }

    // Set middleware matchers on manifest (if middleware exists)
    if let Some(mw_path) = &scan.middleware {
        let source = fs::read_to_string(mw_path)?;
        manifest.middleware_matchers = Some(extract_middleware_matchers(&source));
    }

    // Build RSC bundles if app/ scan is present
    if let Some(app_scan) = &scan.app_scan {
        // Pre-process any .mdx pages/layouts in the app router
        let app_scan = &crate::mdx::process_mdx_app_pages(
            app_scan,
            &config.server_build_dir(),
            &config.project_root,
        )?;

        // Pre-process font imports in app router layout/page files
        let app_font_result = crate::font::process_font_app_pages(
            app_scan,
            &client_dir,
            &build_id,
            &config.project_root,
        )
        .await?;
        let app_scan = &app_font_result.app_scan;

        // Add app router font CSS and preloads to manifest
        if !app_font_result.font_css.is_empty() {
            let font_css_filename = format!("app-fonts-{}.css", &build_id[..8]);
            let font_css_path = client_dir.join(&font_css_filename);
            fs::write(&font_css_path, &app_font_result.font_css)?;
            manifest
                .css_contents
                .insert(font_css_filename.clone(), app_font_result.font_css);
            manifest.global_css.insert(0, font_css_filename);
            manifest.font_preloads.extend(app_font_result.font_preloads);
        }

        let rsc_result =
            crate::rsc_bundler::build_rsc_bundles(config, app_scan, &build_id, &define).await?;

        // Populate app_routes in manifest with automatic static optimization
        for route in &app_scan.routes {
            let has_dynamic_segments = !route.dynamic_segments.is_empty();

            // Check if any server component in this route's tree uses dynamic functions
            let mut route_entries: Vec<std::path::PathBuf> = Vec::new();
            route_entries.push(route.page_path.clone());
            route_entries.extend(route.layout_chain.iter().cloned());
            // Canonicalize paths to match the module graph keys
            let canonical_entries: Vec<std::path::PathBuf> = route_entries
                .iter()
                .filter_map(|p| p.canonicalize().ok())
                .collect();
            let uses_dynamic = rsc_result
                .module_graph
                .has_dynamic_functions(&canonical_entries);

            let render_mode = if has_dynamic_segments || uses_dynamic {
                rex_core::RenderMode::ServerRendered
            } else {
                rex_core::RenderMode::Static
            };

            manifest.app_routes.insert(
                route.pattern.clone(),
                crate::manifest::AppRouteAssets {
                    client_chunks: rsc_result.client_chunks.clone(),
                    layout_chain: route
                        .layout_chain
                        .iter()
                        .map(|p| p.to_string_lossy().to_string())
                        .collect(),
                    render_mode,
                },
            );
        }

        manifest.client_reference_manifest = Some(rsc_result.client_manifest);
        manifest.rsc_server_bundle =
            Some(rsc_result.server_bundle_path.to_string_lossy().to_string());
        manifest.rsc_ssr_bundle = Some(rsc_result.ssr_bundle_path.to_string_lossy().to_string());

        // Expose server action IDs so clients can discover them
        for (action_id, entry) in &rsc_result.server_action_manifest.actions {
            manifest
                .server_actions
                .insert(action_id.clone(), entry.export_name.clone());
        }

        // Collect CSS imports from app/ layout and page files into global_css.
        // In the app router, layout CSS applies to all child routes, so we treat
        // it as global. Tailwind outputs (already compiled above) are used when available.
        {
            let hash = &build_id[..8];
            let mut seen_files = std::collections::HashSet::new();
            for route in &app_scan.routes {
                let mut source_files = vec![route.page_path.clone()];
                source_files.extend(route.layout_chain.iter().cloned());
                for src in source_files {
                    if !seen_files.insert(src.clone()) {
                        continue;
                    }
                    let css_paths = match crate::css_collect::extract_css_imports(&src) {
                        Ok(p) => p,
                        Err(_) => continue,
                    };
                    for css_path in css_paths {
                        if !css_path.exists() {
                            continue;
                        }
                        let filename = crate::css_collect::css_output_filename(&css_path, hash);
                        if manifest.css_contents.contains_key(&filename) {
                            // Already collected (e.g. by font processing)
                            continue;
                        }
                        let dest = client_dir.join(&filename);
                        if let Some(tw_output) = tailwind_outputs.get(&css_path) {
                            let content = fs::read_to_string(tw_output)?;
                            fs::write(&dest, &content)?;
                            manifest.css_contents.insert(filename.clone(), content);
                        } else {
                            let content = fs::read_to_string(&css_path)?;
                            fs::write(&dest, &content)?;
                            manifest.css_contents.insert(filename.clone(), content);
                        }
                        manifest.global_css.push(filename);
                    }
                }
            }
        }

        debug!(app_routes = manifest.app_routes.len(), "RSC bundles built");
    }

    // Save manifest
    manifest.save(&config.manifest_path())?;

    // Clean up temp dirs from pre-processing
    let css_modules_dir = client_dir.join("_css_modules");
    let _ = fs::remove_dir_all(&css_modules_dir);
    let fonts_dir = client_dir.join("_fonts");
    let _ = fs::remove_dir_all(&fonts_dir);
    let mdx_dir = client_dir.join("_mdx");
    let _ = fs::remove_dir_all(&mdx_dir);
    let server_mdx_dir = server_dir.join("_mdx");
    let _ = fs::remove_dir_all(&server_mdx_dir);

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
            // Add the app route handler runtime (method dispatch, async support)
            entry.push_str(APP_ROUTE_RUNTIME);
        }
    }

    let entry_path = entry_dir.join("server-entry.js");
    fs::write(&entry_path, entry)?;

    let runtime_dir = runtime_server_dir()?;
    let mut module_types = rustc_hash::FxHashMap::default();
    for ext in &[".css", ".scss", ".sass", ".less", ".mdx", ".svg"] {
        module_types.insert((*ext).to_string(), rolldown::ModuleType::Empty);
    }
    for ext in &[
        ".png", ".jpg", ".jpeg", ".gif", ".webp", ".ico", ".woff", ".woff2", ".ttf", ".eot",
    ] {
        module_types.insert((*ext).to_string(), rolldown::ModuleType::Binary);
    }

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
