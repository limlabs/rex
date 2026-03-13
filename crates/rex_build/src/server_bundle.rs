use crate::build_utils::runtime_server_dir;
use anyhow::Result;
use rex_core::{ProjectConfig, RexConfig};
use rex_router::ScanResult;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::debug;

/// Rolldown plugin that intercepts specific bare-specifier imports that aliases
/// miss when the import originates from within node_modules (pnpm symlinks).
#[derive(Debug)]
pub(crate) struct NodePolyfillResolvePlugin {
    /// Maps bare specifier prefixes → absolute path of stub file.
    redirects: Vec<(String, String)>,
    /// Fallback stub for unloadable file types (e.g. `.wasm?module`).
    empty_stub: String,
}

impl NodePolyfillResolvePlugin {
    pub fn new(redirects: Vec<(String, String)>, empty_stub: String) -> Self {
        Self {
            redirects,
            empty_stub,
        }
    }
}

impl rolldown::plugin::Plugin for NodePolyfillResolvePlugin {
    fn name(&self) -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("rex:node-polyfill-resolve")
    }

    fn resolve_id(
        &self,
        _ctx: &rolldown::plugin::PluginContext,
        args: &rolldown::plugin::HookResolveIdArgs<'_>,
    ) -> impl std::future::Future<Output = rolldown::plugin::HookResolveIdReturn> + Send {
        let specifier = args.specifier;
        let result = self.redirects.iter().find_map(|(prefix, target)| {
            if specifier == prefix
                || (specifier.len() > prefix.len()
                    && specifier.starts_with(prefix)
                    && matches!(specifier.as_bytes()[prefix.len()], b'/' | b'.'))
            {
                Some(rolldown::plugin::HookResolveIdOutput::from_id(
                    target.clone(),
                ))
            } else {
                None
            }
        });
        // Catch .wasm imports with query strings (e.g. "./resvg.wasm?module")
        // that rolldown can't load as regular files.
        let result = result.or_else(|| {
            if specifier.contains(".wasm?") || specifier.contains(".wasm!") {
                Some(rolldown::plugin::HookResolveIdOutput::from_id(
                    self.empty_stub.clone(),
                ))
            } else {
                None
            }
        });
        async move { Ok(result) }
    }

    fn load(
        &self,
        _ctx: rolldown::plugin::SharedLoadPluginContext,
        args: &rolldown::plugin::HookLoadArgs<'_>,
    ) -> impl std::future::Future<Output = rolldown::plugin::HookLoadReturn> + Send {
        // Intercept .wasm imports with query strings (e.g. "...resvg.wasm?module")
        // that rolldown resolved but can't load from disk.
        let result = if args.id.contains(".wasm?") {
            Some(rolldown::plugin::HookLoadOutput {
                code: "export default {};".into(),
                ..Default::default()
            })
        } else {
            None
        };
        async move { Ok(result) }
    }

    fn register_hook_usage(&self) -> rolldown::plugin::HookUsage {
        rolldown::plugin::HookUsage::ResolveId | rolldown::plugin::HookUsage::Load
    }
}

/// Rolldown plugin that stubs out heavy server-only packages (database drivers,
/// AWS SDK, etc.) that leak into the RSC server bundle via PayloadCMS or other
/// frameworks. These packages can't run in V8 and their class hierarchies crash
/// with "Class extends value is not a constructor" when their parent classes
/// resolve to empty stubs.
///
/// Unlike `NodePolyfillResolvePlugin` which redirects imports to an empty file,
/// this plugin reads the original file's exports and generates a stub that
/// provides constructable classes for each export, preventing class extension
/// crashes at evaluation time.
#[derive(Debug)]
pub(crate) struct HeavyPackageStubPlugin {
    /// Package path prefixes to stub (matched against resolved file paths).
    /// e.g., `["node_modules/@aws-sdk/", "node_modules/drizzle-orm/"]`
    package_prefixes: Vec<String>,
}

impl HeavyPackageStubPlugin {
    pub fn new(package_prefixes: Vec<String>) -> Self {
        Self { package_prefixes }
    }

