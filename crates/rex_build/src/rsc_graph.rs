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
    /// Resolved import paths from this module.
    pub imports: Vec<PathBuf>,
    /// Export names from this module.
    pub exports: Vec<String>,
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

/// Detect `"use client"` directive and extract exports from a source file.
fn analyze_module(path: &Path, root: &Path) -> Result<ModuleInfo> {
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

    let mut imports = Vec::new();
    let mut exports = Vec::new();

    for stmt in &parsed.program.body {
        // Collect imports
        if let oxc_ast::ast::Statement::ImportDeclaration(import) = stmt {
            let specifier = import.source.value.as_str();
            if let Some(resolved) = resolve_import(path, specifier, root) {
                imports.push(resolved);
            }
        }

        // Collect export names
        match stmt {
            oxc_ast::ast::Statement::ExportDefaultDeclaration(_) => {
                exports.push("default".to_string());
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
                                exports.push(id.name.to_string());
                            }
                        }
                        oxc_ast::ast::Declaration::ClassDeclaration(c) => {
                            if let Some(ref id) = c.id {
                                exports.push(id.name.to_string());
                            }
                        }
                        oxc_ast::ast::Declaration::VariableDeclaration(v) => {
                            for decl in &v.declarations {
                                if let oxc_ast::ast::BindingPattern::BindingIdentifier(ref id) =
                                    decl.id
                                {
                                    exports.push(id.name.to_string());
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
        imports,
        exports,
    })
}

/// Resolve a relative import specifier to an absolute path.
///
/// Handles: relative paths (`./Foo`, `../Foo`), with extension guessing
/// for `.tsx`, `.ts`, `.jsx`, `.js`, and `/index.tsx` etc.
/// Does NOT resolve bare specifiers (e.g., `react`) — those are external.
fn resolve_import(from: &Path, specifier: &str, _root: &Path) -> Option<PathBuf> {
    // Only resolve relative imports
    if !specifier.starts_with('.') {
        return None;
    }

    let dir = from.parent()?;
    let candidate = dir.join(specifier);

    // If it already has an extension and exists, use it
    if candidate.exists() && candidate.is_file() {
        return candidate.canonicalize().ok();
    }

    // Try standard extensions
    let extensions = ["tsx", "ts", "jsx", "js"];
    for ext in &extensions {
        let with_ext = candidate.with_extension(ext);
        if with_ext.exists() && with_ext.is_file() {
            return with_ext.canonicalize().ok();
        }
    }

    // Try as directory with index file
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

        // Don't walk into client modules' dependencies — they are leaf nodes
        // for the server graph. The client bundler handles their deps separately.
        if !info.is_client {
            for import in &info.imports {
                if !visited.contains(import) {
                    visited.insert(import.clone());
                    queue.push_back(import.clone());
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
    fn detects_use_client_directive() {
        let source = r#"
"use client";

export default function Counter() {
    return <button>Click</button>;
}
"#;
        assert!(has_use_client_directive(
            source,
            oxc_span::SourceType::tsx()
        ));
    }

    #[test]
    fn no_use_client_for_server_component() {
        let source = r#"
export default function Page() {
    return <div>Hello</div>;
}
"#;
        assert!(!has_use_client_directive(
            source,
            oxc_span::SourceType::tsx()
        ));
    }

    #[test]
    fn use_client_must_be_directive() {
        // "use client" as a regular string expression (not a directive)
        let source = r#"
const x = "use client";
export default function Page() { return <div />; }
"#;
        assert!(!has_use_client_directive(
            source,
            oxc_span::SourceType::tsx()
        ));
    }

    #[test]
    fn analyze_simple_graph() {
        let dir = setup_temp_dir();
        let root = dir.path();

        // Server component (page)
        fs::write(
            root.join("page.tsx"),
            r#"
import Counter from './Counter';
export default function Page() { return <Counter />; }
"#,
        )
        .unwrap();

        // Client component
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

        assert_eq!(graph.modules.len(), 2);

        let page_canonical = root.join("page.tsx").canonicalize().unwrap();
        let counter_canonical = root.join("Counter.tsx").canonicalize().unwrap();

        let page = graph.modules.get(&page_canonical).unwrap();
        assert!(!page.is_client);
        assert!(page.exports.contains(&"default".to_string()));

        let counter = graph.modules.get(&counter_canonical).unwrap();
        assert!(counter.is_client);
        assert!(counter.exports.contains(&"default".to_string()));
    }

    #[test]
    fn client_boundary_detection() {
        let dir = setup_temp_dir();
        let root = dir.path();

        // layout (server)
        fs::write(
            root.join("layout.tsx"),
            r#"
import Header from './Header';
export default function Layout({ children }) { return <div><Header />{children}</div>; }
"#,
        )
        .unwrap();

        // Header (server)
        fs::write(
            root.join("Header.tsx"),
            r#"
export default function Header() { return <nav>Nav</nav>; }
"#,
        )
        .unwrap();

        // page (server, imports client)
        fs::write(
            root.join("page.tsx"),
            r#"
import SearchForm from './SearchForm';
export default function Page() { return <SearchForm />; }
"#,
        )
        .unwrap();

        // SearchForm (client)
        fs::write(
            root.join("SearchForm.tsx"),
            r#"
"use client";
import { useState } from 'react';
export default function SearchForm() { return <input />; }
"#,
        )
        .unwrap();

        let entries = vec![root.join("layout.tsx"), root.join("page.tsx")];
        let graph = analyze_module_graph(&entries, root).unwrap();

        let client_boundaries = graph.client_boundary_modules();
        assert_eq!(client_boundaries.len(), 1);
        assert!(client_boundaries[0].path.ends_with("SearchForm.tsx"));

        let server_mods = graph.server_modules();
        assert_eq!(server_mods.len(), 3); // layout, Header, page
    }

    #[test]
    fn named_exports_detection() {
        let dir = setup_temp_dir();
        let root = dir.path();

        fs::write(
            root.join("utils.tsx"),
            r#"
"use client";
export function Counter() { return <div />; }
export const INITIAL = 0;
export default function Main() { return <div />; }
"#,
        )
        .unwrap();

        let entries = vec![root.join("utils.tsx")];
        let graph = analyze_module_graph(&entries, root).unwrap();

        let canonical = root.join("utils.tsx").canonicalize().unwrap();
        let info = graph.modules.get(&canonical).unwrap();
        assert!(info.is_client);
        assert!(info.exports.contains(&"Counter".to_string()));
        assert!(info.exports.contains(&"INITIAL".to_string()));
        assert!(info.exports.contains(&"default".to_string()));
    }

    #[test]
    fn resolve_import_with_extension_guessing() {
        let dir = setup_temp_dir();
        let root = dir.path();

        fs::write(root.join("Foo.tsx"), "export default function Foo() {}").unwrap();

        let from = root.join("page.tsx");
        let resolved = resolve_import(&from, "./Foo", root);
        assert!(resolved.is_some());
        assert!(resolved.unwrap().ends_with("Foo.tsx"));
    }

    #[test]
    fn resolve_import_ignores_bare_specifiers() {
        let dir = setup_temp_dir();
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
}
