//! Server dependency pre-bundling for ESM module loading.
//!
//! Bundles React and friends for V8's native ESM module system.
//!
//! **Server deps** (react, jsx-runtime, react-server-dom-webpack/server) are built
//! as a single IIFE with `react-server` conditions — evaluated as a script to set
//! globals, then wrapped as thin ESM modules. This guarantees a single React instance.
//!
//! **SSR deps** (react-dom/server, react-server-dom-webpack/client) are built as
//! separate self-contained ESM modules with standard conditions. They bundle their
//! own React copy, which is fine because the SSR pass operates on serialized flight
//! data, not shared React element trees.

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
    /// V8 polyfills + server deps IIFE to evaluate as a script before any modules.
    pub polyfills: String,
    /// ESM wrapper modules for each dep specifier.
    pub modules: Vec<DepEsmModule>,
}

/// Build all dep ESM modules for the project.
///
/// For app router: builds a single IIFE with all react-server deps (single React
/// instance), plus separate ESM modules for SSR deps. Thin ESM wrappers re-export
/// from the IIFE's globals.
///
/// For pages router: builds each dep as a separate self-contained ESM module
/// (no RSC rendering, so dual-instance issues don't apply).
pub async fn build_dep_esm_modules(
    config: &RexConfig,
    has_app_dir: bool,
    module_dirs: &[String],
) -> Result<ServerDepBundles> {
    if has_app_dir {
        build_app_router_deps(config, module_dirs).await
    } else {
        build_pages_router_deps(config, module_dirs).await
    }
}

/// App router: single IIFE for react-server deps + separate ESM for SSR deps.
async fn build_app_router_deps(
    config: &RexConfig,
    module_dirs: &[String],
) -> Result<ServerDepBundles> {
    let webpack_shims = include_str!("../../../runtime/rsc/webpack-shims.ts");

    // Build single IIFE with ALL react-server condition deps.
    // This ensures one React instance shared by createElement and renderToReadableStream.
    let server_iife = build_server_deps_iife(
        config,
        module_dirs,
        APP_SERVER_DEPS_ENTRY,
        webpack_shims,
        &["react-server", "workerd", "browser", "import", "default"],
    )
    .await?;

    let mut polyfills = V8_POLYFILLS.to_string();
    polyfills.push('\n');
    polyfills.push_str(&server_iife);

    // Create thin ESM wrapper modules that re-export from the IIFE globals.
    let mut modules = vec![
        // react wrapper
        DepEsmModule {
            specifier: "react".to_string(),
            source: REACT_WRAPPER_ESM.to_string(),
        },
        // react/jsx-runtime wrapper
        DepEsmModule {
            specifier: "react/jsx-runtime".to_string(),
            source: JSX_RUNTIME_WRAPPER_ESM.to_string(),
        },
        // react/jsx-dev-runtime wrapper
        DepEsmModule {
            specifier: "react/jsx-dev-runtime".to_string(),
            source: JSX_DEV_RUNTIME_WRAPPER_ESM.to_string(),
        },
        // react-server-dom-webpack/server wrapper
        DepEsmModule {
            specifier: "react-server-dom-webpack/server".to_string(),
            source: concat!(
                "var S = globalThis.__rex_flight_server;\n",
                "export var renderToReadableStream = S.renderToReadableStream;\n",
                "export var registerServerReference = S.registerServerReference;\n",
                "export var decodeReply = S.decodeReply;\n",
                "export var decodeAction = S.decodeAction;\n",
                "export default S;\n",
            )
            .to_string(),
        },
    ];

    // react-dom/server: separate self-contained ESM (standard conditions).
    // Used for SSR pass — doesn't need to share React with RSC rendering.
    let rdom_source = build_dep_esm(
        config,
        "import S from 'react-dom/server'; var { renderToString, renderToStaticMarkup } = S; export { renderToString, renderToStaticMarkup }; export default S;",
        "react-dom/server",
        &["require", "default"],
        "",
        module_dirs,
    ).await?;
    modules.push(DepEsmModule {
        specifier: "react-dom/server".to_string(),
        source: rdom_source,
    });

    // react-server-dom-webpack/client: separate self-contained ESM.
    // Used for SSR pass (createFromReadableStream) — doesn't share React with RSC.
    let flight_client_source = build_dep_esm(
        config,
        concat!(
            "import C from 'react-server-dom-webpack/client';\n",
            "var { createFromReadableStream } = C;\n",
            "export { createFromReadableStream };\n",
            "export default C;\n",
        ),
        "react-server-dom-webpack/client",
        &["react-server", "workerd", "browser", "import", "default"],
        webpack_shims,
        module_dirs,
    )
    .await?;
    modules.push(DepEsmModule {
        specifier: "react-server-dom-webpack/client".to_string(),
        source: flight_client_source,
    });

    Ok(ServerDepBundles { polyfills, modules })
}