    fn should_stub(&self, resolved_id: &str) -> bool {
        self.package_prefixes
            .iter()
            .any(|prefix| resolved_id.contains(prefix.as_str()))
    }

    /// Extract export names from a JS/TS source file using basic pattern matching.
    /// Public for reuse by `UseClientDetectPlugin`.
    /// Handles: `export function X`, `export class X`, `export const/let/var X`,
    /// `export { X, Y }`, `export default`, `exports.X = `, `module.exports`.
    pub fn extract_exports(source: &str) -> Vec<String> {
        let mut exports = Vec::new();
        let mut has_default = false;

        for line in source.lines() {
            let trimmed = line.trim();

            // ESM: export default
            if trimmed.starts_with("export default") {
                has_default = true;
                continue;
            }

            // ESM: export function/class/const/let/var NAME
            if let Some(rest) = trimmed.strip_prefix("export ") {
                let rest = rest.trim_start();
                // Skip "export {" and "export *"
                if rest.starts_with('{') || rest.starts_with('*') {
                    // Handle "export { X, Y, Z }"
                    if rest.starts_with('{') {
                        let block = rest.trim_start_matches('{');
                        for part in block.split(',') {
                            let name = part
                                .split_whitespace()
                                .next()
                                .unwrap_or("")
                                .trim_end_matches('}');
                            if !name.is_empty()
                                && name != "}"
                                && name.chars().next().is_some_and(|c| c.is_alphabetic())
                            {
                                exports.push(name.to_string());
                            }
                        }
                    }
                    continue;
                }
                // Skip keywords to get to the name
                let rest = rest
                    .strip_prefix("async ")
                    .unwrap_or(rest)
                    .strip_prefix("function ")
                    .or_else(|| rest.strip_prefix("function* "))
                    .or_else(|| rest.strip_prefix("class "))
                    .or_else(|| rest.strip_prefix("const "))
                    .or_else(|| rest.strip_prefix("let "))
                    .or_else(|| rest.strip_prefix("var "))
                    .unwrap_or("");
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
                    .collect();
                if !name.is_empty() {
                    exports.push(name);
                }
                continue;
            }

            // CJS: exports.X = or module.exports.X = or module.exports =
            if let Some(rest) = trimmed.strip_prefix("exports.") {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
                    .collect();
                if !name.is_empty() && name != "default" {
                    exports.push(name);
                }
            }
            if let Some(rest) = trimmed.strip_prefix("module.exports.") {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
                    .collect();
                if !name.is_empty() && name != "default" {
                    exports.push(name);
                }
            } else if trimmed.starts_with("module.exports") {
                has_default = true;
            }
        }

        if has_default && !exports.contains(&"default".to_string()) {
            exports.push("default".to_string());
        }
        exports.sort();
        exports.dedup();
        exports
    }

    fn generate_stub_code(exports: &[String]) -> String {
        // Deep proxy stub: every property access, function call, and constructor
        // invocation returns another proxy. Handles all chaining patterns like
        // `db.init().connect()`. Special cases prevent infinite loops and handle
        // primitive coercion for string/number contexts.
        let mut code = String::from(
            r#"var __$P=new Proxy(function(){},{get(t,k){if(k==="then")return function(r){if(r)r(null)};if(k===Symbol.toPrimitive)return()=>"";if(k===Symbol.iterator)return function*(){};if(k==="toString"||k==="toJSON")return()=>"";if(k==="valueOf")return()=>0;if(k==="length"||k==="size")return 0;if(k==="prototype")return{};if(k==="docs"||k==="data")return[];if(k==="totalDocs"||k==="totalPages")return 0;return __$P},apply(){return __$P},construct(){return __$P},set(){return true},has(){return true}});
"#,
        );
        for name in exports {
            if name == "default" {
                code.push_str("export default __$P;\n");
            } else {
                code.push_str(&format!("export var {name} = __$P;\n"));
            }
        }
        if !exports.contains(&"default".to_string()) {
            code.push_str("export default __$P;\n");
        }
        code
    }
}

