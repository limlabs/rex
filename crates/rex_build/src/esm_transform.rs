//! ESM source transformation for V8 native module loading.
//!
//! Transforms TypeScript/TSX source files to valid ESM JavaScript using OXC,
//! and walks import graphs to collect all source modules needed for V8 loading.

use anyhow::{Context, Result};
use rex_v8::{EsmSourceModule, SyntheticModuleDef};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

/// File extensions that are non-code assets — skip during module collection.
const ASSET_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "ico", "avif", "bmp", "tiff", "svg", "css", "scss",
    "sass", "less", "woff", "woff2", "ttf", "eot", "otf", "mp3", "mp4", "wav", "ogg", "webm",
    "pdf", "json", "mdx",
];

/// Information about a bare specifier import (node_modules dependency).
#[derive(Debug, Clone)]
pub struct DepImport {
    /// The bare specifier (e.g., "dayjs", "lodash/merge").
    pub specifier: String,
    /// Named exports used from this dep across all source files.
    pub named_exports: HashSet<String>,
    /// Whether any import uses a default import.
    pub has_default: bool,
}

/// Result of collecting source modules from an import graph walk.
pub struct CollectedModules {
    /// Source modules (local files), OXC-transformed to valid ESM JS.
    pub source_modules: Vec<EsmSourceModule>,
    /// Bare specifier imports (node_modules deps) discovered during the walk.
    /// Does NOT include deps already covered by the dep IIFE (React, etc.).
    pub extra_dep_imports: Vec<DepImport>,
}

/// Transform a TypeScript/TSX source file to valid ESM JavaScript.
///
/// - Strips TypeScript types (interfaces, type annotations, enums, etc.)
/// - Transforms JSX to createElement/jsx-runtime calls
/// - Preserves import/export statements as valid ESM
pub fn transform_to_esm(source: &str, filename: &str) -> Result<String> {
    let allocator = oxc_allocator::Allocator::default();
    let source_type = source_type_from_filename(filename);
    let mut ret = oxc_parser::Parser::new(&allocator, source, source_type).parse();

    if ret.panicked {
        anyhow::bail!("Parse error in {filename}");
    }

    let semantic = oxc_semantic::SemanticBuilder::new()
        .build(&ret.program)
        .semantic;

    let options = oxc_transformer::TransformOptions::default();
    let transformer = oxc_transformer::Transformer::new(&allocator, Path::new(filename), &options);
    transformer.build_with_scoping(semantic.into_scoping(), &mut ret.program);

    Ok(oxc_codegen::Codegen::new().build(&ret.program).code)
}

/// Collect all source modules by walking the import graph from entry files.
///
/// - Resolves relative imports to absolute paths
/// - Transforms each discovered source file with OXC
/// - Rewrites import specifiers in generated code to use absolute paths
/// - Collects bare specifier imports for dep IIFE generation
/// - Skips asset imports (CSS, images, etc.)
///
/// `known_dep_specifiers` lists deps already handled by the dep IIFE (e.g., "react").
/// These are not included in the returned `extra_dep_imports`.
pub fn collect_source_modules(
    entries: &[PathBuf],
    project_root: &Path,
    known_dep_specifiers: &HashSet<String>,
) -> Result<CollectedModules> {
    let mut source_modules = Vec::new();
    let mut dep_imports: HashMap<String, DepImport> = HashMap::new();
    let mut visited: HashSet<PathBuf> = HashSet::new();
    let mut queue: VecDeque<PathBuf> = VecDeque::new();

    for entry in entries {
        let canonical = entry
            .canonicalize()
            .with_context(|| format!("Failed to canonicalize {}", entry.display()))?;
        if visited.insert(canonical.clone()) {
            queue.push_back(canonical);
        }
    }

    while let Some(path) = queue.pop_front() {
        // Asset files (CSS, images, etc.) get registered as empty modules
        // so V8 can resolve `import '../styles/globals.css'` without errors.
        if is_asset_file(&path) {
            source_modules.push(EsmSourceModule {
                specifier: path.to_string_lossy().to_string(),
                source: "export default {};".to_string(),
            });
            continue;
        }

        let source = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;

        let filename = path.to_string_lossy().to_string();

        // Extract imports and resolve them
        let (local_imports, bare_imports) =
            extract_and_resolve_imports(&source, &path, project_root, known_dep_specifiers);

        // Build import resolution map (original specifier → absolute path)
        let mut import_map: HashMap<String, String> = HashMap::new();
        for (specifier, resolved_path) in &local_imports {
            import_map.insert(
                specifier.clone(),
                resolved_path.to_string_lossy().to_string(),
            );
        }

        // Track bare specifier imports for extra dep IIFE
        for (specifier, named, has_default) in &bare_imports {
            if known_dep_specifiers.contains(specifier.as_str()) {
                continue;
            }
            let entry = dep_imports
                .entry(specifier.clone())
                .or_insert_with(|| DepImport {
                    specifier: specifier.clone(),
                    named_exports: HashSet::new(),
                    has_default: false,
                });
            entry.named_exports.extend(named.iter().cloned());
            entry.has_default |= has_default;
        }

        // Transform source (strip TS + JSX)
        let transformed = transform_to_esm(&source, &filename)?;

        // Rewrite import specifiers to absolute paths
        let rewritten = rewrite_imports_to_absolute(&transformed, &import_map);

        source_modules.push(EsmSourceModule {
            specifier: filename,
            source: rewritten,
        });

        // Queue discovered local imports
        for (_specifier, import_path) in local_imports {
            if !visited.contains(&import_path) {
                visited.insert(import_path.clone());
                queue.push_back(import_path);
            }
        }
    }

    Ok(CollectedModules {
        source_modules,
        extra_dep_imports: dep_imports.into_values().collect(),
    })
}

