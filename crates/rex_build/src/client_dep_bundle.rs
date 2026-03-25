//! Browser-side dependency pre-bundling for unbundled dev serving.
//!
//! Bundles React and npm deps as self-contained ESM modules for the browser.
//! These are served via `/_rex/dep/{specifier}.js` and mapped through an
//! HTML import map so user source files can use bare specifiers (`react`, etc.).
//!
//! Unlike `server_dep_bundle` (which targets V8 with react-server conditions),
//! this uses standard browser conditions and bundles `react-dom/client` for
//! hydration instead of `react-dom/server`. No V8 polyfills are injected.

use crate::build_utils::runtime_client_dir;
use crate::esm_transform::DepImport;
use anyhow::Result;
use rex_core::RexConfig;
use std::collections::HashMap;
use tracing::debug;

/// Result of browser dep pre-bundling.
pub struct ClientDepBundle {
    /// Mapping of URL key → ESM source code.
    /// Keys use URL-safe encoding (e.g., "react__jsx-runtime" for "react/jsx-runtime").
    pub modules: HashMap<String, String>,
    /// The generated import map JSON string for injection into HTML `<script type="importmap">`.
    pub import_map_json: String,
}

/// Encode a bare specifier into a URL-safe filename.
/// `react/jsx-runtime` → `react__jsx-runtime`
/// `@scope/pkg` → `_scope__pkg`
pub fn specifier_to_url_key(specifier: &str) -> String {
    specifier.replace('@', "_").replace('/', "__")
}

/// Build browser-targeted ESM bundles for React and all discovered npm deps.
///
/// Returns pre-bundled modules and an import map mapping bare specifiers
/// to `/_rex/dep/{key}.js` URLs.
pub async fn build_client_dep_esm(
    config: &RexConfig,
    extra_deps: &[DepImport],
    module_dirs: &[String],
) -> Result<ClientDepBundle> {
    let mut modules: HashMap<String, String> = HashMap::new();
    let mut import_entries: Vec<(String, String)> = Vec::new();

    let browser_conditions: &[&str] = &["browser", "import", "module", "default"];

    // Core React deps for hydration
    let react_deps: Vec<(&str, &str)> = vec![
        ("react", concat!(
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
        )),
        ("react/jsx-runtime", "import { jsx, jsxs, Fragment } from 'react/jsx-runtime'; export { jsx, jsxs, Fragment };"),
        ("react/jsx-dev-runtime", "import { jsxDEV, Fragment } from 'react/jsx-dev-runtime'; export { jsxDEV, Fragment };"),
        ("react-dom/client", concat!(
            "import ReactDOM from 'react-dom/client';\n",
            "var { createRoot, hydrateRoot } = ReactDOM;\n",
            "export { createRoot, hydrateRoot };\n",
            "export default ReactDOM;\n",
        )),
    ];

    for (specifier, entry_source) in &react_deps {
        let source = bundle_for_browser(
            config,
            entry_source,
            specifier,
            browser_conditions,
            module_dirs,
        )
        .await?;
        let key = specifier_to_url_key(specifier);
        debug!(specifier, key, size = source.len(), "Client dep bundled");
        modules.insert(key.clone(), source);
        import_entries.push((specifier.to_string(), key));
    }

    // Framework stubs: rex/head, rex/link, rex/image, rex/router
    let runtime_dir = runtime_client_dir()?;
    let stubs: Vec<(&str, &str)> = vec![
        ("rex/head", "head.ts"),
        ("rex/link", "link.ts"),
        ("rex/image", "image.ts"),
        ("rex/router", "use-router.ts"),
    ];
    for (specifier, file) in &stubs {
        let stub_path = runtime_dir.join(file);
        if stub_path.exists() {
            let ts_source = std::fs::read_to_string(&stub_path)?;
            let js = crate::esm_transform::transform_to_esm(&ts_source, file)?;
            let key = specifier_to_url_key(specifier);
            debug!(specifier, key, "Client framework stub");
            modules.insert(key.clone(), js);
            import_entries.push((specifier.to_string(), key));
        }
    }

    // Extra deps discovered by import graph walk (clsx, lodash, etc.)
    for dep in extra_deps {
        let entry_source = build_reexport_entry(dep);
        let source = bundle_for_browser(
            config,
            &entry_source,
            &dep.specifier,
            browser_conditions,
            module_dirs,
        )
        .await?;
        let key = specifier_to_url_key(&dep.specifier);
        debug!(
            specifier = dep.specifier,
            key,
            size = source.len(),
            "Client extra dep bundled"
        );
        modules.insert(key.clone(), source);
        import_entries.push((dep.specifier.clone(), key));
    }

    // Generate import map JSON
    let import_map_json = generate_import_map(&import_entries);

    debug!(modules = modules.len(), "Client dep pre-bundling complete");

    Ok(ClientDepBundle {
        modules,
        import_map_json,
    })
}

