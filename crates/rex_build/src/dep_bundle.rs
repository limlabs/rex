use crate::build_utils::{node_polyfill_aliases, runtime_server_dir};
use crate::server_bundle::{NodePolyfillResolvePlugin, V8_POLYFILLS};
use anyhow::Result;
use rex_core::RexConfig;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tracing::debug;

/// Pre-bundled dependencies for unbundled dev mode.
pub struct DepBundle {
    /// Server IIFE: React + react-dom/server + polyfills, sets globals.
    pub server_iife: Arc<String>,
    /// Client dependency ESM files: bare specifier → JS content.
    pub client_deps: HashMap<String, String>,
    /// Import map JSON for the browser: `{"imports": {"react": "/_rex/deps/react.js", ...}}`
    pub import_map: serde_json::Value,
}

/// Pre-bundle dependencies at dev startup.
///
/// Uses rolldown to bundle React + polyfills into:
/// 1. A server IIFE that sets `globalThis.__rex_React`, `__rex_renderToString`, etc.
/// 2. Client ESM files served at `/_rex/deps/` with an import map.
pub async fn prebundle_deps(config: &RexConfig, module_dirs: &[String]) -> Result<DepBundle> {
    let runtime_dir = runtime_server_dir()?;
    let output_dir = config.output_dir.join("_dep_bundle");
    std::fs::create_dir_all(&output_dir)?;

    let server_iife = bundle_server_deps(config, &runtime_dir, &output_dir, module_dirs).await?;
    let (client_deps, import_map) =
        bundle_client_deps(config, &runtime_dir, &output_dir, module_dirs).await?;

    Ok(DepBundle {
        server_iife: Arc::new(server_iife),
        client_deps,
        import_map,
    })
}