/// Generate synthetic module definitions for the pages dep IIFE.
///
/// These wrap `globalThis.__rex_deps` properties as ESM modules.
pub fn pages_synthetic_modules() -> Vec<SyntheticModuleDef> {
    vec![
        SyntheticModuleDef {
            specifier: "react".to_string(),
            export_names: react_export_names(),
            globals_expr: "globalThis.__rex_deps.react".to_string(),
        },
        SyntheticModuleDef {
            specifier: "react/jsx-runtime".to_string(),
            export_names: vec!["jsx".into(), "jsxs".into(), "Fragment".into()],
            globals_expr: "globalThis.__rex_deps[\"react/jsx-runtime\"]".to_string(),
        },
        SyntheticModuleDef {
            specifier: "react/jsx-dev-runtime".to_string(),
            export_names: vec!["jsxDEV".into(), "Fragment".into()],
            globals_expr: "globalThis.__rex_deps[\"react/jsx-dev-runtime\"]".to_string(),
        },
        SyntheticModuleDef {
            specifier: "react-dom/server".to_string(),
            export_names: vec!["renderToString".into(), "default".into()],
            globals_expr: "globalThis.__rex_deps[\"react-dom/server\"]".to_string(),
        },
    ]
}

/// Generate synthetic module definitions for the RSC flight dep IIFE.
pub fn flight_synthetic_modules() -> Vec<SyntheticModuleDef> {
    vec![
        SyntheticModuleDef {
            specifier: "react".to_string(),
            export_names: react_export_names(),
            globals_expr: "globalThis.__rex_flight_deps.react".to_string(),
        },
        SyntheticModuleDef {
            specifier: "react/jsx-runtime".to_string(),
            export_names: vec!["jsx".into(), "jsxs".into(), "Fragment".into()],
            globals_expr: "globalThis.__rex_flight_deps[\"react/jsx-runtime\"]".to_string(),
        },
        SyntheticModuleDef {
            specifier: "react/jsx-dev-runtime".to_string(),
            export_names: vec!["jsxDEV".into(), "Fragment".into()],
            globals_expr: "globalThis.__rex_flight_deps[\"react/jsx-dev-runtime\"]".to_string(),
        },
        SyntheticModuleDef {
            specifier: "react-server-dom-webpack/server".to_string(),
            export_names: vec![
                "renderToReadableStream".into(),
                "registerServerReference".into(),
                "decodeReply".into(),
                "decodeAction".into(),
                "default".into(),
            ],
            globals_expr: "globalThis.__rex_flight_deps[\"react-server-dom-webpack/server\"]"
                .to_string(),
        },
        SyntheticModuleDef {
            specifier: "react-dom/server".to_string(),
            export_names: vec!["renderToString".into(), "default".into()],
            globals_expr: "globalThis.__rex_flight_deps[\"react-dom/server\"]".to_string(),
        },
        SyntheticModuleDef {
            specifier: "react-server-dom-webpack/client".to_string(),
            export_names: vec!["createFromReadableStream".into(), "default".into()],
            globals_expr: "globalThis.__rex_flight_deps[\"react-server-dom-webpack/client\"]"
                .to_string(),
        },
    ]
}

