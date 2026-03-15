//! RSC module graph analysis.
//!
//! Walks import graphs from app/ entry points, detects `"use client"` boundaries,
//! and produces a split: server-only modules vs client boundary modules.

use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

/// Information about a single module in the graph.
#[derive(Debug, Clone)]
pub struct ModuleInfo {
    pub path: PathBuf,
    /// Whether this module has `"use client"` directive.
    pub is_client: bool,
    /// Whether this module has `"use server"` directive (module-level).
    pub is_server: bool,
    /// Whether this module imports dynamic request functions (`cookies`, `headers`)
    /// from `rex/actions`, which forces the route to be server-rendered.
    pub uses_dynamic_functions: bool,
    /// Resolved import paths from this module.
    pub imports: Vec<PathBuf>,
    /// Export names from this module.
    pub exports: Vec<String>,
    /// Exported functions that have function-level `"use server"` directives.
    /// Only populated for modules without a module-level `"use server"` directive.
    pub server_functions: Vec<String>,
    /// Whether this module contains `"use server"` strings beyond what was
    /// extracted into `server_functions`. True when inline server actions
    /// are used inside JSX expressions — these can't be extracted by Rex yet.
    pub has_unextracted_server_directives: bool,
}

/// The analyzed module graph.
#[derive(Debug, Default)]
pub struct ModuleGraph {
    pub modules: HashMap<PathBuf, ModuleInfo>,
}

impl ModuleGraph {
    /// Return all modules that have `"use client"` and are imported by a server module.
    pub fn client_boundary_modules(&self) -> Vec<&ModuleInfo> {
        self.modules.values().filter(|m| m.is_client).collect()
    }

    /// Return all modules that are server-only (no `"use client"`).
    pub fn server_modules(&self) -> Vec<&ModuleInfo> {
        self.modules.values().filter(|m| !m.is_client).collect()
    }

    /// Return all modules that have `"use server"` directive (module-level).
    pub fn server_action_modules(&self) -> Vec<&ModuleInfo> {
        self.modules.values().filter(|m| m.is_server).collect()
    }

    /// Return all modules that have function-level `"use server"` directives
    /// (without a module-level directive).
    pub fn inline_server_action_modules(&self) -> Vec<&ModuleInfo> {
        self.modules
            .values()
            .filter(|m| !m.is_server && !m.server_functions.is_empty())
            .collect()
    }

    /// Return modules with inline `"use server"` directives that Rex couldn't
    /// extract (e.g. server actions defined inside JSX expressions).
    pub fn unextracted_server_action_modules(&self) -> Vec<&ModuleInfo> {
        self.modules
            .values()
            .filter(|m| m.has_unextracted_server_directives)
            .collect()
    }

    /// Check whether any server component reachable from the given entry points
    /// uses dynamic functions (`cookies()`, `headers()` from `rex/actions`).
    ///
    /// This is used for automatic static optimization: if any module in a route's
    /// component tree uses dynamic functions, the route must be server-rendered.
    pub fn has_dynamic_functions(&self, entry_paths: &[PathBuf]) -> bool {
        let mut visited = HashSet::new();
        let mut queue: VecDeque<&PathBuf> = entry_paths.iter().collect();

        while let Some(path) = queue.pop_front() {
            if !visited.insert(path) {
                continue;
            }
            if let Some(info) = self.modules.get(path) {
                if info.uses_dynamic_functions {
                    return true;
                }
                // Only traverse server modules (stop at client boundaries)
                if !info.is_client {
                    for import in &info.imports {
                        queue.push_back(import);
                    }
                }
            }
        }
        false
    }
}

/// Check if a source file has a `"use server"` directive.
pub fn has_use_server_directive(source: &str, source_type: oxc_span::SourceType) -> bool {
    let allocator = oxc_allocator::Allocator::default();
    let parsed = oxc_parser::Parser::new(&allocator, source, source_type).parse();
    parsed
        .program
        .directives
        .iter()
        .any(|d| d.directive.as_str() == "use server")
}

