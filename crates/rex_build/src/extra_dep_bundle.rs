//! Extra dependency bundling for ESM module loading.
//!
//! Bundles node_modules dependencies discovered during import graph walking
//! (e.g. payload, clsx, @payloadcms/*) that aren't covered by the pre-bundled
//! React deps in `server_dep_bundle`.
//!
//! Uses rolldown multi-entry bundling to produce native ESM modules. Each dep
//! is a separate entry point; rolldown code-splits shared code into chunks.
//! All outputs (entries + chunks) are loaded directly as V8 ESM modules,
//! preserving class hierarchies and live bindings.

use crate::build_utils::{node_polyfill_aliases, runtime_server_dir};
use crate::esm_transform::DepImport;
use anyhow::Result;
use rex_core::RexConfig;
use std::collections::HashMap;
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
            "node_modules/pg/".to_string(),
            "node_modules/pg-pool/".to_string(),
            "node_modules/pg-cloudflare/".to_string(),
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

/// Result of multi-entry dep bundling.
pub struct ExtraDepBundleResult {
    /// All ESM modules: (specifier, source). Specifiers use `/_rex_deps/` prefix.
    pub modules: Vec<(String, String)>,
    /// Alias mappings: (bare_specifier, path_specifier). The bare specifier
    /// (e.g., "payload") should resolve to the same V8 module as the path
    /// specifier (e.g., "/_rex_deps/payload.js").
    pub aliases: Vec<(String, String)>,
}

/// Bundle all extra deps as native ESM using rolldown multi-entry bundling.
///
/// Each dep becomes its own entry point. Rolldown code-splits shared code into
/// chunks. All modules use `/_rex_deps/` path-based specifiers so relative
/// imports between chunks resolve correctly. Entry modules also get bare-specifier
/// aliases (e.g., "payload" → "/_rex_deps/payload.js").
pub async fn build_extra_deps_multi_entry(
    config: &RexConfig,
    deps: &[DepImport],
    module_dirs: &[String],
    externals: &[String],
) -> Result<ExtraDepBundleResult> {
    let output_dir = config
        .server_build_dir()
        .join("_dep_bundles")
        .join("_extra");
    std::fs::create_dir_all(&output_dir)?;

    // Write a virtual entry file per dep that re-exports everything.
    let mut inputs = Vec::new();
    let mut entry_name_to_specifier: HashMap<String, String> = HashMap::new();

    for dep in deps {
        let safe_name = dep.specifier.replace(['/', '-', '.', '@'], "_");
        let mut entry_source = String::new();

        // Re-export pattern: import then re-export to preserve all bindings
        if dep.has_default && dep.named_exports.is_empty() {
            entry_source.push_str(&format!("export {{ default }} from '{}';\n", dep.specifier));
        } else if dep.has_default && !dep.named_exports.is_empty() {
            entry_source.push_str(&format!("export {{ default }} from '{}';\n", dep.specifier));
            let names: Vec<&String> = dep.named_exports.iter().collect();
            entry_source.push_str(&format!(
                "export {{ {} }} from '{}';\n",
                names
                    .iter()
                    .map(|n| n.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                dep.specifier
            ));
        } else if !dep.named_exports.is_empty() {
            let names: Vec<&String> = dep.named_exports.iter().collect();
            entry_source.push_str(&format!(
                "export {{ {} }} from '{}';\n",
                names
                    .iter()
                    .map(|n| n.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                dep.specifier
            ));
        } else {
            // No specific imports known — re-export everything
            entry_source.push_str(&format!("export * from '{}';\n", dep.specifier));
            entry_source.push_str(&format!("export {{ default }} from '{}';\n", dep.specifier));
        }

        let entry_path = output_dir.join(format!("{safe_name}-entry.js"));
        std::fs::write(&entry_path, &entry_source)?;

        inputs.push(rolldown::InputItem {
            name: Some(safe_name.clone()),
            import: entry_path.to_string_lossy().to_string(),
        });
        entry_name_to_specifier.insert(safe_name, dep.specifier.clone());
    }

    let bc = build_config(config)?;

    // Mark pre-bundled deps as external so rolldown doesn't re-bundle them.
    // They're already loaded as separate V8 ESM modules (React, etc.).
    let external = if externals.is_empty() {
        None
    } else {
        Some(rolldown_common::IsExternal::from(externals.to_vec()))
    };

    let options = rolldown::BundlerOptions {
        input: Some(inputs),
        cwd: Some(config.project_root.clone()),
        format: Some(rolldown::OutputFormat::Esm),
        dir: Some(output_dir.to_string_lossy().to_string()),
        platform: Some(rolldown::Platform::Browser),
        module_types: Some(bc.module_types),
        external,
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
            condition_names: Some(
                ["default", "import", "module"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            ),
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
        .map_err(|e| anyhow::anyhow!("Failed to create multi-entry dep bundler: {e}"))?;

    if let Err(e) = bundler.write().await {
        if !crate::diagnostics::is_all_missing_exports(&e) {
            return Err(anyhow::anyhow!(
                "Multi-entry dep bundle failed:\n{}",
                crate::diagnostics::format_build_diagnostics(&e)
            ));
        }
    }

    // Collect all .js files from output directory.
    // ALL modules use `/_rex_deps/` path-based specifiers.
    // Entry modules also get alias mappings from bare specifier → path specifier.
    let mut modules = Vec::new();
    let mut aliases = Vec::new();
    let entry_files: HashMap<String, String> = entry_name_to_specifier
        .iter()
        .map(|(safe, spec)| (format!("{safe}.js"), spec.clone()))
        .collect();

    if let Ok(dir_entries) = std::fs::read_dir(&output_dir) {
        for dir_entry in dir_entries.flatten() {
            let path = dir_entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("js") {
                continue;
            }
            let filename = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if filename.ends_with("-entry.js") {
                continue;
            }

            let content = std::fs::read_to_string(&path)?;
            let path_specifier = format!("/_rex_deps/{filename}");

            debug!(
                specifier = %path_specifier,
                size = content.len(),
                "Extra dep module"
            );
            modules.push((path_specifier.clone(), content));

            // Record alias for entry files: bare specifier → path specifier
            if let Some(dep_specifier) = entry_files.get(&filename) {
                debug!(
                    bare = %dep_specifier,
                    path = %path_specifier,
                    "Extra dep alias"
                );
                aliases.push((dep_specifier.clone(), path_specifier));
            }
        }
    }

    let _ = std::fs::remove_dir_all(&output_dir);
    Ok(ExtraDepBundleResult { modules, aliases })
}