/// Pages router: one IIFE holding React, both JSX runtimes and `react-dom/server`
/// (a single React instance, so hooks see the dispatcher `renderToString` sets),
/// exposed through thin ESM wrappers like the app router. Extra deps reach the
/// same instance through [`crate::shared_react::SharedReactResolvePlugin`].
async fn build_pages_router_deps(
    config: &RexConfig,
    module_dirs: &[String],
) -> Result<ServerDepBundles> {
    let server_iife = build_server_deps_iife(
        config,
        module_dirs,
        crate::shared_react::PAGES_SERVER_DEPS_ENTRY,
        "",
        &["require", "default"],
    )
    .await?;

    let mut polyfills = V8_POLYFILLS.to_string();
    polyfills.push('\n');
    polyfills.push_str(&server_iife);

    let modules = vec![
        DepEsmModule {
            specifier: "react".to_string(),
            source: REACT_WRAPPER_ESM.to_string(),
        },
        DepEsmModule {
            specifier: "react/jsx-runtime".to_string(),
            source: JSX_RUNTIME_WRAPPER_ESM.to_string(),
        },
        DepEsmModule {
            specifier: "react/jsx-dev-runtime".to_string(),
            source: JSX_DEV_RUNTIME_WRAPPER_ESM.to_string(),
        },
        DepEsmModule {
            specifier: "react-dom/server".to_string(),
            source: crate::shared_react::REACT_DOM_SERVER_WRAPPER_ESM.to_string(),
        },
    ];

    Ok(ServerDepBundles { polyfills, modules })
}

/// IIFE entry for app-router server deps (`react-server` conditions).
const APP_SERVER_DEPS_ENTRY: &str = concat!(
    "import React from 'react';\n",
    "import { jsx, jsxs, Fragment } from 'react/jsx-runtime';\n",
    "import { jsxDEV } from 'react/jsx-dev-runtime';\n",
    "import S from 'react-server-dom-webpack/server';\n",
    "\n",
    "globalThis.__rex_React = React;\n",
    "globalThis.__rex_jsx_runtime = { jsx: jsx, jsxs: jsxs, Fragment: Fragment };\n",
    "globalThis.__rex_jsx_dev_runtime = { jsxDEV: jsxDEV, Fragment: Fragment };\n",
    "globalThis.__rex_flight_server = S;\n",
);

/// Thin ESM wrapper for `react/jsx-runtime` over the IIFE global.
const JSX_RUNTIME_WRAPPER_ESM: &str = "var R = globalThis.__rex_jsx_runtime; export var jsx = R.jsx; export var jsxs = R.jsxs; export var Fragment = R.Fragment;";

/// Thin ESM wrapper for `react/jsx-dev-runtime` over the IIFE global.
const JSX_DEV_RUNTIME_WRAPPER_ESM: &str = "var R = globalThis.__rex_jsx_dev_runtime; export var jsxDEV = R.jsxDEV; export var Fragment = R.Fragment;";