impl rolldown::plugin::Plugin for HeavyPackageStubPlugin {
    fn name(&self) -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("rex:heavy-package-stub")
    }

    fn load(
        &self,
        _ctx: rolldown::plugin::SharedLoadPluginContext,
        args: &rolldown::plugin::HookLoadArgs<'_>,
    ) -> impl std::future::Future<Output = rolldown::plugin::HookLoadReturn> + Send {
        let result = if self.should_stub(args.id) {
            let source = fs::read_to_string(args.id).unwrap_or_default();
            let exports = Self::extract_exports(&source);
            let code = Self::generate_stub_code(&exports);
            Some(rolldown::plugin::HookLoadOutput {
                code: code.into(),
                ..Default::default()
            })
        } else {
            None
        };
        async move { Ok(result) }
    }

    fn register_hook_usage(&self) -> rolldown::plugin::HookUsage {
        rolldown::plugin::HookUsage::Load
    }
}

/// Rolldown plugin that intercepts `.module.css` imports and returns a JS proxy
/// with scoped class name mappings. This allows server components in RSC/SSR
/// bundles to import CSS modules and get the correct class name strings.
///
/// Without this plugin, `.module.css` files match the `.css → Empty` module type
/// rule and resolve to `undefined`, crashing components that access class names.
#[derive(Debug)]
pub(crate) struct CssModulePlugin {
    /// Directory where proxy JS files are written.
    temp_dir: PathBuf,
}

impl CssModulePlugin {
    pub fn new(temp_dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&temp_dir);
        Self { temp_dir }
    }

    fn generate_proxy(&self, css_path: &Path) -> Result<PathBuf> {
        let css_content = fs::read_to_string(css_path)?;
        let classes = crate::css_modules::parse_css_classes(&css_content);
        let file_hash = crate::css_modules::css_module_hash(css_path);
        let stem = crate::css_modules::css_module_stem(css_path);

        let mut class_map = HashMap::new();
        for class in &classes {
            class_map.insert(class.clone(), format!("{stem}_{class}_{file_hash}"));
        }

        let proxy_code = crate::css_modules::generate_css_module_proxy(&class_map);

        let proxy_name = format!("{stem}_{file_hash}.css-module.js");
        let proxy_path = self.temp_dir.join(&proxy_name);
        fs::write(&proxy_path, &proxy_code)?;

        Ok(proxy_path)
    }
}

impl rolldown::plugin::Plugin for CssModulePlugin {
    fn name(&self) -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("rex:css-module")
    }

    fn resolve_id(
        &self,
        _ctx: &rolldown::plugin::PluginContext,
        args: &rolldown::plugin::HookResolveIdArgs<'_>,
    ) -> impl std::future::Future<Output = rolldown::plugin::HookResolveIdReturn> + Send {
        let specifier = args.specifier;
        let importer = args.importer.map(|s| s.to_string());

        let result = if specifier.ends_with(".module.css") {
            let css_path = if specifier.starts_with('.') {
                importer
                    .as_deref()
                    .and_then(|imp| Path::new(imp).parent().map(|dir| dir.join(specifier)))
            } else {
                // Absolute path or bare specifier — try as-is
                Some(PathBuf::from(specifier))
            };

            css_path.and_then(|p| {
                if p.exists() {
                    self.generate_proxy(&p).ok().map(|proxy| {
                        rolldown::plugin::HookResolveIdOutput::from_id(
                            proxy.to_string_lossy().to_string(),
                        )
                    })
                } else {
                    None
                }
            })
        } else {
            None
        };

        async move { Ok(result) }
    }

    fn register_hook_usage(&self) -> rolldown::plugin::HookUsage {
        rolldown::plugin::HookUsage::ResolveId
    }
}