/// Bundle server dependencies into a single IIFE.
///
/// Sets globals:
/// - `globalThis.__rex_React` → React
/// - `globalThis.__rex_renderToString` → renderToString
/// - V8 polyfills (process, MessageChannel, TextEncoder, etc.)
async fn bundle_server_deps(
    config: &RexConfig,
    runtime_dir: &Path,
    output_dir: &Path,
    module_dirs: &[String],
) -> Result<String> {
    let entries_dir = output_dir.join("_server_dep_entry");
    std::fs::create_dir_all(&entries_dir)?;

    // Virtual entry that imports React and exports to globals
    let entry_code = r#"
import React, { createElement } from 'react';
import { renderToString } from 'react-dom/server';
import 'rex/head';

globalThis.__rex_React = React;
globalThis.__rex_createElement = createElement;
globalThis.__rex_renderToString = renderToString;
"#;

    let entry_path = entries_dir.join("server-deps.js");
    std::fs::write(&entry_path, entry_code)?;

    // Rex built-in aliases
    let rex_aliases = [
        ("rex/head", "head.ts"),
        ("rex/link", "link.ts"),
        ("rex/router", "router.ts"),
        ("rex/document", "document.ts"),
        ("rex/image", "image.ts"),
        ("rex/middleware", "middleware.ts"),
        ("next/document", "document.ts"),
    ];
    let make_alias = |spec: &str, file: &str| {
        (
            spec.to_string(),
            vec![Some(runtime_dir.join(file).to_string_lossy().to_string())],
        )
    };
    let mut aliases: Vec<_> = rex_aliases.iter().map(|(s, f)| make_alias(s, f)).collect();
    aliases.extend(node_polyfill_aliases(runtime_dir));

    let mut module_types = rustc_hash::FxHashMap::default();
    for ext in &[".css", ".scss", ".sass", ".less", ".mdx", ".svg", ".wasm"] {
        module_types.insert((*ext).to_string(), rolldown::ModuleType::Empty);
    }
    for ext in &[
        ".png", ".jpg", ".jpeg", ".gif", ".webp", ".ico", ".woff", ".woff2", ".ttf", ".eot",
    ] {
        module_types.insert((*ext).to_string(), rolldown::ModuleType::Binary);
    }

    let options = rolldown::BundlerOptions {
        input: Some(vec![rolldown::InputItem {
            name: Some("server-deps".to_string()),
            import: entry_path.to_string_lossy().to_string(),
        }]),
        cwd: Some(config.project_root.clone()),
        format: Some(rolldown::OutputFormat::Iife),
        dir: Some(output_dir.to_string_lossy().to_string()),
        entry_filenames: Some("server-deps.js".to_string().into()),
        platform: Some(rolldown::Platform::Browser),
        module_types: Some(module_types),
        banner: Some(rolldown::AddonOutputOption::String(Some(
            V8_POLYFILLS.to_string(),
        ))),
        tsconfig: Some(rolldown_common::TsConfig::Auto(false)),
        shim_missing_exports: Some(true),
        treeshake: crate::rsc_build_config::react_treeshake_options(),
        resolve: Some(rolldown::ResolveOptions {
            alias: Some(aliases),
            condition_names: Some(vec!["require".to_string(), "default".to_string()]),
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

    let empty_stub = runtime_dir.join("empty.ts").to_string_lossy().to_string();
    let polyfill_plugin = Arc::new(NodePolyfillResolvePlugin::new(vec![], empty_stub));

    let mut bundler = rolldown::Bundler::with_plugins(
        options,
        vec![polyfill_plugin as Arc<dyn rolldown::plugin::Pluginable>],
    )
    .map_err(|e| anyhow::anyhow!("Failed to create dep bundler: {e}"))?;

    if let Err(e) = bundler.write().await {
        let all_missing = e.iter().all(|d| format!("{d:?}").contains("MissingExport"));
        if !all_missing {
            return Err(anyhow::anyhow!("Server dep bundle failed: {:?}", e));
        }
    }

    let bundle_path = output_dir.join("server-deps.js");
    let bundle_js = std::fs::read_to_string(&bundle_path)?;

    // Cleanup
    let _ = std::fs::remove_dir_all(&entries_dir);
    let _ = std::fs::remove_file(&bundle_path);

    debug!("Server deps bundled ({} bytes)", bundle_js.len());
    Ok(bundle_js)
}

/// Bundle client dependencies into individual ESM files.
///
/// Returns (client_deps, import_map) where:
/// - client_deps maps bare specifiers to ESM JS content
/// - import_map is the browser import map JSON
async fn bundle_client_deps(
    config: &RexConfig,
    _runtime_dir: &Path,
    output_dir: &Path,
    module_dirs: &[String],
) -> Result<(HashMap<String, String>, serde_json::Value)> {
    let entries_dir = output_dir.join("_client_dep_entries");
    std::fs::create_dir_all(&entries_dir)?;
    let client_output = output_dir.join("_client_deps");
    std::fs::create_dir_all(&client_output)?;

    // Create entry files for each dep we want to pre-bundle
    let deps = vec![
        (
            "react",
            "export * from 'react'; export { default } from 'react';",
        ),
        (
            "react-dom-client",
            "export * from 'react-dom/client'; export { default } from 'react-dom/client';",
        ),
        ("react-jsx-runtime", "export * from 'react/jsx-runtime';"),
    ];

    let mut inputs = Vec::new();
    for (name, code) in &deps {
        let entry_path = entries_dir.join(format!("{name}.js"));
        std::fs::write(&entry_path, code)?;
        inputs.push(rolldown::InputItem {
            name: Some(name.to_string()),
            import: entry_path.to_string_lossy().to_string(),
        });
    }

    // Client-side rex aliases: point to client runtime stubs
    let client_runtime_dir = crate::build_utils::runtime_client_dir()?;
    let make_client_alias = |spec: &str, file: &str| {
        (
            spec.to_string(),
            vec![Some(
                client_runtime_dir.join(file).to_string_lossy().to_string(),
            )],
        )
    };
    let mut aliases: Vec<_> = vec![
        make_client_alias("rex/head", "head.ts"),
        make_client_alias("rex/link", "link.ts"),
    ];
    // Node polyfills not needed on client — just need the rex aliases
    // Add next/* shims that exist in client runtime
    for (spec, file) in [
        ("next/link", "next-link.ts"),
        ("next/image", "next-image.ts"),
    ] {
        if client_runtime_dir.join(file).exists() {
            aliases.push(make_client_alias(spec, file));
        }
    }

    let mut module_types = rustc_hash::FxHashMap::default();
    for ext in &[".css", ".scss", ".sass", ".less"] {
        module_types.insert((*ext).to_string(), rolldown::ModuleType::Empty);
    }

    let options = rolldown::BundlerOptions {
        input: Some(inputs),
        cwd: Some(config.project_root.clone()),
        format: Some(rolldown::OutputFormat::Esm),
        dir: Some(client_output.to_string_lossy().to_string()),
        platform: Some(rolldown::Platform::Browser),
        module_types: Some(module_types),
        // Default code splitting for ESM output
        tsconfig: Some(rolldown_common::TsConfig::Auto(false)),
        shim_missing_exports: Some(true),
        resolve: Some(rolldown::ResolveOptions {
            alias: Some(aliases),
            condition_names: Some(vec![
                "browser".to_string(),
                "module".to_string(),
                "import".to_string(),
                "default".to_string(),
            ]),
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

    let mut bundler = rolldown::Bundler::new(options)
        .map_err(|e| anyhow::anyhow!("Failed to create client dep bundler: {e}"))?;

    if let Err(e) = bundler.write().await {
        let all_missing = e.iter().all(|d| format!("{d:?}").contains("MissingExport"));
        if !all_missing {
            return Err(anyhow::anyhow!("Client dep bundle failed: {:?}", e));
        }
    }

    // Read output files and build the dep map + import map
    let mut client_deps = HashMap::new();
    let mut imports = serde_json::Map::new();

    for entry in std::fs::read_dir(&client_output)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("js") {
            let filename = path
                .file_name()
                .expect("file_name is present for read_dir entries")
                .to_string_lossy()
                .to_string();
            let content = std::fs::read_to_string(&path)?;
            let dep_url = format!("/_rex/deps/{filename}");
            client_deps.insert(filename.clone(), content);

            // Map bare specifiers to dep URLs
            if filename.starts_with("react.") || filename == "react.js" {
                imports.insert(
                    "react".to_string(),
                    serde_json::Value::String(dep_url.clone()),
                );
                imports.insert(
                    "react/".to_string(),
                    serde_json::Value::String("/_rex/deps/".to_string()),
                );
            } else if filename.starts_with("react-dom-client") {
                imports.insert(
                    "react-dom/client".to_string(),
                    serde_json::Value::String(dep_url),
                );
            } else if filename.starts_with("react-jsx-runtime") {
                imports.insert(
                    "react/jsx-runtime".to_string(),
                    serde_json::Value::String(dep_url),
                );
            }
        }
    }

    // Add rex alias mappings to import map
    imports.insert(
        "rex/head".to_string(),
        serde_json::Value::String("/_rex/dev/@rex/head.ts".to_string()),
    );
    imports.insert(
        "rex/link".to_string(),
        serde_json::Value::String("/_rex/dev/@rex/link.ts".to_string()),
    );
    imports.insert(
        "rex/router".to_string(),
        serde_json::Value::String("/_rex/dev/@rex/router.ts".to_string()),
    );

    let import_map = serde_json::json!({ "imports": imports });

    // Cleanup entry dir
    let _ = std::fs::remove_dir_all(&entries_dir);

    debug!(deps = client_deps.len(), "Client deps bundled");
    Ok((client_deps, import_map))
}
