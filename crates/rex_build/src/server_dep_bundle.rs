//! Server dependency pre-bundling for ESM module loading.
//!
//! Produces self-contained IIFEs that bundle React and friends, evaluated once
//! as V8 scripts. The IIFE sets globals (`globalThis.__rex_deps_*`) that the
//! ESM module registry wraps as synthetic modules.
//!
//! Two variants:
//! - **Pages dep IIFE**: React (standard conditions) + renderToString + polyfills
//! - **RSC flight dep IIFE**: React (react-server condition) + renderToReadableStream
//!   + registerServerReference + decodeReply/decodeAction + polyfills + webpack shims

use crate::build_utils::{node_polyfill_aliases, runtime_server_dir};
use crate::server_bundle::V8_POLYFILLS;
use anyhow::Result;
use rex_core::RexConfig;
use std::sync::Arc;
use tracing::debug;

/// Result of server dep pre-bundling.
pub struct ServerDepBundle {
    /// Pages dep IIFE: React + renderToString + V8 polyfills.
    /// Evaluated as a script, sets `globalThis.__rex_deps`.
    pub pages_iife: String,
    /// RSC flight dep IIFE: React (react-server) + flight protocol + polyfills + webpack shims.
    /// Evaluated as a script, sets `globalThis.__rex_flight_deps`.
    /// `None` if the project has no app/ directory.
    pub flight_iife: Option<String>,
}

/// Build the pages dep IIFE.
///
/// Entry bundles React + react-dom/server with standard Node.js conditions,
/// exposing them on `globalThis.__rex_deps` for synthetic module wrapping.
pub async fn build_pages_dep_iife(config: &RexConfig, module_dirs: &[String]) -> Result<String> {
    let entry_source = r#"
import * as React from 'react';
import * as ReactJSXRuntime from 'react/jsx-runtime';
import * as ReactJSXDevRuntime from 'react/jsx-dev-runtime';
import { renderToString } from 'react-dom/server';

globalThis.__rex_deps = {
    react: React,
    "react/jsx-runtime": ReactJSXRuntime,
    "react/jsx-dev-runtime": ReactJSXDevRuntime,
    "react-dom/server": { renderToString },
};

// Also set individual globals for backwards compat with existing runtime
globalThis.__rex_React = React;
globalThis.__rex_renderToString = renderToString;
"#;

    build_dep_iife(
        config,
        entry_source,
        "pages-deps",
        &["require", "default"],
        V8_POLYFILLS,
        module_dirs,
    )
    .await
}

/// Build the RSC flight dep IIFE.
///
/// Entry bundles React with `react-server` condition + flight protocol APIs,
/// exposing them on `globalThis.__rex_flight_deps` for synthetic module wrapping.
pub async fn build_flight_dep_iife(config: &RexConfig, module_dirs: &[String]) -> Result<String> {
    let webpack_shims = include_str!("../../../runtime/rsc/webpack-shims.ts");
    let banner = format!("{}\n{}", V8_POLYFILLS, webpack_shims);

    let entry_source = r#"
import * as React from 'react';
import * as ReactJSXRuntime from 'react/jsx-runtime';
import * as ReactJSXDevRuntime from 'react/jsx-dev-runtime';
import { renderToReadableStream } from 'react-server-dom-webpack/server';
import { registerServerReference, decodeReply, decodeAction } from 'react-server-dom-webpack/server';
import { createElement } from 'react';
import { renderToString } from 'react-dom/server';
import { createFromReadableStream } from 'react-server-dom-webpack/client';

globalThis.__rex_flight_deps = {
    react: React,
    "react/jsx-runtime": ReactJSXRuntime,
    "react/jsx-dev-runtime": ReactJSXDevRuntime,
    "react-server-dom-webpack/server": {
        renderToReadableStream,
        registerServerReference,
        decodeReply,
        decodeAction,
    },
    "react-dom/server": { renderToString },
    "react-server-dom-webpack/client": { createFromReadableStream },
};

// Individual globals used by the flight runtime
globalThis.__rex_createElement = createElement;
globalThis.__rex_renderToReadableStream = renderToReadableStream;
globalThis.__rex_renderToString = renderToString;
globalThis.__rex_createFromReadableStream = createFromReadableStream;
globalThis.__rex_registerServerReference = registerServerReference;
globalThis.__rex_decodeReply = decodeReply;
globalThis.__rex_decodeAction = decodeAction;
"#;

    build_dep_iife(
        config,
        entry_source,
        "flight-deps",
        &["react-server", "workerd", "browser", "import", "default"],
        &banner,
        module_dirs,
    )
    .await
}