/// V8 polyfills for bare V8 environment (React 19 needs these).
/// Injected as a rolldown banner so they run before any bundled code.
/// Compiled from TypeScript at build time by build.rs.
/// Compiled V8 polyfills JS, concatenated from `runtime/server/polyfills/*.ts`.
/// Public so `rex_v8` tests can use the real polyfills instead of duplicating them.
pub const V8_POLYFILLS: &str = include_str!(concat!(env!("OUT_DIR"), "/v8-polyfills.js"));

/// SSR runtime functions appended to the virtual entry.
/// These are bundled into the IIFE by rolldown alongside React and page code.
const SSR_RUNTIME: &str = include_str!(concat!(env!("OUT_DIR"), "/ssr-runtime.js"));

/// MCP tool runtime functions appended to the virtual entry when mcp/ tools exist.
/// Defines __rex_list_mcp_tools() and __rex_call_mcp_tool(name, paramsJson).
const MCP_RUNTIME: &str = include_str!(concat!(env!("OUT_DIR"), "/mcp-runtime.js"));

/// Middleware runtime functions appended to the virtual entry when middleware.ts exists.
/// Defines __rex_run_middleware(reqJson) and __rex_resolve_middleware().
const MIDDLEWARE_RUNTIME: &str = include_str!(concat!(env!("OUT_DIR"), "/middleware-runtime.js"));

/// App route handler runtime for route.ts dispatch.
/// Defines __rex_call_app_route_handler(routePattern, reqJson) and __rex_resolve_app_route().
const APP_ROUTE_RUNTIME: &str = include_str!(concat!(env!("OUT_DIR"), "/app-route-runtime.js"));