/// Check if a source file has a `"use client"` directive.
pub fn has_use_client_directive(source: &str, source_type: oxc_span::SourceType) -> bool {
    let allocator = oxc_allocator::Allocator::default();
    let parsed = oxc_parser::Parser::new(&allocator, source, source_type).parse();
    parsed
        .program
        .directives
        .iter()
        .any(|d| d.directive.as_str() == "use client")
}

/// Check if a function body has a `"use server"` directive.
fn has_function_body_use_server(
    body: Option<&oxc_allocator::Box<oxc_ast::ast::FunctionBody>>,
) -> bool {
    body.is_some_and(|b| {
        b.directives
            .iter()
            .any(|d| d.directive.as_str() == "use server")
    })
}

/// Check if an expression (arrow function or function expression) has `"use server"`.
fn has_expression_use_server(expr: &oxc_ast::ast::Expression) -> bool {
    match expr {
        oxc_ast::ast::Expression::ArrowFunctionExpression(arrow) => arrow
            .body
            .directives
            .iter()
            .any(|d| d.directive.as_str() == "use server"),
        oxc_ast::ast::Expression::FunctionExpression(func) => {
            has_function_body_use_server(func.body.as_ref())
        }
        _ => false,
    }
}

/// File extensions that are non-code assets — skip during module graph analysis.
const ASSET_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "ico", "avif", "bmp", "tiff", "svg", "css", "scss",
    "sass", "less", "woff", "woff2", "ttf", "eot", "otf", "mp3", "mp4", "wav", "ogg", "webm",
    "pdf", "json", "mdx",
];