/// Build both dep IIFEs (pages + optional flight).
pub async fn build_server_dep_bundles(
    config: &RexConfig,
    has_app_dir: bool,
    module_dirs: &[String],
) -> Result<ServerDepBundle> {
    let pages_iife = build_pages_dep_iife(config, module_dirs).await?;

    let flight_iife = if has_app_dir {
        Some(build_flight_dep_iife(config, module_dirs).await?)
    } else {
        None
    };

    Ok(ServerDepBundle {
        pages_iife,
        flight_iife,
    })
}

/// Internal: build a dep IIFE using rolldown.
async fn build_dep_iife(
    config: &RexConfig,
    entry_source: &str,
    name: &str,
    condition_names: &[&str],
    banner: &str,
    module_dirs: &[String],
) -> Result<String> {
    let output_dir = config.server_build_dir().join("_dep_bundles");
    std::fs::create_dir_all(&output_dir)?;

    // Write entry to temp file
    let entry_path = output_dir.join(format!("{name}-entry.js"));
    std::fs::write(&entry_path, entry_source)?;

    // Build resolve aliases
    let runtime_dir = runtime_server_dir()?;
    let mut aliases: Vec<(String, Vec<Option<String>>)> = Vec::new();
    aliases.extend(node_polyfill_aliases(&runtime_dir));

    // Rex built-in aliases for server
    let make_alias = |spec: &str, file: &str| {
        (
            spec.to_string(),
            vec![Some(runtime_dir.join(file).to_string_lossy().to_string())],
        )
    };
    let rex_aliases = [
        ("rex/head", "head.ts"),
        ("rex/link", "link.ts"),
        ("rex/router", "router.ts"),
        ("rex/document", "document.ts"),
        ("rex/image", "image.ts"),
        ("rex/middleware", "middleware.ts"),
    ];
    for (s, f) in &rex_aliases {
        aliases.push(make_alias(s, f));
    }

    // Non-JS asset module types
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

    let options = rolldown::BundlerOptions {
        input: Some(vec![rolldown::InputItem {
            name: Some(name.to_string()),
            import: entry_path.to_string_lossy().to_string(),
        }]),
        cwd: Some(config.project_root.clone()),
        format: Some(rolldown::OutputFormat::Iife),
        dir: Some(output_dir.to_string_lossy().to_string()),
        entry_filenames: Some(format!("{name}.js").into()),
        platform: Some(rolldown::Platform::Browser),
        module_types: Some(module_types),
        define: Some(
            [("process.env.NODE_ENV".to_string(), define_env.to_string())]
                .into_iter()
                .collect(),
        ),
        banner: Some(rolldown::AddonOutputOption::String(Some(
            banner.to_string(),
        ))),
        tsconfig: Some(rolldown_common::TsConfig::Auto(false)),
        shim_missing_exports: Some(true),
        treeshake: crate::rsc_build_config::react_treeshake_options(),
        resolve: Some(rolldown::ResolveOptions {
            alias: Some(aliases),
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

    let mut bundler = rolldown::Bundler::with_plugins(options, vec![polyfill_plugin])
        .map_err(|e| anyhow::anyhow!("Failed to create dep bundler: {e}"))?;

    if let Err(e) = bundler.write().await {
        if !crate::diagnostics::is_all_missing_exports(&e) {
            return Err(anyhow::anyhow!(
                "Dep bundle ({name}) failed:\n{}",
                crate::diagnostics::format_build_diagnostics(&e)
            ));
        }
    }

    let bundle_path = output_dir.join(format!("{name}.js"));
    let content = std::fs::read_to_string(&bundle_path)?;

    // Clean up temp files
    let _ = std::fs::remove_file(&entry_path);
    let _ = std::fs::remove_file(&bundle_path);

    debug!(name, size = content.len(), "Dep IIFE built");
    Ok(content)
}
