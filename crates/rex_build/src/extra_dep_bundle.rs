//! Extra dependency bundling for ESM module loading.
//!
//! Bundles node_modules dependencies discovered during import graph walking
//! (e.g. payload, clsx, @payloadcms/*) that aren't covered by the pre-bundled
//! React deps in `server_dep_bundle`.
//!
//! Uses rolldown to produce self-contained ESM or IIFE bundles with proper
//! Node.js polyfill aliases and heavy package stubbing.

use crate::build_utils::{node_polyfill_aliases, runtime_server_dir};
use anyhow::Result;
use rex_core::RexConfig;
use std::sync::Arc;
use tracing::debug;

/// Common rolldown configuration for extra dep bundling.
struct DepBundleConfig {
    aliases: Vec<(String, Vec<Option<String>>)>,
    module_types: rustc_hash::FxHashMap<String, rolldown::ModuleType>,
    define_env: &'static str,
    plugins: Vec<Arc<dyn rolldown::plugin::Pluginable>>,
}

fn build_config(config: &RexConfig) -> Result<DepBundleConfig> {
    let runtime_dir = runtime_server_dir()?;
    let mut aliases: Vec<(String, Vec<Option<String>>)> = Vec::new();
    aliases.extend(node_polyfill_aliases(&runtime_dir));

    let make_alias = |spec: &str, file: &str| {
        (
            spec.to_string(),
            vec![Some(runtime_dir.join(file).to_string_lossy().to_string())],
        )
    };
    for (s, f) in &[
        ("rex/head", "head.ts"),
        ("rex/link", "link.ts"),
        ("rex/router", "router.ts"),
        ("rex/document", "document.ts"),
        ("rex/image", "image.ts"),
        ("rex/middleware", "middleware.ts"),
    ] {
        aliases.push(make_alias(s, f));
    }

    let mut module_types = rustc_hash::FxHashMap::default();
    for ext in &[".css", ".scss", ".sass", ".less", ".mdx", ".svg", ".wasm"] {
        module_types.insert((*ext).to_string(), rolldown::ModuleType::Empty);
    }
    for ext in &[
        ".png", ".jpg", ".jpeg", ".gif", ".webp", ".ico", ".woff", ".woff2", ".ttf", ".eot",
    ] {
        module_types.insert((*ext).to_string(), rolldown::ModuleType::Binary);
    }

    let define_env = if config.dev {
        "\"development\""
    } else {
        "\"production\""
    };

    let empty_stub = runtime_dir.join("empty.ts").to_string_lossy().to_string();
    let polyfill_plugin: Arc<dyn rolldown::plugin::Pluginable> =
        Arc::new(crate::server_bundle::NodePolyfillResolvePlugin::new(
            vec![
                (
                    "file-type".to_string(),
                    runtime_dir
                        .join("file-type.ts")
                        .to_string_lossy()
                        .to_string(),
                ),
                ("@vercel/og".to_string(), empty_stub.clone()),
            ],
            empty_stub,
        ));

    // Stub heavy server-only packages that can't run in V8 (same list as
    // the RSC server IIFE bundle).
    let heavy_stub_plugin: Arc<dyn rolldown::plugin::Pluginable> =
        Arc::new(crate::server_bundle::HeavyPackageStubPlugin::new(vec![
            "node_modules/@aws-sdk/".to_string(),
            "node_modules/@smithy/".to_string(),
            "node_modules/pg-native/".to_string(),
            "node_modules/@node-rs/".to_string(),
            "node_modules/undici/".to_string(),
        ]));

    Ok(DepBundleConfig {
        aliases,
        module_types,
        define_env,
        plugins: vec![polyfill_plugin, heavy_stub_plugin],
    })
}

/// Bundle extra deps as ESM, returning ALL chunks (entry + code-split) as
/// `(specifier, source)` pairs. The entry gets the dep's specifier; chunks
/// get relative specifiers (e.g., `./chunk-abc.js`) so V8 can resolve them.
///
/// No polyfill banner is included — these modules are loaded into an existing
/// V8 context that already has polyfills evaluated.
pub async fn build_dep_esm_with_chunks(
    config: &RexConfig,
    entry_source: &str,
    name: &str,
    condition_names: &[&str],
    module_dirs: &[String],
) -> Result<Vec<(String, String)>> {
    let sanitized_name = name.replace(['/', '-', '.', '@'], "_");
    let output_dir = config
        .server_build_dir()
        .join("_dep_bundles")
        .join(&sanitized_name);
    std::fs::create_dir_all(&output_dir)?;

    let entry_path = output_dir.join(format!("{sanitized_name}-entry.js"));
    std::fs::write(&entry_path, entry_source)?;

    let bc = build_config(config)?;

    let options = rolldown::BundlerOptions {
        input: Some(vec![rolldown::InputItem {
            name: Some(sanitized_name.clone()),
            import: entry_path.to_string_lossy().to_string(),
        }]),
        cwd: Some(config.project_root.clone()),
        format: Some(rolldown::OutputFormat::Esm),
        dir: Some(output_dir.to_string_lossy().to_string()),
        entry_filenames: Some(format!("{sanitized_name}.js").into()),
        platform: Some(rolldown::Platform::Browser),
        module_types: Some(bc.module_types),
        define: Some(
            [(
                "process.env.NODE_ENV".to_string(),
                bc.define_env.to_string(),
            )]
            .into_iter()
            .collect(),
        ),
        tsconfig: Some(rolldown_common::TsConfig::Auto(false)),
        shim_missing_exports: Some(true),
        treeshake: crate::rsc_build_config::react_treeshake_options(),
        resolve: Some(rolldown::ResolveOptions {
            alias: Some(bc.aliases),
            condition_names: Some(condition_names.iter().map(|s| s.to_string()).collect()),
            extensions: Some(vec![
                ".tsx".to_string(),
                ".ts".to_string(),
                ".jsx".to_string(),
                ".js".to_string(),
            ]),
            modules: Some(module_dirs.to_vec()),
            ..Default::default()
        }),
        ..Default::default()
    };

    let mut bundler = rolldown::Bundler::with_plugins(options, bc.plugins)
        .map_err(|e| anyhow::anyhow!("Failed to create dep bundler: {e}"))?;

    if let Err(e) = bundler.write().await {
        if !crate::diagnostics::is_all_missing_exports(&e) {
            return Err(anyhow::anyhow!(
                "Dep bundle ({name}) failed:\n{}",
                crate::diagnostics::format_build_diagnostics(&e)
            ));
        }
    }

    // Collect all produced .js files: entry gets the dep specifier,
    // chunks get their filename as specifier (for relative import resolution).
    let mut results = Vec::new();
    let entry_file = format!("{sanitized_name}.js");
    if let Ok(entries) = std::fs::read_dir(&output_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("js") && path != entry_path {
                let filename = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let content = std::fs::read_to_string(&path)?;
                let specifier = if filename == entry_file {
                    name.to_string()
                } else {
                    format!("./{filename}")
                };
                debug!(
                    specifier = %specifier,
                    size = content.len(),
                    "Dep chunk"
                );
                results.push((specifier, content));
            }
        }
    }

    let _ = std::fs::remove_dir_all(&output_dir);
    Ok(results)
}