/// Build the server bundle using rolldown.
///
/// Produces a self-contained IIFE that includes React, all pages, and SSR
/// runtime functions. Runs in bare V8 with no module loader.
pub(crate) async fn build_server_bundle(
    config: &RexConfig,
    scan: &ScanResult,
    output_dir: &Path,
    page_overrides: &HashMap<PathBuf, PathBuf>,
    define: &[(String, String)],
    project_config: &ProjectConfig,
    module_dirs: &[String],
) -> Result<PathBuf> {
    let runtime_dir = runtime_server_dir()?;

    // Generate virtual entry that imports everything and registers on globalThis
    let entries_dir = output_dir.join("_server_entry");
    fs::create_dir_all(&entries_dir)?;

    let mut entry = String::new();

    // Import React (resolved from node_modules by rolldown)
    entry.push_str("import { createElement } from 'react';\n");
    entry.push_str("import { renderToString } from 'react-dom/server';\n");
    // Make these available to runtime functions via globals
    entry.push_str("var __rex_createElement = createElement;\n");
    entry.push_str("var __rex_renderToString = renderToString;\n\n");

    // Import server-side head runtime (side effect: sets up globalThis.__rex_head_elements)
    entry.push_str("import 'rex/head';\n\n");

    // Import and register pages
    entry.push_str("globalThis.__rex_pages = {};\n");
    for (i, route) in scan.routes.iter().enumerate() {
        let effective_path = page_overrides
            .get(&route.abs_path)
            .unwrap_or(&route.abs_path);
        let page_path = effective_path.to_string_lossy().replace('\\', "/");
        let module_name = route.module_name();
        entry.push_str(&format!("import * as __page{i} from '{page_path}';\n"));
        entry.push_str(&format!(
            "globalThis.__rex_pages['{module_name}'] = __page{i};\n"
        ));
    }

    // Special pages (404, _error)
    for (label, route_opt) in [("404", &scan.not_found), ("_error", &scan.error)] {
        if let Some(route) = route_opt {
            let effective_path = page_overrides
                .get(&route.abs_path)
                .unwrap_or(&route.abs_path);
            let page_path = effective_path.to_string_lossy().replace('\\', "/");
            entry.push_str(&format!("import * as __page_{label} from '{page_path}';\n"));
            entry.push_str(&format!(
                "globalThis.__rex_pages['{label}'] = __page_{label};\n"
            ));
        }
    }

    // API routes
    if !scan.api_routes.is_empty() {
        entry.push_str("\nglobalThis.__rex_api_handlers = {};\n");
        for (i, route) in scan.api_routes.iter().enumerate() {
            let api_path = route.abs_path.to_string_lossy().replace('\\', "/");
            let module_name = route.module_name();
            entry.push_str(&format!("import * as __api{i} from '{api_path}';\n"));
            entry.push_str(&format!(
                "globalThis.__rex_api_handlers['{module_name}'] = __api{i};\n"
            ));
        }
    }

    // App router route handlers (app/**/route.ts)
    if let Some(app_scan) = &scan.app_scan {
        if !app_scan.api_routes.is_empty() {
            entry.push_str("\nglobalThis.__rex_app_route_handlers = {};\n");
            for (i, route) in app_scan.api_routes.iter().enumerate() {
                let handler_path = route.handler_path.to_string_lossy().replace('\\', "/");
                let pattern = &route.pattern;
                entry.push_str(&format!(
                    "import * as __app_route{i} from '{handler_path}';\n"
                ));
                entry.push_str(&format!(
                    "globalThis.__rex_app_route_handlers['{pattern}'] = __app_route{i};\n"
                ));
            }
        }
    }

    // _app
    if let Some(app) = &scan.app {
        let effective_app = page_overrides.get(&app.abs_path).unwrap_or(&app.abs_path);
        let app_path = effective_app.to_string_lossy().replace('\\', "/");
        entry.push_str(&format!("\nimport * as __app from '{app_path}';\n"));
        entry.push_str("globalThis.__rex_app = __app;\n");
    }

    // _document (imports rex/document which sets up __rex_render_document)
    if let Some(doc) = &scan.document {
        entry.push_str("\nimport 'rex/document';\n");
        let effective_doc = page_overrides.get(&doc.abs_path).unwrap_or(&doc.abs_path);
        let doc_path = effective_doc.to_string_lossy().replace('\\', "/");
        entry.push_str(&format!("import * as __doc from '{doc_path}';\n"));
        entry.push_str("globalThis.__rex_document = __doc;\n");
    }

    // Middleware (if middleware.ts exists at project root)
    if let Some(mw_path) = &scan.middleware {
        let path = mw_path.to_string_lossy().replace('\\', "/");
        entry.push_str(&format!("\nimport * as __middleware from '{path}';\n"));
        entry.push_str("globalThis.__rex_middleware = __middleware;\n");
    }

    // MCP tools (if mcp/ directory has tool files)
    if !scan.mcp_tools.is_empty() {
        entry.push_str("\nglobalThis.__rex_mcp_tools = {};\n");
        for (i, tool) in scan.mcp_tools.iter().enumerate() {
            let tool_path = tool.abs_path.to_string_lossy().replace('\\', "/");
            let tool_name = &tool.name;
            entry.push_str(&format!("import * as __mcp{i} from '{tool_path}';\n"));
            entry.push_str(&format!(
                "globalThis.__rex_mcp_tools['{tool_name}'] = __mcp{i};\n"
            ));
        }
    }

    // SSR runtime functions
    entry.push_str(SSR_RUNTIME);

    // App route handler runtime (only when app route.ts files exist)
    if scan
        .app_scan
        .as_ref()
        .is_some_and(|a| !a.api_routes.is_empty())
    {
        entry.push_str(APP_ROUTE_RUNTIME);
    }

    // Middleware runtime (only when middleware exists)
    if scan.middleware.is_some() {
        entry.push_str(MIDDLEWARE_RUNTIME);
    }

    // MCP runtime (only when mcp/ tools exist)
    if !scan.mcp_tools.is_empty() {
        entry.push_str(MCP_RUNTIME);
    }

    let entry_path = entries_dir.join("server-entry.js");
    fs::write(&entry_path, &entry)?;

    // Non-JS assets → empty/binary modules (server doesn't need these)
    let mut module_types = rustc_hash::FxHashMap::default();
    for ext in &[".css", ".scss", ".sass", ".less", ".mdx", ".svg", ".wasm"] {
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

    // Rex built-in aliases, then Node.js polyfills, then user aliases
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
        ("next/document", "document.ts"),
    ];
    let mut aliases: Vec<_> = rex_aliases.iter().map(|(s, f)| make_alias(s, f)).collect();
    // Node.js polyfills + next/* shims from shared helper
    aliases.extend(crate::build_utils::node_polyfill_aliases(&runtime_dir));

    // Append user-defined aliases from rex.config build.alias
    aliases.extend(project_config.build.resolved_aliases(&config.project_root));
    // tsconfig auto-resolution is disabled (to prevent rex/* overrides), so we
    // manually parse tsconfig paths for user aliases like @/ → src/.
    aliases.extend(crate::build_utils::tsconfig_path_aliases(
        &config.project_root,
    ));

    let options = rolldown::BundlerOptions {
        input: Some(vec![rolldown::InputItem {
            name: Some("server-bundle".to_string()),
            import: entry_path.to_string_lossy().to_string(),
        }]),
        cwd: Some(config.project_root.clone()),
        format: Some(rolldown::OutputFormat::Iife),
        dir: Some(output_dir.to_string_lossy().to_string()),
        entry_filenames: Some("server-bundle.js".to_string().into()),
        platform: Some(rolldown::Platform::Browser),
        module_types: Some(module_types),
        minify: minify.clone(),
        define: Some(define.iter().cloned().collect()),
        banner: Some(rolldown::AddonOutputOption::String(Some(
            V8_POLYFILLS.to_string(),
        ))),
        // Disable tsconfig path resolution for the server bundle — we provide
        // explicit resolve.alias entries for rex/* stubs.  tsconfig.json `paths`
        // (e.g. "rex/*" → "@limlabs/rex/src/*") would otherwise shadow the
        // server-safe stubs with the client-only package source, causing
        // "window is not defined" at SSR time.
        tsconfig: Some(rolldown_common::TsConfig::Auto(false)),
        shim_missing_exports: Some(true),
        treeshake: crate::rsc_build_config::react_treeshake_options(),
        resolve: Some(rolldown::ResolveOptions {
            alias: Some(aliases),
            // Use "node" condition so packages with conditional exports (e.g. file-type)
            // resolve to their Node.js entry point with full API surface.
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
        sourcemap: if project_config.build.sourcemap {
            Some(rolldown::SourceMapType::File)
        } else {
            None
        },
        ..Default::default()
    };

    // Plugin to intercept imports that resolve.alias misses (e.g. from within
    // node_modules via pnpm symlinks). Aliases only match project-root imports;
    // this plugin catches the rest.
    let stub = runtime_dir
        .join("file-type.ts")
        .to_string_lossy()
        .to_string();
    let empty_stub = runtime_dir.join("empty.ts").to_string_lossy().to_string();
    let polyfill_plugin = Arc::new(NodePolyfillResolvePlugin::new(
        vec![
            ("file-type".to_string(), stub),
            ("@vercel/og".to_string(), empty_stub.clone()),
            (
                "next/dist/compiled/@vercel/og".to_string(),
                empty_stub.clone(),
            ),
            ("next/og".to_string(), empty_stub.clone()),
        ],
        empty_stub,
    ));

    let mut bundler = rolldown::Bundler::with_plugins(
        options,
        vec![polyfill_plugin as Arc<dyn rolldown::plugin::Pluginable>],
    )
    .map_err(|e| anyhow::anyhow!("Failed to create rolldown bundler: {e}"))?;

    if let Err(e) = bundler.write().await {
        // Allow MissingExport diagnostics when shim_missing_exports is on
        if !crate::diagnostics::is_all_missing_exports(&e) {
            return Err(anyhow::anyhow!(
                "Server bundle failed:\n{}",
                crate::diagnostics::format_build_diagnostics(&e)
            ));
        }
        tracing::warn!("Server bundle had {} shimmed missing export(s)", e.len());
    }

    let _ = fs::remove_dir_all(&entries_dir);

    let bundle_path = output_dir.join("server-bundle.js");
    debug!(path = %bundle_path.display(), "Server bundle written");
    Ok(bundle_path)
}
