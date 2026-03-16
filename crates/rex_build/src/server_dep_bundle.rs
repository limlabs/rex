//! Server dependency pre-bundling for ESM module loading.
//!
//! Bundles React and friends into self-contained ESM modules using rolldown.
//! Each dep is resolved with the correct conditions (standard or react-server)
//! and output as ESM so it can be loaded directly into V8's module registry.
//!
//! Dep modules are grouped by condition:
//! - **Standard**: `react`, `react/jsx-runtime`, `react-dom/server` (renderToString)
//! - **RSC flight** (react-server condition): `react-server-dom-webpack/server`
//!   (renderToReadableStream), `react-server-dom-webpack/client` (createFromReadableStream)

use crate::build_utils::{node_polyfill_aliases, runtime_server_dir};
use crate::server_bundle::V8_POLYFILLS;
use anyhow::Result;
use rex_core::RexConfig;
use std::sync::Arc;
use tracing::debug;

/// A pre-bundled dependency ESM module ready to load into V8.
pub struct DepEsmModule {
    /// The specifier user code imports from (e.g., "react", "react-dom/server").
    pub specifier: String,
    /// Self-contained ESM source bundled by rolldown.
    pub source: String,
}

/// Result of server dep pre-bundling.
pub struct ServerDepBundles {
    /// V8 polyfills to evaluate before any modules.
    pub polyfills: String,
    /// Pre-bundled dep ESM modules.
    pub modules: Vec<DepEsmModule>,
}

/// Build all dep ESM modules for the project.
///
/// Standard deps (React, renderToString) are always built.
/// RSC flight deps are built only if the project has an app/ directory.
pub async fn build_dep_esm_modules(
    config: &RexConfig,
    has_app_dir: bool,
    module_dirs: &[String],
) -> Result<ServerDepBundles> {
    let mut modules = Vec::new();

    // React core module.
    // For app router, use `react-server` conditions so RSC rendering works.
    // For pages router, use standard conditions (hooks must be available).
    let react_entry = concat!(
        "import React from 'react';\n",
        "var { createElement, useState, useEffect, useContext, useReducer, useCallback,\n",
        "  useMemo, useRef, useLayoutEffect, useImperativeHandle, useDebugValue,\n",
        "  useDeferredValue, useTransition, useId, useSyncExternalStore, useInsertionEffect,\n",
        "  useActionState, useOptimistic, use, memo, forwardRef, lazy, Suspense, Fragment,\n",
        "  Children, Component, PureComponent, createContext, createRef, cloneElement,\n",
        "  isValidElement, startTransition, cache } = React;\n",
        "export { createElement, useState, useEffect, useContext, useReducer, useCallback,\n",
        "  useMemo, useRef, useLayoutEffect, useImperativeHandle, useDebugValue,\n",
        "  useDeferredValue, useTransition, useId, useSyncExternalStore, useInsertionEffect,\n",
        "  useActionState, useOptimistic, use, memo, forwardRef, lazy, Suspense, Fragment,\n",
        "  Children, Component, PureComponent, createContext, createRef, cloneElement,\n",
        "  isValidElement, startTransition, cache };\n",
        "export default React;\n",
    );

    let react_conditions: &[&str] = if has_app_dir {
        &["react-server", "workerd", "browser", "import", "default"]
    } else {
        &["require", "default"]
    };

    // React core + jsx-runtime + jsx-dev-runtime: same conditions
    let react_deps = vec![
        ("react", react_entry),
        ("react/jsx-runtime", "import { jsx, jsxs, Fragment } from 'react/jsx-runtime'; export { jsx, jsxs, Fragment };"),
        ("react/jsx-dev-runtime", "import { jsxDEV, Fragment } from 'react/jsx-dev-runtime'; export { jsxDEV, Fragment };"),
    ];

    for (specifier, entry_source) in &react_deps {
        let source = build_dep_esm(
            config,
            entry_source,
            specifier,
            react_conditions,
            "",
            module_dirs,
        )
        .await?;
        modules.push(DepEsmModule {
            specifier: specifier.to_string(),
            source,
        });
    }

    // react-dom/server always uses standard conditions (renderToString)
    {
        let source = build_dep_esm(
            config,
            "import S from 'react-dom/server'; var { renderToString, renderToStaticMarkup } = S; export { renderToString, renderToStaticMarkup }; export default S;",
            "react-dom/server",
            &["require", "default"],
            "",
            module_dirs,
        )
        .await?;
        modules.push(DepEsmModule {
            specifier: "react-dom/server".to_string(),
            source,
        });
    }

    // RSC flight deps (react-server conditions) — only if app/ exists
    if has_app_dir {
        let webpack_shims = include_str!("../../../runtime/rsc/webpack-shims.ts");

        let flight_deps = vec![
            (
                "react-server-dom-webpack/server",
                concat!(
                    "import S from 'react-server-dom-webpack/server';\n",
                    "var { renderToReadableStream, registerServerReference, decodeReply, decodeAction } = S;\n",
                    "export { renderToReadableStream, registerServerReference, decodeReply, decodeAction };\n",
                    "export default S;\n",
                ),
            ),
            (
                "react-server-dom-webpack/client",
                concat!(
                    "import C from 'react-server-dom-webpack/client';\n",
                    "var { createFromReadableStream } = C;\n",
                    "export { createFromReadableStream };\n",
                    "export default C;\n",
                ),
            ),
        ];

        for (specifier, entry_source) in &flight_deps {
            let source = build_dep_esm(
                config,
                entry_source,
                specifier,
                &["react-server", "workerd", "browser", "import", "default"],
                webpack_shims,
                module_dirs,
            )
            .await?;
            modules.push(DepEsmModule {
                specifier: specifier.to_string(),
                source,
            });
        }
    }

    Ok(ServerDepBundles {
        polyfills: V8_POLYFILLS.to_string(),
        modules,
    })
}

/// Internal: build a single dep as a self-contained ESM module using rolldown.
async fn build_dep_esm(
    config: &RexConfig,
    entry_source: &str,
    name: &str,
    condition_names: &[&str],
    banner: &str,
    module_dirs: &[String],
) -> Result<String> {
    let sanitized_name = name.replace(['/', '-', '.', '@'], "_");
    let output_dir = config.server_build_dir().join("_dep_bundles");
    std::fs::create_dir_all(&output_dir)?;

    let entry_path = output_dir.join(format!("{sanitized_name}-entry.js"));
    std::fs::write(&entry_path, entry_source)?;

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

    // Combine V8 polyfills + any extra banner (e.g., webpack shims for RSC)
    let full_banner = if banner.is_empty() {
        V8_POLYFILLS.to_string()
    } else {
        format!("{}\n{}", V8_POLYFILLS, banner)
    };

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
        module_types: Some(module_types),
        define: Some(
            [("process.env.NODE_ENV".to_string(), define_env.to_string())]
                .into_iter()
                .collect(),
        ),
        banner: Some(rolldown::AddonOutputOption::String(Some(full_banner))),
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

    let bundle_path = output_dir.join(format!("{sanitized_name}.js"));
    let content = std::fs::read_to_string(&bundle_path)?;

    let _ = std::fs::remove_file(&entry_path);
    let _ = std::fs::remove_file(&bundle_path);

    debug!(name, size = content.len(), "Dep ESM module built");
    Ok(content)
}
