//! One React instance for pages-router SSR in V8's ESM loader.
//!
//! The pages-router dev path loads React, the JSX runtimes and `react-dom/server`
//! as ESM modules, and bundles node_modules deps imported by user code (icon
//! libraries, UI kits) as further ESM modules. If each of those carries its own
//! copy of React, `renderToString` sets the hooks dispatcher on one copy while
//! components call hooks on another, and every hook throws
//! `Cannot read properties of null (reading 'useState')`.
//!
//! The fix mirrors the app router: [`PAGES_SERVER_DEPS_ENTRY`] is bundled as a
//! single IIFE (one React by construction) that publishes the instances on
//! `globalThis`, the dep specifiers become thin ESM wrappers over those globals,
//! and [`SharedReactResolvePlugin`] points every React import inside extra deps
//! at the same globals.

/// IIFE entry for pages-router server deps: React, both JSX runtimes and
/// `react-dom/server` in one rolldown graph, published on `globalThis`.
pub const PAGES_SERVER_DEPS_ENTRY: &str = concat!(
    "import React from 'react';\n",
    "import { jsx, jsxs, Fragment } from 'react/jsx-runtime';\n",
    "import { jsxDEV } from 'react/jsx-dev-runtime';\n",
    "import S from 'react-dom/server';\n",
    "\n",
    "globalThis.__rex_React = React;\n",
    "globalThis.__rex_jsx_runtime = { jsx: jsx, jsxs: jsxs, Fragment: Fragment };\n",
    "globalThis.__rex_jsx_dev_runtime = { jsxDEV: jsxDEV, Fragment: Fragment };\n",
    "globalThis.__rex_react_dom_server = S;\n",
);

/// Thin ESM wrapper for `react-dom/server` over the IIFE global.
pub const REACT_DOM_SERVER_WRAPPER_ESM: &str = concat!(
    "var S = globalThis.__rex_react_dom_server;\n",
    "export var renderToString = S.renderToString;\n",
    "export var renderToStaticMarkup = S.renderToStaticMarkup;\n",
    "export default S;\n",
);

/// Specifiers redirected to the shared instances, with the global each one
/// re-exports. Matched exactly: `react/compiler-runtime` and `react-dom` are
/// left to normal resolution (and their own `require('react')` lands here).
pub const SHARED_REACT_SPECIFIERS: &[(&str, &str)] = &[
    ("react", "__rex_React"),
    ("react/jsx-runtime", "__rex_jsx_runtime"),
    ("react/jsx-dev-runtime", "__rex_jsx_dev_runtime"),
    ("react-dom/server", "__rex_react_dom_server"),
];

const VIRTUAL_PREFIX: &str = "\0rex-shared-react:";

/// Rolldown plugin that resolves the [`SHARED_REACT_SPECIFIERS`] to virtual
/// CommonJS modules (`module.exports = globalThis.__rex_React`). CommonJS so
/// that both `import { useState } from 'react'` and `require('react')` in a
/// dependency get the shared instance through rolldown's interop.
#[derive(Debug, Default)]
pub struct SharedReactResolvePlugin;

impl SharedReactResolvePlugin {
    fn global_for(specifier: &str) -> Option<&'static str> {
        SHARED_REACT_SPECIFIERS
            .iter()
            .find(|(s, _)| *s == specifier)
            .map(|(_, g)| *g)
    }

    /// The module source a shared specifier resolves to, if it is one.
    pub fn shim_source(specifier: &str) -> Option<String> {
        Self::global_for(specifier).map(|g| format!("module.exports = globalThis.{g};\n"))
    }
}

impl rolldown::plugin::Plugin for SharedReactResolvePlugin {
    fn name(&self) -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("rex:shared-react")
    }

    fn resolve_id(
        &self,
        _ctx: &rolldown::plugin::PluginContext,
        args: &rolldown::plugin::HookResolveIdArgs<'_>,
    ) -> impl std::future::Future<Output = rolldown::plugin::HookResolveIdReturn> + Send {
        let result = Self::global_for(args.specifier).map(|_| {
            rolldown::plugin::HookResolveIdOutput::from_id(format!(
                "{VIRTUAL_PREFIX}{}",
                args.specifier
            ))
        });
        async move { Ok(result) }
    }

    fn load(
        &self,
        _ctx: rolldown::plugin::SharedLoadPluginContext,
        args: &rolldown::plugin::HookLoadArgs<'_>,
    ) -> impl std::future::Future<Output = rolldown::plugin::HookLoadReturn> + Send {
        let result = args
            .id
            .strip_prefix(VIRTUAL_PREFIX)
            .and_then(Self::shim_source)
            .map(|code| rolldown::plugin::HookLoadOutput {
                code: code.into(),
                ..Default::default()
            });
        async move { Ok(result) }
    }

    fn register_hook_usage(&self) -> rolldown::plugin::HookUsage {
        rolldown::plugin::HookUsage::ResolveId | rolldown::plugin::HookUsage::Load
    }
}