/// Specifiers handled by the pages dep IIFE.
pub fn pages_known_specifiers() -> HashSet<String> {
    [
        "react",
        "react/jsx-runtime",
        "react/jsx-dev-runtime",
        "react-dom/server",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

/// Specifiers handled by the RSC flight dep IIFE.
pub fn flight_known_specifiers() -> HashSet<String> {
    [
        "react",
        "react/jsx-runtime",
        "react/jsx-dev-runtime",
        "react-server-dom-webpack/server",
        "react-dom/server",
        "react-server-dom-webpack/client",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

/// Build a `DepModuleConfig` from an IIFE JS string and synthetic module defs.
pub fn build_dep_config(
    iife_js: String,
    synthetic_modules: Vec<SyntheticModuleDef>,
) -> rex_v8::DepModuleConfig {
    rex_v8::DepModuleConfig {
        iife_js,
        synthetic_modules,
    }
}

// --- Internal helpers ---

/// A resolved local import: (original_specifier, resolved_absolute_path).
type LocalImport = (String, PathBuf);
/// A bare specifier import: (specifier, named_exports, has_default).
type BareImport = (String, Vec<String>, bool);

/// Extract imports from a source file, resolving local ones to absolute paths.
#[allow(clippy::type_complexity)]
fn extract_and_resolve_imports(
    source: &str,
    file_path: &Path,
    project_root: &Path,
    known_specifiers: &HashSet<String>,
) -> (Vec<LocalImport>, Vec<BareImport>) {
    let allocator = oxc_allocator::Allocator::default();
    let source_type = source_type_from_filename(&file_path.to_string_lossy());
    let parsed = oxc_parser::Parser::new(&allocator, source, source_type).parse();

    let mut local_imports = Vec::new();
    let mut bare_imports: Vec<(String, Vec<String>, bool)> = Vec::new();

    for stmt in &parsed.program.body {
        let (specifier, named, has_default) = match stmt {
            oxc_ast::ast::Statement::ImportDeclaration(import) => {
                let spec = import.source.value.as_str();
                let mut named = Vec::new();
                let mut has_default = false;
                if let Some(specifiers) = &import.specifiers {
                    for s in specifiers {
                        match s {
                            oxc_ast::ast::ImportDeclarationSpecifier::ImportSpecifier(s) => {
                                named.push(s.imported.name().to_string());
                            }
                            oxc_ast::ast::ImportDeclarationSpecifier::ImportDefaultSpecifier(_) => {
                                has_default = true;
                            }
                            oxc_ast::ast::ImportDeclarationSpecifier::ImportNamespaceSpecifier(
                                _,
                            ) => {
                                has_default = true;
                            }
                        }
                    }
                }
                (spec, named, has_default)
            }
            oxc_ast::ast::Statement::ExportNamedDeclaration(export) => {
                if let Some(source) = &export.source {
                    (source.value.as_str(), Vec::new(), false)
                } else {
                    continue;
                }
            }
            oxc_ast::ast::Statement::ExportAllDeclaration(export) => {
                (export.source.value.as_str(), Vec::new(), true)
            }
            _ => continue,
        };

        // Skip specifiers handled by synthetic modules (rex/*, react, etc.)
        if known_specifiers.contains(specifier) || specifier.starts_with("rex/") {
            continue;
        }

        // Try to resolve as a local import
        if let Some(resolved) = crate::rsc_graph::resolve_import(file_path, specifier, project_root)
        {
            local_imports.push((specifier.to_string(), resolved));
        } else if !specifier.starts_with('.') && !specifier.starts_with('/') {
            // Bare specifier — node_modules dependency
            bare_imports.push((specifier.to_string(), named, has_default));
        }
    }

    (local_imports, bare_imports)
}

/// Rewrite import specifiers in generated JS to use absolute paths.
///
/// Parses the generated code, finds import source spans, and does
/// position-based replacement. This is precise (no regex) and handles
/// all import/export-from forms.
fn rewrite_imports_to_absolute(js: &str, import_map: &HashMap<String, String>) -> String {
    if import_map.is_empty() {
        return js.to_string();
    }

    let allocator = oxc_allocator::Allocator::default();
    let parsed = oxc_parser::Parser::new(&allocator, js, oxc_span::SourceType::mjs()).parse();

    let mut replacements: Vec<(u32, u32, String)> = Vec::new();

    for stmt in &parsed.program.body {
        let source_literal = match stmt {
            oxc_ast::ast::Statement::ImportDeclaration(import) => Some(&import.source),
            oxc_ast::ast::Statement::ExportNamedDeclaration(export) => export.source.as_ref(),
            oxc_ast::ast::Statement::ExportAllDeclaration(export) => Some(&export.source),
            _ => None,
        };

        if let Some(lit) = source_literal {
            let spec = lit.value.as_str();
            if let Some(resolved) = import_map.get(spec) {
                // Replace the string content (inside quotes): span includes quotes
                let start = lit.span.start + 1; // skip opening quote
                let end = lit.span.end - 1; // skip closing quote
                replacements.push((start, end, resolved.clone()));
            }
        }
    }

    if replacements.is_empty() {
        return js.to_string();
    }

    // Apply replacements in reverse order to preserve positions
    let mut result = js.to_string();
    replacements.sort_by(|a, b| b.0.cmp(&a.0));
    for (start, end, replacement) in replacements {
        result.replace_range(start as usize..end as usize, &replacement);
    }
    result
}

fn source_type_from_filename(filename: &str) -> oxc_span::SourceType {
    if filename.ends_with(".tsx") {
        oxc_span::SourceType::tsx()
    } else if filename.ends_with(".ts") {
        oxc_span::SourceType::ts()
    } else if filename.ends_with(".jsx") {
        oxc_span::SourceType::jsx()
    } else {
        oxc_span::SourceType::mjs()
    }
}

fn is_asset_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| ASSET_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
}

fn react_export_names() -> Vec<String> {
    [
        "createElement",
        "useState",
        "useEffect",
        "useContext",
        "useReducer",
        "useCallback",
        "useMemo",
        "useRef",
        "useLayoutEffect",
        "useImperativeHandle",
        "useDebugValue",
        "useDeferredValue",
        "useTransition",
        "useId",
        "useSyncExternalStore",
        "useInsertionEffect",
        "useActionState",
        "useOptimistic",
        "use",
        "memo",
        "forwardRef",
        "lazy",
        "Suspense",
        "Fragment",
        "Children",
        "Component",
        "PureComponent",
        "createContext",
        "createRef",
        "cloneElement",
        "isValidElement",
        "startTransition",
        "cache",
        "default",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn transform_strips_typescript() {
        let source = "const x: number = 42;\nexport default x;\n";
        let result = transform_to_esm(source, "test.ts").unwrap();
        assert!(!result.contains(": number"));
        assert!(result.contains("42"));
    }

    #[test]
    fn transform_handles_jsx() {
        let source = r#"import React from 'react';
export default function App() { return <div>Hello</div>; }
"#;
        let result = transform_to_esm(source, "test.tsx").unwrap();
        assert!(!result.contains("<div>"));
    }

    #[test]
    fn rewrite_imports_replaces_relative() {
        let js = r#"import { Button } from "./Button";
import utils from "../utils";
const x = 1;
"#;
        let mut map = HashMap::new();
        map.insert("./Button".to_string(), "/abs/Button.tsx".to_string());
        map.insert("../utils".to_string(), "/abs/utils.ts".to_string());

        let result = rewrite_imports_to_absolute(js, &map);
        assert!(result.contains("/abs/Button.tsx"));
        assert!(result.contains("/abs/utils.ts"));
        assert!(!result.contains("./Button"));
    }

    #[test]
    fn rewrite_imports_preserves_bare_specifiers() {
        let js = r#"import React from "react";
import { format } from "./utils";
"#;
        let mut map = HashMap::new();
        map.insert("./utils".to_string(), "/abs/utils.ts".to_string());

        let result = rewrite_imports_to_absolute(js, &map);
        assert!(result.contains("\"react\""));
        assert!(result.contains("/abs/utils.ts"));
    }

    #[test]
    fn pages_synthetic_modules_has_react() {
        let mods = pages_synthetic_modules();
        assert!(mods.iter().any(|m| m.specifier == "react"));
        assert!(mods.iter().any(|m| m.specifier == "react/jsx-runtime"));
    }

    #[test]
    fn flight_synthetic_modules_has_rsdw() {
        let mods = flight_synthetic_modules();
        assert!(mods
            .iter()
            .any(|m| m.specifier == "react-server-dom-webpack/server"));
    }
}