/// Thin ESM wrapper for the `react` module.
/// Re-exports from `globalThis.__rex_React` (set by the server deps IIFE).
const REACT_WRAPPER_ESM: &str = concat!(
    "var R = globalThis.__rex_React;\n",
    "export var createElement = R.createElement;\n",
    "export var useState = R.useState;\n",
    "export var useEffect = R.useEffect;\n",
    "export var useContext = R.useContext;\n",
    "export var useReducer = R.useReducer;\n",
    "export var useCallback = R.useCallback;\n",
    "export var useMemo = R.useMemo;\n",
    "export var useRef = R.useRef;\n",
    "export var useLayoutEffect = R.useLayoutEffect;\n",
    "export var useImperativeHandle = R.useImperativeHandle;\n",
    "export var useDebugValue = R.useDebugValue;\n",
    "export var useDeferredValue = R.useDeferredValue;\n",
    "export var useTransition = R.useTransition;\n",
    "export var useId = R.useId;\n",
    "export var useSyncExternalStore = R.useSyncExternalStore;\n",
    "export var useInsertionEffect = R.useInsertionEffect;\n",
    "export var useActionState = R.useActionState;\n",
    "export var useOptimistic = R.useOptimistic;\n",
    "export var use = R.use;\n",
    "export var memo = R.memo;\n",
    "export var forwardRef = R.forwardRef;\n",
    "export var lazy = R.lazy;\n",
    "export var Suspense = R.Suspense;\n",
    "export var Fragment = R.Fragment;\n",
    "export var Children = R.Children;\n",
    "export var Component = R.Component;\n",
    "export var PureComponent = R.PureComponent;\n",
    "export var createContext = R.createContext;\n",
    "export var createRef = R.createRef;\n",
    "export var cloneElement = R.cloneElement;\n",
    "export var isValidElement = R.isValidElement;\n",
    "export var startTransition = R.startTransition;\n",
    "export var cache = R.cache;\n",
    "export default R;\n",
);

/// Build a single IIFE from `entry_source`, which imports React and its
/// renderer(s) and sets globals on `globalThis`. Bundled as one IIFE so
/// everything shares one React instance (no dual-instance bugs).
async fn build_server_deps_iife(
    config: &RexConfig,
    module_dirs: &[String],
    entry_source: &str,
    webpack_shims: &str,
    condition_names: &[&str],
) -> Result<String> {
    let sanitized_name = "server_deps";
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

    // Banner: webpack shims (needed by react-server-dom-webpack)
    let banner = if webpack_shims.is_empty() {
        String::new()
    } else {
        webpack_shims.to_string()
    };

    let options = rolldown::BundlerOptions {
        input: Some(vec![rolldown::InputItem {
            name: Some(sanitized_name.to_string()),
            import: entry_path.to_string_lossy().to_string(),
        }]),
        cwd: Some(config.project_root.clone()),
        format: Some(rolldown::OutputFormat::Iife),
        dir: Some(output_dir.to_string_lossy().to_string()),
        entry_filenames: Some(format!("{sanitized_name}.js").into()),
        platform: Some(rolldown::Platform::Browser),
        module_types: Some(module_types),
        define: Some(
            [("process.env.NODE_ENV".to_string(), define_env.to_string())]
                .into_iter()
                .collect(),
        ),
        banner: Some(rolldown::AddonOutputOption::String(Some(banner))),
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
        .map_err(|e| anyhow::anyhow!("Failed to create server deps bundler: {e}"))?;

    if let Err(e) = bundler.write().await {
        if !crate::diagnostics::is_all_missing_exports(&e) {
            return Err(anyhow::anyhow!(
                "Server deps IIFE build failed:\n{}",
                crate::diagnostics::format_build_diagnostics(&e)
            ));
        }
    }

    let bundle_path = output_dir.join(format!("{sanitized_name}.js"));
    let content = std::fs::read_to_string(&bundle_path)?;

    let _ = std::fs::remove_file(&entry_path);
    let _ = std::fs::remove_file(&bundle_path);

    debug!(size = content.len(), "Server deps IIFE built");
    Ok(content)
}

/// Internal: build a single dep as a self-contained ESM module using rolldown.
/// Bundle a bare specifier dependency as a self-contained ESM module via rolldown.
///
/// Used for both pre-known deps (react-dom/server) and extra deps discovered
/// during import graph walking (payload, clsx, etc.).
pub async fn build_dep_esm(
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

// Extra dep bundling functions are in `extra_dep_bundle.rs`.
