//! RSC bundle builder.
//!
//! Produces three bundles from an app/ directory scan:
//! 1. **Flight bundle** (IIFE, `react-server` condition): Contains all server components.
//!    At `"use client"` boundaries, imports are replaced with client reference stubs.
//!    Uses `renderToReadableStream` from `react-server-dom-webpack/server`.
//! 2. **SSR bundle** (IIFE, standard conditions): Contains `createFromReadableStream`
//!    and `renderToString` for converting flight data to HTML. Also includes client
//!    components for SSR rendering.
//! 3. **Client bundle** (ESM): Contains only `"use client"` components and their
//!    dependencies, with code splitting.
//!
//! Also produces a `ClientReferenceManifest` mapping reference IDs to chunk URLs.

use crate::bundler::runtime_client_dir;
use crate::client_manifest::{client_reference_id, ClientReferenceManifest};
use crate::rsc_graph::{analyze_module_graph, ModuleGraph};
use anyhow::Result;
use rex_core::app_route::AppScanResult;
use rex_core::RexConfig;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Result of the RSC bundle build.
#[derive(Debug)]
pub struct RscBuildResult {
    /// Path to the server RSC flight bundle (IIFE, `react-server` condition).
    pub server_bundle_path: PathBuf,
    /// Path to the SSR bundle (IIFE, standard conditions).
    pub ssr_bundle_path: PathBuf,
    /// Client reference manifest mapping ref IDs to chunk URLs.
    pub client_manifest: ClientReferenceManifest,
    /// Client chunk files produced (relative paths from client output dir).
    pub client_chunks: Vec<String>,
}

/// Build RSC bundles for an app/ directory.
///
/// This is called from `build_bundles` when an `AppScanResult` is present.
pub async fn build_rsc_bundles(
    config: &RexConfig,
    app_scan: &AppScanResult,
    build_id: &str,
    define: &[(String, String)],
) -> Result<RscBuildResult> {
    let project_root = config.project_root.canonicalize().unwrap_or_else(|e| {
        debug!(
            path = %config.project_root.display(),
            error = %e,
            "Failed to canonicalize project root, using original path"
        );
        config.project_root.clone()
    });

    let server_dir = config.server_build_dir().join("rsc");
    let client_dir = config.client_build_dir().join("rsc");
    fs::create_dir_all(&server_dir)?;
    fs::create_dir_all(&client_dir)?;

    // Collect all entry points from the app scan
    let mut entries: Vec<PathBuf> = Vec::new();
    entries.push(app_scan.root_layout.clone());
    for route in &app_scan.routes {
        entries.push(route.page_path.clone());
        entries.extend(route.layout_chain.iter().cloned());
    }
    entries.sort();
    entries.dedup();

    // Analyze the module graph
    let graph = analyze_module_graph(&entries, &config.project_root)?;

    // Generate client reference stubs for "use client" modules
    let stubs_dir = server_dir.join("_client_stubs");
    fs::create_dir_all(&stubs_dir)?;

    let client_boundaries = graph.client_boundary_modules();
    let mut stub_aliases: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut client_manifest = ClientReferenceManifest::new();

    for module in &client_boundaries {
        let rel_path = module
            .path
            .strip_prefix(&project_root)
            .unwrap_or(&module.path)
            .to_string_lossy()
            .replace('\\', "/");

        // Generate stub file with client reference objects
        let stub_source = generate_client_stub(&rel_path, &module.exports, build_id);
        let stub_name = sanitize_filename(&rel_path);
        let stub_path = stubs_dir.join(format!("{stub_name}.js"));
        fs::write(&stub_path, &stub_source)?;

        // Map original module path → stub path for rolldown aliases
        stub_aliases.push((module.path.clone(), stub_path));

        // Register in manifest (chunk URLs filled in after client build)
        for export in &module.exports {
            let ref_id = client_reference_id(&rel_path, export, build_id);
            // Placeholder chunk URL — updated after client bundle build
            client_manifest.add(&ref_id, String::new(), export.clone());
        }
    }

    // Build rex/* → stub aliases for client boundaries discovered via rex/* imports.
    // The stub_aliases map absolute paths, but rolldown also needs the specifier alias
    // (e.g. "rex/link" → stub) for when source code uses `import Link from 'rex/link'`.
    let pkg_src = project_root.join("node_modules/@limlabs/rex/src");
    let rex_client_specifiers = ["link", "head", "router", "image"];
    for name in &rex_client_specifiers {
        let specifier = format!("rex/{name}");
        for ext in &["tsx", "ts", "jsx", "js"] {
            let candidate = pkg_src.join(format!("{name}.{ext}"));
            if candidate.exists() {
                if let Ok(canonical) = candidate.canonicalize() {
                    // If this file is a client boundary (has a stub), add specifier → stub alias
                    if let Some((_orig, stub)) = stub_aliases.iter().find(|(p, _)| *p == canonical)
                    {
                        stub_aliases.push((PathBuf::from(&specifier), stub.clone()));
                    }
                    break;
                }
            }
        }
    }

    // Build client bundles first so manifest is populated before server bundle
    let client_chunks = build_rsc_client_bundles(
        config,
        &graph,
        &client_dir,
        build_id,
        define,
        &mut client_manifest,
    )
    .await?;

    // Build server RSC flight bundle (after client build so manifest is populated)
    let server_bundle_path = build_rsc_server_bundle(
        config,
        app_scan,
        &graph,
        &server_dir,
        &stub_aliases,
        define,
        &client_manifest,
    )
    .await?;

    // Build SSR bundle (after client build so manifest is populated)
    let ssr_bundle_path = build_rsc_ssr_bundle(
        config,
        &graph,
        &server_dir,
        build_id,
        define,
        &client_manifest,
    )
    .await?;

    // Clean up stubs
    let _ = fs::remove_dir_all(&stubs_dir);

    Ok(RscBuildResult {
        server_bundle_path,
        ssr_bundle_path,
        client_manifest,
        client_chunks,
    })
}

