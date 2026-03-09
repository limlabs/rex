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

    Ok(ModuleInfo {
        path: path.to_path_buf(),
        is_client,
        is_server,
        uses_dynamic_functions,
        imports,
        exports,
        server_functions,
    })
}

/// Try resolving a candidate path with extension guessing and index fallback.
fn try_resolve_path(candidate: &Path) -> Option<PathBuf> {
    if candidate.exists() && candidate.is_file() {
        return candidate.canonicalize().ok();
    }
    let extensions = ["tsx", "ts", "jsx", "js", "mdx"];
    for ext in &extensions {
        let with_ext = candidate.with_extension(ext);
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
    if let Some(name) = specifier.strip_prefix("rex/") {
        let pkg_src = root.join("node_modules/@limlabs/rex/src");
        for ext in &["tsx", "ts", "jsx", "js"] {
            let candidate = pkg_src.join(format!("{name}.{ext}"));
            if candidate.exists() && candidate.is_file() {
                return candidate.canonicalize().ok();
            }
        }
        return None;
    }

    // Try tsconfig path aliases (e.g. @/ → src/)
    let aliases = crate::build_utils::tsconfig_path_aliases(root);
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
            for import in &info.imports {
                if !visited.contains(import) {
                    if let Ok(dep_info) = analyze_module(import, root) {
                        if dep_info.is_server {
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
mod tests {
    use super::*;
    use std::fs;

    fn setup_temp_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn resolve_import_with_extension_guessing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        std::fs::write(root.join("Foo.tsx"), "export default function Foo() {}").unwrap();

        let from = root.join("page.tsx");
        let resolved = resolve_import(&from, "./Foo", root);
        assert!(resolved.is_some());
        assert!(resolved.unwrap().ends_with("Foo.tsx"));
    }

    #[test]
    fn resolve_import_ignores_bare_specifiers() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let from = root.join("page.tsx");
        assert!(resolve_import(&from, "react", root).is_none());
        assert!(resolve_import(&from, "next/link", root).is_none());
    }

    #[test]
    fn graph_walk_stops_at_client_boundary() {
        let dir = setup_temp_dir();
        let root = dir.path();

        // Server page imports client Counter
        fs::write(
            root.join("page.tsx"),
            r#"
import Counter from './Counter';
export default function Page() { return <Counter />; }
"#,
        )
        .unwrap();

        // Client Counter imports a client-only helper
        fs::write(
            root.join("Counter.tsx"),
            r#"
"use client";
import { format } from './client-utils';
export default function Counter() { return <button>{format(0)}</button>; }
"#,
        )
        .unwrap();

        // Client-only helper (should NOT be in the graph)
        fs::write(
            root.join("client-utils.tsx"),
            r#"
export function format(n: number) { return `Count: ${n}`; }
"#,
        )
        .unwrap();

        let entries = vec![root.join("page.tsx")];
        let graph = analyze_module_graph(&entries, root).unwrap();

        // Only page (server) and Counter (client boundary) should be in the graph.
        // client-utils.tsx should NOT be included — the walk stops at the client boundary.
        assert_eq!(graph.modules.len(), 2);

        let counter_canonical = root.join("Counter.tsx").canonicalize().unwrap();
        let counter = graph.modules.get(&counter_canonical).unwrap();
        assert!(counter.is_client);

        // Verify client-utils is not in the graph
        let utils_canonical = root.join("client-utils.tsx").canonicalize().unwrap();
        assert!(
            !graph.modules.contains_key(&utils_canonical),
            "client-utils.tsx should not be in the server module graph"
        );
    }

    #[test]
    fn detects_use_server_directive() {
        let source = r#"
"use server";

export async function increment(n: number): Promise<number> {
    return n + 1;
}
"#;
        assert!(has_use_server_directive(source, oxc_span::SourceType::ts()));
    }

    #[test]
    fn no_use_server_for_regular_module() {
        let source = r#"
export function add(a: number, b: number) { return a + b; }
"#;
        assert!(!has_use_server_directive(
            source,
            oxc_span::SourceType::ts()
        ));
    }

    #[test]
    fn use_server_must_be_directive() {
        // "use server" as a regular string expression (not a directive)
        let source = r#"
const x = "use server";
export function add(a: number, b: number) { return a + b; }
"#;
        assert!(!has_use_server_directive(
            source,
            oxc_span::SourceType::ts()
        ));
    }

    #[test]
    fn use_client_and_use_server_conflict() {
        let dir = setup_temp_dir();
        let root = dir.path();

        fs::write(
            root.join("conflict.tsx"),
            "\"use client\";\n\"use server\";\nexport default function Foo() { return null; }\n",
        )
        .unwrap();

        let entries = vec![root.join("conflict.tsx")];
        let result = analyze_module_graph(&entries, root);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("\"use client\""));
        assert!(err_msg.contains("\"use server\""));
    }

    #[test]
    fn graph_walk_continues_through_use_server() {
        let dir = setup_temp_dir();
        let root = dir.path();

        // Server page imports a "use server" actions module
        fs::write(
            root.join("page.tsx"),
            "import { increment } from './actions';\nexport default function Page() { return null; }\n",
        )
        .unwrap();

        // "use server" module imports a helper
        fs::write(
            root.join("actions.ts"),
            "\"use server\";\nimport { db } from './db';\nexport async function increment(n: number) { return db.inc(n); }\n",
        )
        .unwrap();

        // Server-only helper (should be in the graph because "use server" is server code)
        fs::write(
            root.join("db.ts"),
            "export const db = { inc: (n: number) => n + 1 };\n",
        )
        .unwrap();

        let entries = vec![root.join("page.tsx")];
        let graph = analyze_module_graph(&entries, root).unwrap();

        // All three modules should be in the graph
        assert_eq!(graph.modules.len(), 3);

        let actions_canonical = root.join("actions.ts").canonicalize().unwrap();
        let actions = graph.modules.get(&actions_canonical).unwrap();
        assert!(actions.is_server);
        assert!(!actions.is_client);

        let db_canonical = root.join("db.ts").canonicalize().unwrap();
        assert!(graph.modules.contains_key(&db_canonical));
    }

    #[test]
    fn server_action_modules_method() {
        let dir = setup_temp_dir();
        let root = dir.path();

        fs::write(
            root.join("page.tsx"),
            "import { inc } from './actions';\nexport default function Page() { return null; }\n",
        )
        .unwrap();

        fs::write(
            root.join("actions.ts"),
            "\"use server\";\nexport async function inc(n: number) { return n + 1; }\n",
        )
        .unwrap();

        let entries = vec![root.join("page.tsx")];
        let graph = analyze_module_graph(&entries, root).unwrap();

        let sa_modules = graph.server_action_modules();
        assert_eq!(sa_modules.len(), 1);
        assert!(sa_modules[0].path.ends_with("actions.ts"));
        assert!(sa_modules[0].exports.contains(&"inc".to_string()));
    }

    #[test]
    fn client_component_importing_use_server_module() {
        let dir = setup_temp_dir();
        let root = dir.path();

        // Server component page imports a client component
        fs::write(
            root.join("page.tsx"),
            "import Counter from './Counter';\nexport default function Page() { return null; }\n",
        )
        .unwrap();

        // Client component imports from a "use server" module
        fs::write(
            root.join("Counter.tsx"),
            "\"use client\";\nimport { increment } from './actions';\nexport default function Counter() { return null; }\n",
        )
        .unwrap();

        // "use server" module
        fs::write(
            root.join("actions.ts"),
            "\"use server\";\nexport async function increment(n: number) { return n + 1; }\n",
        )
        .unwrap();

        let entries = vec![root.join("page.tsx")];
        let graph = analyze_module_graph(&entries, root).unwrap();

        // page.tsx, Counter.tsx (client boundary), and actions.ts (server action) should all be in graph
        assert_eq!(graph.modules.len(), 3);

        let actions_canonical = root.join("actions.ts").canonicalize().unwrap();
        let actions = graph.modules.get(&actions_canonical).unwrap();
        assert!(actions.is_server);
        assert_eq!(actions.exports, vec!["increment"]);

        // server_action_modules should include actions.ts
        let sa_modules = graph.server_action_modules();
        assert_eq!(sa_modules.len(), 1);
        assert!(sa_modules[0].path.ends_with("actions.ts"));
    }

    #[test]
    fn jsx_file_analyzed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(
            root.join("comp.jsx"),
            "export default function Comp() { return <div>Hello</div>; }\n",
        )
        .unwrap();

        let entries = vec![root.join("comp.jsx")];
        let graph = analyze_module_graph(&entries, root).unwrap();
        assert_eq!(graph.modules.len(), 1);
        let comp = graph.modules.values().next().unwrap();
        assert!(comp.exports.contains(&"default".to_string()));
    }

    #[test]
    fn class_export_detected() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(
            root.join("widget.ts"),
            "\"use client\";\nexport class Widget {}\n",
        )
        .unwrap();

        let entries = vec![root.join("widget.ts")];
        let graph = analyze_module_graph(&entries, root).unwrap();
        let widget = graph.modules.values().next().unwrap();
        assert!(widget.exports.contains(&"Widget".to_string()));
    }

    #[test]
    fn reexport_specifier_detected() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("utils.ts"), "export const helper = 42;\n").unwrap();
        fs::write(root.join("index.ts"), "export { helper } from './utils';\n").unwrap();

        let entries = vec![root.join("index.ts")];
        let graph = analyze_module_graph(&entries, root).unwrap();
        let index_mod = graph
            .modules
            .values()
            .find(|m| m.path.ends_with("index.ts"))
            .unwrap();
        assert!(
            index_mod.exports.contains(&"helper".to_string()),
            "Re-export specifier should be detected, got: {:?}",
            index_mod.exports
        );
    }

    #[test]
    fn unknown_extension_analyzed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("mod.mjs"), "export const value = 1;\n").unwrap();

        let entries = vec![root.join("mod.mjs")];
        let graph = analyze_module_graph(&entries, root).unwrap();
        let mjs_mod = graph.modules.values().next().unwrap();
        assert!(mjs_mod.exports.contains(&"value".to_string()));
    }

    #[test]
    fn function_level_use_server_declaration() {
        let dir = setup_temp_dir();
        let root = dir.path();

        fs::write(
            root.join("page.tsx"),
            r#"
export async function submitForm() {
    "use server";
    return { ok: true };
}

export default function Page() { return null; }
"#,
        )
        .unwrap();

        let entries = vec![root.join("page.tsx")];
        let graph = analyze_module_graph(&entries, root).unwrap();

        let canonical = root.join("page.tsx").canonicalize().unwrap();
        let info = graph.modules.get(&canonical).unwrap();
        assert!(!info.is_server); // no module-level directive
        assert!(info.server_functions.contains(&"submitForm".to_string()));
        assert!(!info.server_functions.contains(&"default".to_string()));
    }

    #[test]
    fn function_level_use_server_arrow() {
        let dir = setup_temp_dir();
        let root = dir.path();

        fs::write(
            root.join("actions.ts"),
            r#"
export const increment = async (n: number) => {
    "use server";
    return n + 1;
};

export const helper = (x: number) => x * 2;
"#,
        )
        .unwrap();

        let entries = vec![root.join("actions.ts")];
        let graph = analyze_module_graph(&entries, root).unwrap();

        let canonical = root.join("actions.ts").canonicalize().unwrap();
        let info = graph.modules.get(&canonical).unwrap();
        assert!(!info.is_server);
        assert!(info.server_functions.contains(&"increment".to_string()));
        assert!(!info.server_functions.contains(&"helper".to_string()));
    }

    #[test]
    fn inline_server_action_modules_method() {
        let dir = setup_temp_dir();
        let root = dir.path();

        fs::write(
            root.join("page.tsx"),
            r#"
export async function submit() {
    "use server";
    return 1;
}
export default function Page() { return null; }
"#,
        )
        .unwrap();

        let entries = vec![root.join("page.tsx")];
        let graph = analyze_module_graph(&entries, root).unwrap();

        let inline = graph.inline_server_action_modules();
        assert_eq!(inline.len(), 1);
        assert!(inline[0].server_functions.contains(&"submit".to_string()));

        // module-level server action modules should be empty
        assert!(graph.server_action_modules().is_empty());
    }

    #[test]
    fn module_level_use_server_overrides_function_level() {
        let dir = setup_temp_dir();
        let root = dir.path();

        // Module-level "use server" — all exports are server actions,
        // function-level detection should be skipped
        fs::write(
            root.join("actions.ts"),
            r#"
"use server";
export async function inc() { return 1; }
export async function dec() { return -1; }
"#,
        )
        .unwrap();

        let entries = vec![root.join("actions.ts")];
        let graph = analyze_module_graph(&entries, root).unwrap();

        let canonical = root.join("actions.ts").canonicalize().unwrap();
        let info = graph.modules.get(&canonical).unwrap();
        assert!(info.is_server);
        assert!(info.server_functions.is_empty()); // not populated when module-level
    }

    #[test]
    fn detects_dynamic_function_cookies_import() {
        let dir = setup_temp_dir();
        let root = dir.path();

        fs::write(
            root.join("page.tsx"),
            r#"
import { cookies } from 'rex/actions';
export default function Page() { return null; }
"#,
        )
        .unwrap();

        let entries = vec![root.join("page.tsx")];
        let graph = analyze_module_graph(&entries, root).unwrap();

        let canonical = root.join("page.tsx").canonicalize().unwrap();
        let info = graph.modules.get(&canonical).unwrap();
        assert!(info.uses_dynamic_functions);
    }

    #[test]
    fn detects_dynamic_function_headers_import() {
        let dir = setup_temp_dir();
        let root = dir.path();

        fs::write(
            root.join("page.tsx"),
            r#"
import { headers } from 'rex/actions';
export default function Page() { return null; }
"#,
        )
        .unwrap();

        let entries = vec![root.join("page.tsx")];
        let graph = analyze_module_graph(&entries, root).unwrap();

        let canonical = root.join("page.tsx").canonicalize().unwrap();
        let info = graph.modules.get(&canonical).unwrap();
        assert!(info.uses_dynamic_functions);
    }

    #[test]
    fn no_dynamic_functions_for_static_page() {
        let dir = setup_temp_dir();
        let root = dir.path();

        fs::write(
            root.join("page.tsx"),
            r#"
export default function Page() { return <div>Hello</div>; }
"#,
        )
        .unwrap();

        let entries = vec![root.join("page.tsx")];
        let graph = analyze_module_graph(&entries, root).unwrap();

        let canonical = root.join("page.tsx").canonicalize().unwrap();
        let info = graph.modules.get(&canonical).unwrap();
        assert!(!info.uses_dynamic_functions);
    }

    #[test]
    fn has_dynamic_functions_traverses_imports() {
        let dir = setup_temp_dir();
        let root = dir.path();

        // Page imports a layout that uses cookies
        fs::write(
            root.join("page.tsx"),
            r#"
import Layout from './layout';
export default function Page() { return null; }
"#,
        )
        .unwrap();

        fs::write(
            root.join("layout.tsx"),
            r#"
import { cookies } from 'rex/actions';
export default function Layout({ children }) { return <div>{children}</div>; }
"#,
        )
        .unwrap();

        let entries = vec![root.join("page.tsx")];
        let graph = analyze_module_graph(&entries, root).unwrap();

        let page_canonical = root.join("page.tsx").canonicalize().unwrap();
        let layout_canonical = root.join("layout.tsx").canonicalize().unwrap();

        // Page itself doesn't use dynamic functions
        assert!(
            !graph
                .modules
                .get(&page_canonical)
                .unwrap()
                .uses_dynamic_functions
        );
        // Layout does
        assert!(
            graph
                .modules
                .get(&layout_canonical)
                .unwrap()
                .uses_dynamic_functions
        );

        // has_dynamic_functions should detect it through the dependency tree
        assert!(graph.has_dynamic_functions(&[page_canonical]));
    }

    #[test]
    fn has_dynamic_functions_stops_at_client_boundary() {
        let dir = setup_temp_dir();
        let root = dir.path();

        // Server page imports a client component
        fs::write(
            root.join("page.tsx"),
            r#"
import Counter from './Counter';
export default function Page() { return null; }
"#,
        )
        .unwrap();

        // Client component (would use cookies, but shouldn't affect server detection)
        fs::write(
            root.join("Counter.tsx"),
            r#"
"use client";
export default function Counter() { return <button>0</button>; }
"#,
        )
        .unwrap();

        let entries = vec![root.join("page.tsx")];
        let graph = analyze_module_graph(&entries, root).unwrap();

        let page_canonical = root.join("page.tsx").canonicalize().unwrap();
        assert!(!graph.has_dynamic_functions(&[page_canonical]));
    }

    #[test]
    fn has_dynamic_functions_empty_entries() {
        let graph = ModuleGraph::default();
        assert!(!graph.has_dynamic_functions(&[]));
    }

    #[test]
    fn other_rex_actions_imports_not_dynamic() {
        let dir = setup_temp_dir();
        let root = dir.path();

        // Import redirect from rex/actions — not a dynamic function
        fs::write(
            root.join("page.tsx"),
            r#"
import { redirect } from 'rex/actions';
export default function Page() { return null; }
"#,
        )
        .unwrap();

        let entries = vec![root.join("page.tsx")];
        let graph = analyze_module_graph(&entries, root).unwrap();

        let canonical = root.join("page.tsx").canonicalize().unwrap();
        let info = graph.modules.get(&canonical).unwrap();
        assert!(!info.uses_dynamic_functions);
    }

    #[test]
    fn mdx_import_does_not_crash() {
        let dir = setup_temp_dir();
        let root = dir.path();

        // Page that imports an MDX file
        fs::write(
            root.join("page.tsx"),
            r#"
import Content from './content.mdx';
export default function Page() { return <Content />; }
"#,
        )
        .unwrap();

        // MDX file with markdown content (not valid JS)
        fs::write(
            root.join("content.mdx"),
            "# Hello World\n\nThis is **markdown** content.\n",
        )
        .unwrap();

        let entries = vec![root.join("page.tsx")];
        let graph = analyze_module_graph(&entries, root).unwrap();

        // Both modules should be in the graph
        assert_eq!(graph.modules.len(), 2);

        // MDX module should have sensible defaults
        let mdx_canonical = root.join("content.mdx").canonicalize().unwrap();
        let mdx_info = graph.modules.get(&mdx_canonical).unwrap();
        assert!(!mdx_info.is_client);
        assert!(!mdx_info.is_server);
        assert!(mdx_info.exports.contains(&"default".to_string()));
        assert!(mdx_info.imports.is_empty());
    }
}