/// Detect `"use client"` directive and extract exports from a source file.
fn analyze_module(path: &Path, root: &Path) -> Result<ModuleInfo> {
    // Skip non-code assets (binary files, stylesheets, fonts, MDX, etc.)
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        if ASSET_EXTENSIONS.contains(&ext.to_lowercase().as_str()) {
            return Ok(ModuleInfo {
                path: path.to_path_buf(),
                is_client: false,
                is_server: false,
                uses_dynamic_functions: false,
                imports: Vec::new(),
                exports: vec!["default".to_string()],
                server_functions: Vec::new(),
                has_unextracted_server_directives: false,
            });
        }
    }

    let source = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;

    let source_type = match path.extension().and_then(|e| e.to_str()) {
        Some("tsx") => oxc_span::SourceType::tsx(),
        Some("ts") => oxc_span::SourceType::ts(),
        Some("jsx") => oxc_span::SourceType::jsx(),
        _ => oxc_span::SourceType::mjs(),
    };

    let allocator = oxc_allocator::Allocator::default();
    let parsed = oxc_parser::Parser::new(&allocator, &source, source_type).parse();

    let is_client = parsed
        .program
        .directives
        .iter()
        .any(|d| d.directive.as_str() == "use client");

    let is_server = parsed
        .program
        .directives
        .iter()
        .any(|d| d.directive.as_str() == "use server");

    if is_client && is_server {
        anyhow::bail!(
            "Module {} has both \"use client\" and \"use server\" directives",
            path.display()
        );
    }

    let mut imports = Vec::new();
    let mut exports = Vec::new();
    let mut server_functions = Vec::new();
    let mut uses_dynamic_functions = false;

    for stmt in &parsed.program.body {
        // Collect imports and detect dynamic function usage
        if let oxc_ast::ast::Statement::ImportDeclaration(import) = stmt {
            let specifier = import.source.value.as_str();

            // Detect dynamic function imports from rex/actions (cookies, headers)
            if specifier == "rex/actions" || specifier == "next/headers" {
                if let Some(specifiers) = &import.specifiers {
                    for spec in specifiers {
                        if let oxc_ast::ast::ImportDeclarationSpecifier::ImportSpecifier(s) = spec {
                            let name = s.imported.name();
                            if name == "cookies" || name == "headers" {
                                uses_dynamic_functions = true;
                            }
                        }
                    }
                }
            }

            if let Some(resolved) = resolve_import(path, specifier, root) {
                imports.push(resolved);
            }
        }

        // Collect export names and detect function-level "use server"
        match stmt {
            oxc_ast::ast::Statement::ExportDefaultDeclaration(export) => {
                exports.push("default".to_string());
                // Check if the default export is a function with "use server"
                if !is_server {
                    if let oxc_ast::ast::ExportDefaultDeclarationKind::FunctionDeclaration(ref f) =
                        export.declaration
                    {
                        if has_function_body_use_server(f.body.as_ref()) {
                            server_functions.push("default".to_string());
                        }
                    }
                }
            }
            oxc_ast::ast::Statement::ExportNamedDeclaration(export) => {
                for spec in &export.specifiers {
                    exports.push(spec.exported.name().to_string());
                }
                // Also check for `export function Foo()` / `export const Foo = ...`
                if let Some(ref decl) = export.declaration {
                    match decl {
                        oxc_ast::ast::Declaration::FunctionDeclaration(f) => {
                            if let Some(ref id) = f.id {
                                let name = id.name.to_string();
                                exports.push(name.clone());
                                if !is_server && has_function_body_use_server(f.body.as_ref()) {
                                    server_functions.push(name);
                                }
                            }
                        }
                        oxc_ast::ast::Declaration::ClassDeclaration(c) => {
                            if let Some(ref id) = c.id {
                                exports.push(id.name.to_string());
                            }
                        }
                        oxc_ast::ast::Declaration::VariableDeclaration(v) => {
                            for vardecl in &v.declarations {
                                if let oxc_ast::ast::BindingPattern::BindingIdentifier(ref id) =
                                    vardecl.id
                                {
                                    let name = id.name.to_string();
                                    exports.push(name.clone());
                                    // Check arrow/function expressions for "use server"
                                    if !is_server {
                                        if let Some(ref init) = vardecl.init {
                                            if has_expression_use_server(init) {
                                                server_functions.push(name);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    // Detect unextracted "use server" directives: if the source text contains
    // the string "use server" beyond the module-level directive and any extracted
    // function-level directives, there are inline server actions in JSX that Rex
    // can't extract. This produces a build-time warning.
    let has_unextracted_server_directives = if !is_server && !is_client {
        // Count how many times "use server" appears in the source
        let total_occurrences =
            source.matches("\"use server\"").count() + source.matches("'use server'").count();
        // Subtract the ones we already extracted
        total_occurrences > server_functions.len()
    } else {
        false
    };

    Ok(ModuleInfo {
        path: path.to_path_buf(),
        is_client,
        is_server,
        uses_dynamic_functions,
        imports,
        exports,
        server_functions,
        has_unextracted_server_directives,
    })
}

/// Try resolving a candidate path with extension guessing and index fallback.
fn try_resolve_path(candidate: &Path) -> Option<PathBuf> {
    if candidate.exists() && candidate.is_file() {
        return candidate.canonicalize().ok();
    }
    let extensions = ["tsx", "ts", "jsx", "js", "mdx"];
    for ext in &extensions {
        // Use with_file_name to append the extension rather than replace it.
        // `with_extension` replaces the last extension, so `Component.client`
        // would become `Component.tsx` instead of `Component.client.tsx`.
        let file_name = candidate.file_name()?.to_str()?;
        let with_ext = candidate.with_file_name(format!("{file_name}.{ext}"));
        if with_ext.exists() && with_ext.is_file() {
            return with_ext.canonicalize().ok();
        }
    }
    if candidate.is_dir() {
        for ext in &extensions {
            let index = candidate.join(format!("index.{ext}"));
            if index.exists() && index.is_file() {
                return index.canonicalize().ok();
            }
        }
    }
    None
}

/// Resolve a relative import specifier to an absolute path.
///
/// Handles: relative paths (`./Foo`, `../Foo`), with extension guessing
/// for `.tsx`, `.ts`, `.jsx`, `.js`, and `/index.tsx` etc.
/// Also resolves `rex/*` built-in aliases via node_modules so that
/// `"use client"` directives on e.g. `rex/link` are detected.
/// Does NOT resolve other bare specifiers (e.g., `react`) — those are external.
fn resolve_import(from: &Path, specifier: &str, root: &Path) -> Option<PathBuf> {
    // Handle rex/* built-in aliases — resolve through node_modules/@limlabs/rex
    // to match rolldown's resolution (which follows the symlink from the fixture).
    // Falls back to the runtime/client/ directory when the npm package isn't installed
    // (e.g. the docs site which doesn't depend on @limlabs/rex).
    if let Some(name) = specifier.strip_prefix("rex/") {
        let pkg_src = root.join("node_modules/@limlabs/rex/src");
        for ext in &["tsx", "ts", "jsx", "js"] {
            let candidate = pkg_src.join(format!("{name}.{ext}"));
            if candidate.exists() && candidate.is_file() {
                return candidate.canonicalize().ok();
            }
        }
        // Fallback: resolve from the Rex runtime client directory.
        // This ensures "use client" directives on rex/link etc. are detected
        // even when @limlabs/rex isn't in node_modules.
        if let Ok(client_dir) = crate::build_utils::runtime_client_dir() {
            for ext in &["tsx", "ts", "jsx", "js"] {
                let candidate = client_dir.join(format!("{name}.{ext}"));
                if candidate.exists() && candidate.is_file() {
                    return candidate.canonicalize().ok();
                }
            }
        }
        return None;
    }

    // Try tsconfig path aliases (e.g. @/ → src/)
    // Sort by prefix length descending so longer (more specific) prefixes match
    // first — e.g. "@payload-config" matches before "@".
    let mut aliases: Vec<_> = crate::build_utils::tsconfig_path_aliases(root)
        .into_iter()
        .collect();
    aliases.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    for (prefix, targets) in &aliases {
        if specifier.starts_with(prefix) {
            if let Some(Some(target)) = targets.first() {
                let rest = &specifier[prefix.len()..];
                let candidate = PathBuf::from(format!("{target}{rest}"));
                if let Some(resolved) = try_resolve_path(&candidate) {
                    return Some(resolved);
                }
            }
        }
    }

    // Resolve bare specifiers through node_modules to detect "use client"
    // boundaries in third-party packages (e.g. @payloadcms/ui).
    if !specifier.starts_with('.') {
        return resolve_bare_specifier(specifier, root);
    }

    let dir = from.parent()?;
    let candidate = dir.join(specifier);
    try_resolve_path(&candidate)
}

/// Resolve a bare specifier (e.g. `@payloadcms/ui`, `react-datepicker`)
/// through node_modules. Uses package.json `exports`/`main`/`module` fields.
///
/// Only resolves the entry point — does NOT recurse into the package's
/// internal dependencies. This is sufficient for detecting `"use client"`
/// at the package boundary.
fn resolve_bare_specifier(specifier: &str, root: &Path) -> Option<PathBuf> {
    // Split into package name and optional subpath
    // e.g. "@payloadcms/ui/dist/foo" → ("@payloadcms/ui", "dist/foo")
    // e.g. "react-datepicker" → ("react-datepicker", "")
    let (pkg_name, subpath) = split_bare_specifier(specifier);

    // Walk up from root to find node_modules containing this package
    let pkg_dir = find_package_dir(root, pkg_name)?;
    let pkg_json_path = pkg_dir.join("package.json");

    if !subpath.is_empty() {
        // Direct subpath: resolve as a file within the package
        let candidate = pkg_dir.join(subpath);
        return try_resolve_path(&candidate);
    }

    // Read package.json to find the entry point
    let pkg_json = std::fs::read_to_string(&pkg_json_path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&pkg_json).ok()?;

    // Try `exports["."]` first (modern packages)
    if let Some(exports) = parsed.get("exports") {
        if let Some(entry) = resolve_exports_entry(exports) {
            let candidate = pkg_dir.join(entry);
            if let Some(resolved) = try_resolve_path(&candidate) {
                return Some(resolved);
            }
        }
    }

    // Fall back to `module` (ESM) then `main` (CJS)
    for field in &["module", "main"] {
        if let Some(entry) = parsed.get(field).and_then(|v| v.as_str()) {
            let candidate = pkg_dir.join(entry);
            if let Some(resolved) = try_resolve_path(&candidate) {
                return Some(resolved);
            }
        }
    }

    // Last resort: index.js
    try_resolve_path(&pkg_dir.join("index"))
}

/// Split a bare specifier into (package_name, subpath).
fn split_bare_specifier(specifier: &str) -> (&str, &str) {
    if let Some(after_at) = specifier.strip_prefix('@') {
        // Scoped package: @scope/name/subpath
        if let Some(pos) = after_at.find('/') {
            let after_scope = pos + 1; // position in after_at
            if after_scope + 1 > after_at.len() {
                return (specifier, "");
            }
            if let Some(pos2) = after_at[after_scope + 1..].find('/') {
                // +1 for '@' prefix
                let split = 1 + after_scope + 1 + pos2;
                return (&specifier[..split], &specifier[split + 1..]);
            }
            return (specifier, "");
        }
        (specifier, "")
    } else {
        // Regular package: name/subpath
        if let Some(pos) = specifier.find('/') {
            (&specifier[..pos], &specifier[pos + 1..])
        } else {
            (specifier, "")
        }
    }
}

/// Find the package directory in node_modules, walking up from root.
/// Handles pnpm symlinked node_modules by following symlinks.
fn find_package_dir(root: &Path, pkg_name: &str) -> Option<PathBuf> {
    let mut dir = root.to_path_buf();
    loop {
        let candidate = dir.join("node_modules").join(pkg_name);
        if candidate.exists() {
            // Follow symlinks (pnpm uses them)
            return candidate.canonicalize().ok();
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Extract the entry point from a package.json `exports` field.
/// Handles common patterns: string, `{".": ...}`, `{"import": ...}`, `{"default": ...}`.
fn resolve_exports_entry(exports: &serde_json::Value) -> Option<&str> {
    match exports {
        serde_json::Value::String(s) => Some(s.as_str()),
        serde_json::Value::Object(obj) => {
            // Try "." entry first
            if let Some(dot) = obj.get(".") {
                return resolve_exports_entry(dot);
            }
            // Try condition names in priority order
            for key in &["import", "require", "default"] {
                if let Some(val) = obj.get(*key) {
                    return resolve_exports_entry(val);
                }
            }
            None
        }
        _ => None,
    }
}

/// Analyze the module graph starting from the given entry points.
///
/// Performs a BFS walk of imports. Stops at:
/// - External (bare) specifiers (e.g., `react`, `next/link`)
/// - Already-visited modules
///
/// The resulting graph contains all reachable modules with their
/// `is_client` flag and exports.
pub fn analyze_module_graph(entries: &[PathBuf], root: &Path) -> Result<ModuleGraph> {
    let mut graph = ModuleGraph::default();
    let mut queue: VecDeque<PathBuf> = VecDeque::new();
    let mut visited: HashSet<PathBuf> = HashSet::new();

    // Seed with entry points
    for entry in entries {
        let canonical = entry
            .canonicalize()
            .with_context(|| format!("Failed to canonicalize {}", entry.display()))?;
        if visited.insert(canonical.clone()) {
            queue.push_back(canonical);
        }
    }

    while let Some(path) = queue.pop_front() {
        let info = analyze_module(&path, root)?;

        // For node_modules files without "use client", don't recurse into
        // their dependencies — that would be expensive and unnecessary.
        // We only enter node_modules to detect "use client" at the boundary.
        let in_node_modules = path.components().any(|c| c.as_os_str() == "node_modules");

        if !info.is_client && !in_node_modules {
            // Server modules (user code): walk all imports normally.
            for import in &info.imports {
                if !visited.contains(import) {
                    visited.insert(import.clone());
                    queue.push_back(import.clone());
                }
            }
        } else if info.is_client {
            // Client boundary modules: don't fully recurse, but check imports
            // for "use server" modules so we can generate action stubs.
            // Also do a shallow scan for additional "use client" boundaries
            // in node_modules (e.g. re-exported components from third-party
            // packages like Radix, PayloadCMS, etc.).
            for import in &info.imports {
                if !visited.contains(import) {
                    if let Ok(dep_info) = analyze_module(import, root) {
                        if dep_info.is_server || dep_info.is_client {
                            visited.insert(import.clone());
                            graph.modules.insert(import.clone(), dep_info);
                        }
                    }
                }
            }
        }

        graph.modules.insert(path, info);
    }

    Ok(graph)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
#[path = "rsc_graph_tests.rs"]
mod tests;