/// Generate a client reference stub module for a `"use client"` component.
///
/// For each export, produces:
/// ```js
/// export const Foo = { $$typeof: Symbol.for("react.client.reference"), $$id: "<refId>", $$name: "Foo" };
/// ```
fn generate_client_stub(rel_path: &str, exports: &[String], build_id: &str) -> String {
    let mut source = String::new();
    source.push_str("// Auto-generated client reference stub\n");

    for export in exports {
        let ref_id = client_reference_id(rel_path, export, build_id);
        let obj = format!(
            "{{ $$typeof: Symbol.for(\"react.client.reference\"), $$id: \"{ref_id}\", $$name: \"{export}\" }}"
        );

        if export == "default" {
            source.push_str(&format!("export default {obj};\n"));
        } else {
            source.push_str(&format!("export const {export} = {obj};\n"));
        }
    }

    source
}

/// Build the server RSC flight bundle (IIFE, `react-server` condition).
///
/// This bundle includes all server components, with `"use client"` modules
/// replaced by reference stubs via rolldown aliases.
/// Uses `renderToReadableStream` from `react-server-dom-webpack/server`.
async fn build_rsc_server_bundle(
    config: &RexConfig,
    app_scan: &AppScanResult,
    _graph: &ModuleGraph,
    output_dir: &Path,
    stub_aliases: &[(PathBuf, PathBuf)],
    define: &[(String, String)],
    client_manifest: &ClientReferenceManifest,
) -> Result<PathBuf> {
    let entries_dir = output_dir.join("_rsc_server_entry");
    fs::create_dir_all(&entries_dir)?;

    let mut entry = String::new();

    // React imports — resolved with react-server condition
    entry.push_str("import { createElement } from 'react';\n");
    entry.push_str("import { renderToReadableStream } from 'react-server-dom-webpack/server';\n");
    entry.push_str("var __rex_createElement = createElement;\n");
    entry.push_str("var __rex_renderToReadableStream = renderToReadableStream;\n\n");

    // Register layouts
    entry.push_str("globalThis.__rex_app_layouts = {};\n");
    for (i, route) in app_scan.routes.iter().enumerate() {
        for (j, layout) in route.layout_chain.iter().enumerate() {
            let layout_path = layout.to_string_lossy().replace('\\', "/");
            let layout_key = format!("layout_{i}_{j}");
            entry.push_str(&format!(
                "import {{ default as {layout_key} }} from '{layout_path}';\n"
            ));
        }
    }

    // Register pages
    entry.push_str("\nglobalThis.__rex_app_pages = {};\n");
    for (i, route) in app_scan.routes.iter().enumerate() {
        let page_path = route.page_path.to_string_lossy().replace('\\', "/");
        let page_var = format!("__app_page_{i}");
        entry.push_str(&format!(
            "import {{ default as {page_var} }} from '{page_path}';\n"
        ));
        let pattern = &route.pattern;
        entry.push_str(&format!(
            "globalThis.__rex_app_pages[\"{pattern}\"] = {page_var};\n"
        ));
    }

    // Register layout chains per route
    entry.push_str("\nglobalThis.__rex_app_layout_chains = {};\n");
    for (i, route) in app_scan.routes.iter().enumerate() {
        let layout_vars: Vec<String> = (0..route.layout_chain.len())
            .map(|j| format!("layout_{i}_{j}"))
            .collect();
        let array = format!("[{}]", layout_vars.join(", "));
        let pattern = &route.pattern;
        entry.push_str(&format!(
            "globalThis.__rex_app_layout_chains[\"{pattern}\"] = {array};\n"
        ));
    }

    // Server-side webpack bundler config for renderToReadableStream
    let bundler_config_json = serde_json::to_string(&client_manifest.to_server_webpack_config())
        .unwrap_or_else(|_| "{}".to_string());
    entry.push_str(&format!(
        "\nglobalThis.__rex_webpack_bundler_config = {bundler_config_json};\n"
    ));

    // RSC runtime: flight protocol using React's renderToReadableStream
    let flight_runtime = include_str!("../../../runtime/rsc/flight.js");
    entry.push_str("\n// --- RSC Flight Runtime ---\n");
    entry.push_str(flight_runtime);

    let entry_path = entries_dir.join("rsc-server-entry.js");
    fs::write(&entry_path, &entry)?;

    // CSS → empty module
    let mut module_types = rustc_hash::FxHashMap::default();
    module_types.insert(".css".to_string(), rolldown::ModuleType::Empty);

    // Build aliases: map "use client" modules to their stubs
    let mut aliases: Vec<(String, Vec<Option<String>>)> = Vec::new();
    for (original, stub) in stub_aliases {
        aliases.push((
            original.to_string_lossy().to_string(),
            vec![Some(stub.to_string_lossy().to_string())],
        ));
    }

    // V8 polyfills + webpack shims as banner
    let webpack_shims = include_str!("../../../runtime/rsc/webpack-shims.js");
    let banner = format!("{}\n{}", crate::bundler::V8_POLYFILLS, webpack_shims);

    // Minify production builds
    let minify = if !config.dev {
        Some(rolldown_common::RawMinifyOptions::Bool(true))
    } else {
        None
    };

    let options = rolldown::BundlerOptions {
        input: Some(vec![rolldown::InputItem {
            name: Some("rsc-server-bundle".to_string()),
            import: entry_path.to_string_lossy().to_string(),
        }]),
        cwd: Some(config.project_root.clone()),
        format: Some(rolldown::OutputFormat::Iife),
        dir: Some(output_dir.to_string_lossy().to_string()),
        entry_filenames: Some("rsc-server-bundle.js".to_string().into()),
        platform: Some(rolldown::Platform::Browser),
        module_types: Some(module_types),
        define: Some(define.iter().cloned().collect()),
        banner: Some(rolldown::AddonOutputOption::String(Some(banner))),
        // Disable tsconfig auto-resolution for the RSC server bundle.
        // tsconfig `paths` (e.g. "rex/*") would override our stub aliases,
        // resolving to the real module instead of the client reference stub.
        // The entry uses absolute paths, so tsconfig isn't needed here.
        tsconfig: Some(rolldown_common::TsConfig::Auto(false)),
        minify,
        treeshake: react_treeshake_options(),
        resolve: Some(rolldown::ResolveOptions {
            alias: Some(aliases),
            condition_names: Some(vec![
                "react-server".to_string(),
                "browser".to_string(),
                "import".to_string(),
                "default".to_string(),
            ]),
            extensions: Some(vec![
                ".tsx".to_string(),
                ".ts".to_string(),
                ".jsx".to_string(),
                ".js".to_string(),
            ]),
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
        .map_err(|e| anyhow::anyhow!("Failed to create RSC server bundler: {e}"))?;

    bundler
        .write()
        .await
        .map_err(|e| anyhow::anyhow!("RSC server bundle failed: {e:?}"))?;

    let _ = fs::remove_dir_all(&entries_dir);

    let bundle_path = output_dir.join("rsc-server-bundle.js");
    debug!(path = %bundle_path.display(), "RSC server bundle written");
    Ok(bundle_path)
}

/// Build the SSR bundle (IIFE, standard conditions).
///
/// This bundle consumes flight data and produces HTML using:
/// - `createFromReadableStream` from `react-server-dom-webpack/client`
/// - `renderToString` from `react-dom/server`
///
/// It also includes all `"use client"` components for SSR rendering.
async fn build_rsc_ssr_bundle(
    config: &RexConfig,
    graph: &ModuleGraph,
    output_dir: &Path,
    build_id: &str,
    define: &[(String, String)],
    client_manifest: &ClientReferenceManifest,
) -> Result<PathBuf> {
    let project_root = config.project_root.canonicalize().unwrap_or_else(|e| {
        debug!(
            path = %config.project_root.display(),
            error = %e,
            "Failed to canonicalize project root, using original path"
        );
        config.project_root.clone()
    });

    let entries_dir = output_dir.join("_rsc_ssr_entry");
    fs::create_dir_all(&entries_dir)?;

    let mut entry = String::new();

    // React imports — standard (non-react-server) conditions
    entry.push_str("import { createElement } from 'react';\n");
    entry.push_str("import { renderToString } from 'react-dom/server';\n");
    entry.push_str("import { createFromReadableStream } from 'react-server-dom-webpack/client';\n");
    entry.push_str("var __rex_createElement = createElement;\n");
    entry.push_str("var __rex_renderToString = renderToString;\n");
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
            .strip_prefix(&project_root)
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
    let ssr_runtime = include_str!("../../../runtime/rsc/ssr-pass.js");
    entry.push_str("\n// --- RSC SSR Pass Runtime ---\n");
    entry.push_str(ssr_runtime);

    let entry_path = entries_dir.join("rsc-ssr-entry.js");
    fs::write(&entry_path, &entry)?;

    // CSS → empty module
    let mut module_types = rustc_hash::FxHashMap::default();
    module_types.insert(".css".to_string(), rolldown::ModuleType::Empty);

    // V8 polyfills + webpack shims as banner
    let webpack_shims = include_str!("../../../runtime/rsc/webpack-shims.js");
    let banner = format!("{}\n{}", crate::bundler::V8_POLYFILLS, webpack_shims);

    // Minify production builds
    let minify = if !config.dev {
        Some(rolldown_common::RawMinifyOptions::Bool(true))
    } else {
        None
    };

    // Rex built-in aliases for SSR bundle (rex/link → client runtime, etc.)
    let ssr_aliases = build_rex_aliases()?;

    let options = rolldown::BundlerOptions {
        input: Some(vec![rolldown::InputItem {
            name: Some("rsc-ssr-bundle".to_string()),
            import: entry_path.to_string_lossy().to_string(),
        }]),
        cwd: Some(config.project_root.clone()),
        format: Some(rolldown::OutputFormat::Iife),
        dir: Some(output_dir.to_string_lossy().to_string()),
        entry_filenames: Some("rsc-ssr-bundle.js".to_string().into()),
        platform: Some(rolldown::Platform::Browser),
        module_types: Some(module_types),
        define: Some(define.iter().cloned().collect()),
        banner: Some(rolldown::AddonOutputOption::String(Some(banner))),
        tsconfig: Some(rolldown_common::TsConfig::Auto(true)),
        minify,
        treeshake: react_treeshake_options(),
        resolve: Some(rolldown::ResolveOptions {
            alias: Some(ssr_aliases),
            condition_names: Some(vec![
                "browser".to_string(),
                "import".to_string(),
                "default".to_string(),
            ]),
            extensions: Some(vec![
                ".tsx".to_string(),
                ".ts".to_string(),
                ".jsx".to_string(),
                ".js".to_string(),
            ]),
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
        .map_err(|e| anyhow::anyhow!("Failed to create RSC SSR bundler: {e}"))?;

    bundler
        .write()
        .await
        .map_err(|e| anyhow::anyhow!("RSC SSR bundle failed: {e:?}"))?;

    let _ = fs::remove_dir_all(&entries_dir);

    let bundle_path = output_dir.join("rsc-ssr-bundle.js");
    debug!(path = %bundle_path.display(), "RSC SSR bundle written");
    Ok(bundle_path)
}

/// Build client bundles for `"use client"` modules.
///
/// Each client boundary module becomes a separate entry. Rolldown handles
/// code splitting so shared dependencies (React) become shared chunks.
async fn build_rsc_client_bundles(
    config: &RexConfig,
    graph: &ModuleGraph,
    output_dir: &Path,
    build_id: &str,
    define: &[(String, String)],
    client_manifest: &mut ClientReferenceManifest,
) -> Result<Vec<String>> {
    let client_boundaries = graph.client_boundary_modules();
    if client_boundaries.is_empty() {
        return Ok(vec![]);
    }

    let hash = &build_id[..8.min(build_id.len())];

    // Create temporary entry files for rolldown.
    // The hydrate entry uses react-server-dom-webpack/client to parse React's
    // flight format and hydrate the SSR'd HTML with interactive client components.
    let entries_dir = output_dir.join("_rsc_client_entries");
    fs::create_dir_all(&entries_dir)?;

    let hydrate_code = include_str!("../../../runtime/client/rsc-hydrate.js");
    let hydrate_path = entries_dir.join("__rsc_hydrate.js");
    fs::write(&hydrate_path, hydrate_code)?;

    // Create entries: hydrate entry + each client boundary module
    let mut entries: Vec<rolldown::InputItem> = vec![rolldown::InputItem {
        name: Some("__rsc_hydrate".to_string()),
        import: hydrate_path.to_string_lossy().to_string(),
    }];

    entries.extend(client_boundaries.iter().map(|m| {
        let rel_path = m.path.strip_prefix(&config.project_root).unwrap_or(&m.path);
        let name = sanitize_filename(&rel_path.to_string_lossy());
        rolldown::InputItem {
            name: Some(name),
            import: m.path.to_string_lossy().to_string(),
        }
    }));

    // CSS → empty module
    let mut module_types = rustc_hash::FxHashMap::default();
    module_types.insert(".css".to_string(), rolldown::ModuleType::Empty);

    // Minify production builds
    let minify = if !config.dev {
        Some(rolldown_common::RawMinifyOptions::Bool(true))
    } else {
        None
    };

    // Rex built-in aliases for client bundle (rex/link → client runtime, etc.)
    let client_aliases = build_rex_aliases()?;

    // Split React packages into cacheable vendor chunks:
    // 1. react-server-dom-webpack (flight client) — changes rarely, cached independently
    // 2. react + react-dom (core React) — changes rarely, shared across pages
    // Without manual splitting, rolldown inlines everything into __rsc_hydrate
    // because only the hydrate entry imports them (automatic splitting needs 2+ consumers).
    let rsc_flight_group = rolldown_common::MatchGroup {
        name: rolldown_common::MatchGroupName::Static("rsc-flight".to_string()),
        test: Some(rolldown_common::MatchGroupTest::Regex(
            rolldown_utils::js_regex::HybridRegex::new(
                "node_modules[\\\\/]react-server-dom-webpack",
            )
            .expect("valid regex"),
        )),
        priority: Some(20),
        ..Default::default()
    };
    let react_vendor_group = rolldown_common::MatchGroup {
        name: rolldown_common::MatchGroupName::Static("react-vendor".to_string()),
        test: Some(rolldown_common::MatchGroupTest::Regex(
            rolldown_utils::js_regex::HybridRegex::new("node_modules[\\\\/]react")
                .expect("valid regex"),
        )),
        priority: Some(10),
        ..Default::default()
    };

    let options = rolldown::BundlerOptions {
        input: Some(entries),
        cwd: Some(config.project_root.clone()),
        format: Some(rolldown::OutputFormat::Esm),
        dir: Some(output_dir.to_string_lossy().to_string()),
        entry_filenames: Some(format!("[name]-{hash}.js").into()),
        chunk_filenames: Some(format!("chunk-[name]-{hash}.js").into()),
        platform: Some(rolldown::Platform::Browser),
        module_types: Some(module_types),
        define: Some(define.iter().cloned().collect()),
        tsconfig: Some(rolldown_common::TsConfig::Auto(true)),
        minify,
        treeshake: react_treeshake_options(),
        manual_code_splitting: Some(rolldown_common::ManualCodeSplittingOptions {
            groups: Some(vec![rsc_flight_group, react_vendor_group]),
            ..Default::default()
        }),
        resolve: Some(rolldown::ResolveOptions {
            alias: Some(client_aliases),
            extensions: Some(vec![
                ".tsx".to_string(),
                ".ts".to_string(),
                ".jsx".to_string(),
                ".js".to_string(),
            ]),
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
        .map_err(|e| anyhow::anyhow!("Failed to create RSC client bundler: {e}"))?;

    let output = bundler
        .write()
        .await
        .map_err(|e| anyhow::anyhow!("RSC client bundle failed: {e:?}"))?;

    // Clean up temporary entry files
    let _ = fs::remove_dir_all(&entries_dir);

    // Collect output chunks and update the client manifest.
    // Prefix with "rsc/" because the output dir is client_build_dir/rsc/
    // and the static file server mounts client_build_dir at /_rex/static/.
    //
    // The bootstrap chunk must come first so React/ReactDOM globals are set
    // before component chunks (loaded as type="module") execute.
    let mut bootstrap_chunks = Vec::new();
    let mut component_chunks = Vec::new();

    // Pre-compute boundary lookup: sanitized_name → (rel_path, &exports)
    let boundary_lookup: std::collections::HashMap<String, (String, &[String])> = client_boundaries
        .iter()
        .map(|module| {
            let rel_path = module
                .path
                .strip_prefix(&config.project_root)
                .unwrap_or(&module.path)
                .to_string_lossy()
                .replace('\\', "/");
            let module_name = sanitize_filename(&rel_path);
            (module_name, (rel_path, module.exports.as_slice()))
        })
        .collect();

    for item in &output.assets {
        match item {
            rolldown_common::Output::Chunk(chunk) => {
                let filename = chunk.filename.to_string();
                let prefixed = format!("rsc/{filename}");

                if chunk.is_entry {
                    let name = chunk.name.to_string();

                    if name == "__rsc_hydrate" || name == "__rsc_bootstrap" {
                        // Hydrate/bootstrap entry — must load first
                        bootstrap_chunks.push(prefixed);
                    } else {
                        // Client component entry — update manifest via O(1) lookup
                        component_chunks.push(prefixed);
                        if let Some((rel_path, exports)) = boundary_lookup.get(&name) {
                            let chunk_url = format!("/_rex/static/rsc/{filename}");
                            for export in *exports {
                                let ref_id = client_reference_id(rel_path, export, build_id);
                                client_manifest.add(&ref_id, chunk_url.clone(), export.clone());
                            }
                        }
                    }
                } else {
                    // Shared chunks (code-split React, etc.)
                    component_chunks.push(prefixed);
                }
            }
            rolldown_common::Output::Asset(asset) => {
                component_chunks.push(format!("rsc/{}", asset.filename));
            }
        }
    }

    // Bootstrap first, then shared chunks, then component entries
    let mut chunks = bootstrap_chunks;
    chunks.extend(component_chunks);

    debug!(
        count = chunks.len(),
        "RSC client bundles written to {}",
        output_dir.display()
    );
    Ok(chunks)
}

/// Build rolldown resolve aliases for `rex/*` built-in imports.
///
/// Maps `rex/link`, `rex/head`, `rex/router`, `rex/image` to their
/// corresponding runtime files in `runtime/client/`.
fn build_rex_aliases() -> Result<Vec<(String, Vec<Option<String>>)>> {
    let client_dir = runtime_client_dir()?;
    let mut aliases = Vec::new();

    let mappings = [
        ("rex/link", "link"),
        ("rex/head", "head"),
        ("rex/router", "use-router"),
        ("rex/image", "image"),
    ];

    for (specifier, file_stem) in &mappings {
        for ext in &["ts", "tsx", "js", "jsx"] {
            let candidate = client_dir.join(format!("{file_stem}.{ext}"));
            if candidate.exists() {
                aliases.push((
                    specifier.to_string(),
                    vec![Some(candidate.to_string_lossy().to_string())],
                ));
                break;
            }
        }
    }

    Ok(aliases)
}

/// Tree-shake options that mark React packages as side-effect-free.
///
/// Allows rolldown to aggressively eliminate unused exports from
/// `node_modules/react*`. React's production builds use `@__PURE__`
/// annotations which rolldown respects when `annotations: true`.
pub(crate) fn react_treeshake_options() -> rolldown_common::TreeshakeOptions {
    rolldown_common::TreeshakeOptions::Option(rolldown_common::InnerOptions {
        module_side_effects: rolldown_common::ModuleSideEffects::Rules(vec![
            // React packages are side-effect-free in production
            rolldown_common::ModuleSideEffectsRule {
                test: Some(
                    rolldown_utils::js_regex::HybridRegex::new("node_modules[\\\\/]react")
                        .expect("valid regex"),
                ),
                external: None,
                side_effects: false,
            },
        ]),
        annotations: Some(true),
        ..Default::default()
    })
}

/// Sanitize a file path into a valid chunk name.
fn sanitize_filename(path: &str) -> String {
    path.replace(['/', '\\', '.'], "_")
        .trim_matches('_')
        .to_string()
}

// The RSC runtime is now loaded from `runtime/rsc/flight.js` via `include_str!`.

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rex_core::app_route::{AppRoute, AppScanResult, AppSegment};

    fn setup_rsc_mock_node_modules(root: &Path) {
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
            "export function renderToString(el) { return '<div></div>'; }\n",
        )
        .unwrap();
    }

    #[tokio::test]
    async fn test_rsc_build_produces_bundles() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();

        setup_rsc_mock_node_modules(&root);

        // Create app directory with layout + page
        let app_dir = root.join("app");
        fs::create_dir_all(&app_dir).unwrap();

        let layout_path = app_dir.join("layout.tsx");
        fs::write(
            &layout_path,
            "export default function RootLayout({ children }) { return children; }\n",
        )
        .unwrap();

        let page_path = app_dir.join("page.tsx");
        fs::write(
            &page_path,
            "export default function Home() { return 'Hello'; }\n",
        )
        .unwrap();

        // Create a "use client" component
        let comp_dir = root.join("components");
        fs::create_dir_all(&comp_dir).unwrap();
        let counter_path = comp_dir.join("Counter.tsx");
        fs::write(
            &counter_path,
            "\"use client\";\nexport default function Counter() { return 'count'; }\n",
        )
        .unwrap();

        let config = rex_core::RexConfig::new(root.clone()).with_dev(true);

        let app_scan = AppScanResult {
            root: AppSegment {
                segment: String::new(),
                layout: Some(layout_path.clone()),
                page: Some(page_path.clone()),
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
            }],
            root_layout: layout_path,
        };

        let define = vec![(
            "process.env.NODE_ENV".to_string(),
            "\"development\"".to_string(),
        )];

        let result = build_rsc_bundles(&config, &app_scan, "test-build-id", &define)
            .await
            .expect("build_rsc_bundles should succeed");

        // Server bundle file exists
        assert!(
            result.server_bundle_path.exists(),
            "Server bundle should exist at {:?}",
            result.server_bundle_path
        );

        // Server bundle is non-empty
        let server_content = fs::read_to_string(&result.server_bundle_path).unwrap();
        assert!(
            !server_content.is_empty(),
            "Server bundle should not be empty"
        );

        // SSR bundle file exists
        assert!(
            result.ssr_bundle_path.exists(),
            "SSR bundle should exist at {:?}",
            result.ssr_bundle_path
        );

        // SSR bundle is non-empty
        let ssr_content = fs::read_to_string(&result.ssr_bundle_path).unwrap();
        assert!(!ssr_content.is_empty(), "SSR bundle should not be empty");

        // Client manifest was created (may be empty if no "use client" modules in entries)
        // Verify the manifest struct exists and is accessible
        let _ = &result.client_manifest.entries;
    }

    #[test]
    fn generate_stub_default_export() {
        let stub = generate_client_stub("components/Counter.tsx", &["default".to_string()], "abc");
        assert!(stub.contains("export default"));
        assert!(stub.contains("react.client.reference"));
        assert!(stub.contains("$$name: \"default\""));
    }

    #[test]
    fn generate_stub_named_exports() {
        let stub = generate_client_stub(
            "utils.tsx",
            &["Counter".to_string(), "Input".to_string()],
            "abc",
        );
        assert!(stub.contains("export const Counter"));
        assert!(stub.contains("export const Input"));
        assert!(!stub.contains("export default"));
    }

    #[test]
    fn sanitize_path() {
        assert_eq!(
            sanitize_filename("components/Counter.tsx"),
            "components_Counter_tsx"
        );
        assert_eq!(sanitize_filename("app/page.tsx"), "app_page_tsx");
    }
}