/// Build a re-export entry source for an extra dep.
pub(crate) fn build_reexport_entry(dep: &DepImport) -> String {
    let mut entry = String::new();
    if dep.has_default && dep.named_exports.is_empty() {
        entry.push_str(&format!("export {{ default }} from '{}';\n", dep.specifier));
    } else if dep.has_default && !dep.named_exports.is_empty() {
        entry.push_str(&format!("export {{ default }} from '{}';\n", dep.specifier));
        let names = dep
            .named_exports
            .iter()
            .map(|n| n.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        entry.push_str(&format!(
            "export {{ {} }} from '{}';\n",
            names, dep.specifier
        ));
    } else if !dep.named_exports.is_empty() {
        let names = dep
            .named_exports
            .iter()
            .map(|n| n.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        entry.push_str(&format!(
            "export {{ {} }} from '{}';\n",
            names, dep.specifier
        ));
    } else {
        entry.push_str(&format!("export * from '{}';\n", dep.specifier));
        entry.push_str(&format!("export {{ default }} from '{}';\n", dep.specifier));
    }
    entry
}

/// Bundle a single dep as a self-contained ESM module for the browser.
///
/// No V8 polyfills, no node aliases. Browser-native platform with standard
/// resolve conditions.
async fn bundle_for_browser(
    config: &RexConfig,
    entry_source: &str,
    name: &str,
    condition_names: &[&str],
    module_dirs: &[String],
) -> Result<String> {
    let sanitized_name = name.replace(['/', '-', '.', '@'], "_");
    let output_dir = config
        .project_root
        .join(".rex")
        .join("cache")
        .join("client-deps");
    std::fs::create_dir_all(&output_dir)?;

    let entry_path = output_dir.join(format!("{sanitized_name}-entry.js"));
    std::fs::write(&entry_path, entry_source)?;

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
            [(
                "process.env.NODE_ENV".to_string(),
                "\"development\"".to_string(),
            )]
            .into_iter()
            .collect(),
        ),
        // No banner — browser has native APIs, no V8 polyfills needed
        tsconfig: Some(rolldown_common::TsConfig::Auto(false)),
        shim_missing_exports: Some(true),
        treeshake: crate::rsc_build_config::react_treeshake_options(),
        resolve: Some(rolldown::ResolveOptions {
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

    let mut bundler = rolldown::Bundler::new(options)
        .map_err(|e| anyhow::anyhow!("Failed to create client dep bundler: {e}"))?;

    if let Err(e) = bundler.write().await {
        if !crate::diagnostics::is_all_missing_exports(&e) {
            return Err(anyhow::anyhow!(
                "Client dep bundle ({name}) failed:\n{}",
                crate::diagnostics::format_build_diagnostics(&e)
            ));
        }
    }

    let bundle_path = output_dir.join(format!("{sanitized_name}.js"));
    let content = std::fs::read_to_string(&bundle_path)?;

    let _ = std::fs::remove_file(&entry_path);
    let _ = std::fs::remove_file(&bundle_path);

    debug!(name, size = content.len(), "Client dep ESM built");
    Ok(content)
}

/// Generate the import map JSON string from collected entries.
pub(crate) fn generate_import_map(entries: &[(String, String)]) -> String {
    let mut imports = serde_json::Map::new();
    for (specifier, key) in entries {
        imports.insert(
            specifier.clone(),
            serde_json::Value::String(format!("/_rex/dep/{key}.js")),
        );
    }
    let map = serde_json::json!({ "imports": imports });
    serde_json::to_string(&map).expect("import map JSON serialization")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_specifier_to_url_key() {
        assert_eq!(specifier_to_url_key("react"), "react");
        assert_eq!(
            specifier_to_url_key("react/jsx-runtime"),
            "react__jsx-runtime"
        );
        assert_eq!(specifier_to_url_key("@scope/pkg"), "_scope__pkg");
    }

    #[test]
    fn test_build_reexport_entry_default_only() {
        let dep = DepImport {
            specifier: "my-lib".to_string(),
            named_exports: HashSet::new(),
            has_default: true,
        };
        let entry = build_reexport_entry(&dep);
        assert!(
            entry.contains("export { default } from 'my-lib'"),
            "Should re-export default, got: {entry}"
        );
        assert!(
            !entry.contains("export *"),
            "Should not have wildcard export, got: {entry}"
        );
    }

    #[test]
    fn test_build_reexport_entry_named() {
        let mut named = HashSet::new();
        named.insert("foo".to_string());
        named.insert("bar".to_string());
        let dep = DepImport {
            specifier: "my-lib".to_string(),
            named_exports: named,
            has_default: false,
        };
        let entry = build_reexport_entry(&dep);
        assert!(
            entry.contains("foo") && entry.contains("bar"),
            "Should contain named exports, got: {entry}"
        );
        assert!(
            entry.contains("from 'my-lib'"),
            "Should reference specifier, got: {entry}"
        );
        assert!(
            !entry.contains("default"),
            "Should not have default export, got: {entry}"
        );
    }

    #[test]
    fn test_build_reexport_entry_both() {
        let mut named = HashSet::new();
        named.insert("useState".to_string());
        let dep = DepImport {
            specifier: "react".to_string(),
            named_exports: named,
            has_default: true,
        };
        let entry = build_reexport_entry(&dep);
        assert!(
            entry.contains("export { default } from 'react'"),
            "Should have default re-export, got: {entry}"
        );
        assert!(
            entry.contains("useState"),
            "Should have named re-export, got: {entry}"
        );
    }

    #[test]
    fn test_generate_import_map() {
        let entries = vec![
            ("react".to_string(), "react".to_string()),
            (
                "react/jsx-runtime".to_string(),
                "react__jsx-runtime".to_string(),
            ),
        ];
        let json = generate_import_map(&entries);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let imports = parsed.get("imports").unwrap().as_object().unwrap();
        assert_eq!(imports.get("react").unwrap(), "/_rex/dep/react.js");
        assert_eq!(
            imports.get("react/jsx-runtime").unwrap(),
            "/_rex/dep/react__jsx-runtime.js"
        );
    }
}
