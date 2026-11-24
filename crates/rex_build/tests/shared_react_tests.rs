//! Pages-router SSR must run on one React instance: the one `renderToString`
//! sets the hooks dispatcher on. Regression tests for limlabs/rex#240, where
//! dev SSR failed with `Cannot read properties of null (reading 'useState')`
//! because `react`, `react-dom/server` and extra deps each bundled a copy.
//!
//! These use the real React that `resolve_modules_dirs` provides (the embedded
//! builtin packages), not the mocks, because the bug lives in React's
//! dispatcher.

#![allow(clippy::unwrap_used)]

mod common;

use common::setup_test_project;
use rex_build::esm_transform::DepImport;
use rex_build::shared_react::SharedReactResolvePlugin;
use std::collections::HashSet;
use std::fs;

/// A region rolldown emits whenever it bundles React's own CommonJS build.
const REACT_BUILD_REGION: &str = "node_modules/react/cjs/react.";

fn project() -> (tempfile::TempDir, rex_core::RexConfig, Vec<String>) {
    let (tmp, config, _scan) = setup_test_project(
        &[("index.tsx", "export default function P() { return null; }")],
        None,
    );
    // Installs the real react / react-dom into the project's node_modules.
    let module_dirs = rex_build::resolve_modules_dirs(&config).unwrap();
    (tmp, config, module_dirs)
}

#[tokio::test]
async fn pages_router_hooks_render_through_the_shared_instance() {
    let (_tmp, config, module_dirs) = project();

    let deps = rex_build::server_dep_bundle::build_dep_esm_modules(&config, false, &module_dirs)
        .await
        .unwrap();

    // The ESM modules user code imports are thin wrappers over the globals the
    // IIFE publishes, not copies of React.
    for specifier in [
        "react",
        "react/jsx-runtime",
        "react/jsx-dev-runtime",
        "react-dom/server",
    ] {
        let module = deps
            .modules
            .iter()
            .find(|m| m.specifier == specifier)
            .unwrap_or_else(|| panic!("missing dep module {specifier}"));
        assert!(
            module.source.contains("globalThis.__rex_"),
            "{specifier} should re-export a shared global, got:\n{}",
            module.source
        );
        assert!(
            !module.source.contains(REACT_BUILD_REGION),
            "{specifier} must not bundle its own React"
        );
    }

    // A component calling a hook, created with the `react` users import and
    // rendered with the `react-dom/server` users import: the failure in #240.
    let html = rex_v8::eval_once(
        &deps.polyfills,
        &[(
            concat!(
                "var R = globalThis.__rex_React;\n",
                "function Counter() { var s = R.useState(41); var c = R.useContext(R.createContext('ctx')); return R.createElement('b', null, c + ':' + (s[0] + 1)); }\n",
                "globalThis.__rex_react_dom_server.renderToString(R.createElement(Counter));\n",
            ),
            "render.js",
        )],
    )
    .unwrap();
    assert_eq!(html, "<b>ctx:42</b>");
}

fn write_package(root: &std::path::Path, name: &str, source: &str) {
    let dir = root.join("node_modules").join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("package.json"),
        format!(r#"{{"name":"{name}","version":"1.0.0","main":"index.js"}}"#),
    )
    .unwrap();
    fs::write(dir.join("index.js"), source).unwrap();
}

fn extra_deps_project() -> (
    tempfile::TempDir,
    rex_core::RexConfig,
    Vec<String>,
    Vec<DepImport>,
) {
    let (tmp, config, module_dirs) = project();
    // An ESM dep with named hook imports (like an icon library) …
    write_package(
        &config.project_root,
        "esm-hooks-dep",
        "import { createElement, useContext, createContext } from 'react';\nimport { jsx } from 'react/jsx-runtime';\nvar Ctx = createContext('regular');\nexport function Icon() { var w = useContext(Ctx); return jsx('svg', { 'data-weight': w }) || createElement('svg'); }\n",
    );
    // … and a CommonJS dep that requires React.
    write_package(
        &config.project_root,
        "cjs-hooks-dep",
        "var React = require('react');\nmodule.exports = function Box() { return React.createElement('div', null, React.useState('box')[0]); };\n",
    );
    let deps = vec![
        DepImport {
            specifier: "esm-hooks-dep".to_string(),
            named_exports: HashSet::from(["Icon".to_string()]),
            has_default: false,
        },
        DepImport {
            specifier: "cjs-hooks-dep".to_string(),
            named_exports: HashSet::new(),
            has_default: true,
        },
    ];
    (tmp, config, module_dirs, deps)
}

async fn bundle_extra_deps(shared_react: bool) -> String {
    let (_tmp, config, module_dirs, deps) = extra_deps_project();
    let result = rex_build::extra_dep_bundle::build_extra_deps_multi_entry(
        &config,
        &deps,
        &module_dirs,
        &[],
        shared_react,
    )
    .await
    .unwrap();
    result.modules.iter().map(|(_, s)| s.as_str()).collect()
}

#[tokio::test]
async fn extra_deps_use_the_shared_react_in_pages_router() {
    let all = bundle_extra_deps(true).await;

    assert!(
        all.contains("globalThis.__rex_React"),
        "deps should read the shared React:\n{all}"
    );
    assert!(
        all.contains("globalThis.__rex_jsx_runtime"),
        "deps should read the shared JSX runtime"
    );
    assert!(
        !all.contains(REACT_BUILD_REGION),
        "deps must not bundle their own React"
    );
}

#[tokio::test]
async fn extra_deps_keep_their_own_react_without_shared_react() {
    // Control for the test above, and the app-router behaviour: without
    // `shared_react`, rolldown bundles React into the deps.
    let all = bundle_extra_deps(false).await;

    assert!(all.contains(REACT_BUILD_REGION));
    assert!(!all.contains("globalThis.__rex_React"));
}

#[test]
fn shared_react_matches_specifiers_exactly() {
    assert!(SharedReactResolvePlugin::shim_source("react")
        .unwrap()
        .contains("globalThis.__rex_React"));
    assert!(SharedReactResolvePlugin::shim_source("react/jsx-runtime")
        .unwrap()
        .contains("globalThis.__rex_jsx_runtime"));
    assert!(SharedReactResolvePlugin::shim_source("react-dom/server")
        .unwrap()
        .contains("globalThis.__rex_react_dom_server"));
    // Left to normal resolution: not part of the shared instance set.
    assert!(SharedReactResolvePlugin::shim_source("react-dom").is_none());
    assert!(SharedReactResolvePlugin::shim_source("react/compiler-runtime").is_none());
    assert!(SharedReactResolvePlugin::shim_source("react-is").is_none());
}
