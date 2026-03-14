use crate::build_utils::{find_route_for_chunk, route_to_chunk_name, runtime_client_dir};
use crate::css_collect::collect_css_files;
use crate::css_modules::CssModuleProcessing;
use crate::manifest::AssetManifest;
use crate::page_exports::{detect_data_strategy, detect_has_static_paths};
use anyhow::Result;
use rex_core::{ProjectConfig, RexConfig};
use rex_router::ScanResult;
use rolldown_common::Output;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Build client-side bundles using rolldown.
///
/// Rolldown handles the full pipeline: parsing TSX/JSX, resolving imports from
/// node_modules (including React), transforming, and code-splitting shared
/// dependencies into separate chunks. Output is ESM.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn build_client_bundles(
    config: &RexConfig,
    scan: &ScanResult,
    output_dir: &Path,
    build_id: &str,
    css_modules: &CssModuleProcessing,
    define: &[(String, String)],
    tailwind_outputs: &HashMap<PathBuf, PathBuf>,
    project_config: &ProjectConfig,
    module_dirs: &[String],
) -> Result<AssetManifest> {
    let mut manifest = AssetManifest::new(build_id.to_string());
    let hash = &build_id[..8];

    // Collect and copy CSS files referenced by source (rolldown doesn't bundle CSS)
    collect_css_files(
        scan,
        output_dir,
        build_id,
        &mut manifest,
        tailwind_outputs,
        &css_modules.page_overrides,
    )?;

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
        let entry_code = format!("import App from '{page_path}';\nwindow.__REX_APP__ = App;\n");
        let entry_path = entries_dir.join("_app.js");
        fs::write(&entry_path, &entry_code)?;
        inputs.push(rolldown::InputItem {
            name: Some("_app".to_string()),
            import: entry_path.to_string_lossy().to_string(),
        });
    }

    // Page entries (with server-export DCE)
    let dce_dir = output_dir.join("_dce");
    fs::create_dir_all(&dce_dir)?;

    for route in &scan.routes {
        let chunk_name = route_to_chunk_name(route);
        let effective_path = css_modules
            .page_overrides
            .get(&route.abs_path)
            .unwrap_or(&route.abs_path);

        // Apply dead code elimination: strip getServerSideProps/getStaticProps
        // and their server-only dependencies from the client copy.
        let page_path = match apply_dce_to_page(effective_path, &dce_dir, &chunk_name) {
            Ok(Some(dce_path)) => dce_path.to_string_lossy().replace('\\', "/"),
            _ => effective_path.to_string_lossy().replace('\\', "/"),
        };
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

    // Non-JS assets → empty/binary modules
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
    let mut client_aliases = vec![
        (
            "rex/link".to_string(),
            vec![Some(
                runtime_dir.join("link.ts").to_string_lossy().to_string(),
            )],
        ),
        (
            "rex/head".to_string(),
            vec![Some(
                runtime_dir.join("head.ts").to_string_lossy().to_string(),
            )],
        ),
        (
            "rex/router".to_string(),
            vec![Some(
                runtime_dir
                    .join("use-router.ts")
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
        // Next.js compatibility shims
        (
            "next/link".to_string(),
            vec![Some(
                runtime_dir.join("link.ts").to_string_lossy().to_string(),
            )],
        ),
        (
            "next/head".to_string(),
            vec![Some(
                runtime_dir.join("head.ts").to_string_lossy().to_string(),
            )],
        ),
        (
            "next/router".to_string(),
            vec![Some(
                runtime_dir
                    .join("use-router.ts")
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
    ];
    // Append user-defined aliases from rex.config build.alias
    client_aliases.extend(project_config.build.resolved_aliases(&config.project_root));

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
        define: Some(define.iter().cloned().collect()),
        tsconfig: Some(rolldown_common::TsConfig::Auto(true)),
        treeshake: crate::rsc_build_config::react_treeshake_options(),
        resolve: Some(rolldown::ResolveOptions {
            alias: Some(client_aliases),
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

    let output = bundler.write().await.map_err(|e| {
        anyhow::anyhow!(
            "Rolldown bundle failed:\n{}",
            crate::diagnostics::format_build_diagnostics(&e)
        )
    })?;

    // Process rolldown output: register entry chunks and shared chunks in the manifest
    for item in &output.assets {
        if let Output::Chunk(chunk) = item {
            if !chunk.is_entry {
                // Shared chunk (e.g. chunk-client, chunk-react) — track for modulepreload
                manifest.shared_chunks.push(chunk.filename.to_string());
                continue;
            }
            let name = chunk.name.to_string();
            let filename = chunk.filename.to_string();

            if name == "_app" {
                manifest.app_script = Some(filename);
            } else if let Some(route) = find_route_for_chunk(&name, &scan.routes) {
                let strategy = detect_data_strategy(&route.abs_path)?;
                let has_dynamic = !route.dynamic_segments.is_empty();
                // Check if this route has CSS module files to include
                if let Some(css_files) = css_modules.route_css.get(&route.pattern) {
                    manifest.add_page_with_css(
                        &route.pattern,
                        &filename,
                        css_files,
                        strategy,
                        has_dynamic,
                    );
                } else {
                    manifest.add_page(&route.pattern, &filename, strategy, has_dynamic);
                }
                // Detect getStaticPaths on dynamic routes
                let has_static_paths =
                    has_dynamic && detect_has_static_paths(&route.abs_path).unwrap_or(false);
                if has_static_paths {
                    if let Some(page) = manifest.pages.get_mut(&route.pattern) {
                        page.has_static_paths = true;
                    }
                }
            }
        }
    }

    let _ = fs::remove_dir_all(&entries_dir);
    let _ = fs::remove_dir_all(&dce_dir);

    debug!(
        pages = scan.routes.len(),
        "Client bundles built with rolldown"
    );
    Ok(manifest)
}

/// Apply DCE to a page source, stripping server-only exports.
/// Returns the path to the DCE'd file if any code was removed, or None if unchanged.
fn apply_dce_to_page(
    page_path: &Path,
    dce_dir: &Path,
    chunk_name: &str,
) -> Result<Option<PathBuf>> {
    let source = fs::read_to_string(page_path)?;

    let source_type = match page_path.extension().and_then(|e| e.to_str()) {
        Some("tsx") => oxc_span::SourceType::tsx(),
        Some("ts") => oxc_span::SourceType::ts(),
        Some("jsx") => oxc_span::SourceType::jsx(),
        _ => oxc_span::SourceType::mjs(),
    };

    let stripped = crate::dce::strip_server_exports(&source, source_type)?;
    if stripped.len() == source.len() && stripped == source {
        return Ok(None); // no changes
    }

    let ext = page_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("tsx");
    let dce_path = dce_dir.join(format!("{chunk_name}.{ext}"));
    fs::write(&dce_path, &stripped)?;
    debug!(page = %page_path.display(), "DCE stripped server exports for client bundle");
    Ok(Some(dce_path))
}
