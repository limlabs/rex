//! ESM source transformation for V8 native module loading.
//!
//! Transforms TypeScript/TSX source files to valid ESM JavaScript using OXC,
//! and walks import graphs to collect all source modules needed for V8 loading.

use anyhow::{Context, Result};
use rex_v8::EsmSourceModule;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

/// File extensions that are non-code assets — skip during module collection.
const ASSET_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "ico", "avif", "bmp", "tiff", "svg", "css", "scss",
    "sass", "less", "woff", "woff2", "ttf", "eot", "otf", "mp3", "mp4", "wav", "ogg", "webm",
    "pdf", "json",
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

/// A discovered "use client" boundary with its reference IDs.
#[derive(Debug, Clone)]
pub struct ClientBoundary {
    /// Relative path (from project root) of the client boundary module.
    pub rel_path: String,
    /// Export names from this module.
    pub exports: Vec<String>,
    /// Client reference IDs (one per export), computed via `client_reference_id`.
    pub ref_ids: Vec<String>,
}

/// An extracted inline server action from a source file.
#[derive(Debug, Clone)]
pub struct ExtractedServerAction {
    /// Relative path of the source file (from project root).
    pub rel_path: String,
    /// Generated function name (e.g., `__rex_action_0`).
    pub action_name: String,
    /// Stable action ID (SHA-256 based).
    pub action_id: String,
}

/// Result of collecting source modules from an import graph walk.
pub struct CollectedModules {
    /// Source modules (local files), OXC-transformed to valid ESM JS.
    pub source_modules: Vec<EsmSourceModule>,
    /// Bare specifier imports (node_modules deps) not covered by the dep IIFE.
    pub extra_dep_imports: Vec<DepImport>,
    /// Client boundaries discovered during the walk (app router only).
    pub client_boundaries: Vec<ClientBoundary>,
    /// Inline server actions extracted from source files (app router only).
    pub extracted_actions: Vec<ExtractedServerAction>,
}

/// Transform a source file for HMR: strip TS/JSX and rewrite relative imports to absolute paths.
pub fn transform_and_rewrite_imports(
    source: &str,
    file_path: &Path,
    project_root: &Path,
    known_dep_specifiers: &HashSet<String>,
) -> Result<String> {
    let filename = file_path.to_string_lossy().to_string();
    let (local_imports, _bare_imports) =
        extract_and_resolve_imports(source, file_path, project_root, known_dep_specifiers);
    let mut import_map: HashMap<String, String> = HashMap::new();
    for (specifier, resolved_path) in &local_imports {
        import_map.insert(
            specifier.clone(),
            resolved_path.to_string_lossy().to_string(),
        );
    }
    let transformed = transform_to_esm(source, &filename)?;
    Ok(rewrite_imports_to_absolute(&transformed, &import_map))
}

/// Transform TypeScript/TSX to valid ESM JavaScript (strip types, transform JSX).
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
pub fn collect_source_modules(
    entries: &[PathBuf],
    project_root: &Path,
    known_dep_specifiers: &HashSet<String>,
) -> Result<CollectedModules> {
    collect_source_modules_inner(entries, project_root, known_dep_specifiers, None)
}

/// Like `collect_source_modules` but generates client reference stubs for `"use client"` modules.
pub fn collect_source_modules_with_stubs(
    entries: &[PathBuf],
    project_root: &Path,
    known_dep_specifiers: &HashSet<String>,
    build_id: &str,
) -> Result<CollectedModules> {
    collect_source_modules_inner(entries, project_root, known_dep_specifiers, Some(build_id))
}

fn collect_source_modules_inner(
    entries: &[PathBuf],
    project_root: &Path,
    known_dep_specifiers: &HashSet<String>,
    build_id: Option<&str>,
) -> Result<CollectedModules> {
    let mut source_modules = Vec::new();
    let mut dep_imports: HashMap<String, DepImport> = HashMap::new();
    let mut client_boundaries: Vec<ClientBoundary> = Vec::new();
    let mut extracted_actions: Vec<ExtractedServerAction> = Vec::new();
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
        // Node modules: register as empty stub and don't walk further.
        // Only user source files are OXC-transformed and walked.
        let in_node_modules = path.components().any(|c| c.as_os_str() == "node_modules");
        if in_node_modules {
            let filename = path.to_string_lossy().to_string();
            // Check for "use client" to generate proper client reference stubs
            if let Some(bid) = build_id {
                if let Ok(source) = std::fs::read_to_string(&path) {
                    let source_type = source_type_from_filename(&filename);
                    if crate::rsc_graph::has_use_client_directive(&source, source_type) {
                        let rel_path = path
                            .strip_prefix(project_root)
                            .unwrap_or(&path)
                            .to_string_lossy()
                            .to_string();
                        let exports = extract_export_names(&source, &filename);
                        let ref_ids: Vec<String> = exports
                            .iter()
                            .map(|e| crate::client_manifest::client_reference_id(&rel_path, e, bid))
                            .collect();
                        client_boundaries.push(ClientBoundary {
                            rel_path: rel_path.clone(),
                            exports: exports.clone(),
                            ref_ids,
                        });
                        let stub = crate::rsc_stubs::generate_client_stub(&rel_path, &exports, bid);
                        source_modules.push(EsmSourceModule {
                            specifier: filename,
                            source: stub,
                        });
                        continue;
                    }
                }
            }
            source_modules.push(EsmSourceModule {
                specifier: filename,
                source: "export default {};".to_string(),
            });
            continue;
        }

        // Asset stubs: images export { src: "/path" } for next/image, others export {}.
        if is_asset_file(&path) {
            let source = if is_image_asset(&path) {
                let rel = path.strip_prefix(project_root).unwrap_or(&path);
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                // Files in public/ are served at / (strip the public/ prefix)
                let url = rel_str.strip_prefix("public/").unwrap_or(&rel_str);
                format!("export default {{ src: \"/{}\" }};", url)
            } else {
                "export default {};".to_string()
            };
            source_modules.push(EsmSourceModule {
                specifier: path.to_string_lossy().to_string(),
                source,
            });
            continue;
        }

        let specifier = path.to_string_lossy().to_string();

        // MDX → JSX compilation before OXC transform.
        let (mut source, oxc_filename) = if is_mdx_file(&path) {
            let raw = std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read {}", path.display()))?;
            let options = crate::mdx::mdx_options_for_project(project_root);
            let compiled = rex_mdx::compile_mdx_with_options(&raw, &options)
                .with_context(|| format!("Failed to compile MDX: {}", path.display()))?;
            let source_dir = path.parent().unwrap_or(Path::new("."));
            let compiled = crate::css_modules::absolutize_relative_imports(&compiled, source_dir);
            let jsx_name = path.with_extension("jsx").to_string_lossy().to_string();
            (compiled, jsx_name)
        } else {
            let raw = std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read {}", path.display()))?;
            (raw, specifier.clone())
        };

        if let Some(bid) = build_id {
            let source_type = source_type_from_filename(&oxc_filename);
            if crate::rsc_graph::has_use_client_directive(&source, source_type) {
                let rel_path = path
                    .strip_prefix(project_root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string();
                let exports = extract_export_names(&source, &oxc_filename);
                let ref_ids: Vec<String> = exports
                    .iter()
                    .map(|e| crate::client_manifest::client_reference_id(&rel_path, e, bid))
                    .collect();
                tracing::debug!(
                    rel_path = %rel_path,
                    exports = ?exports,
                    ref_ids = ?ref_ids,
                    "ESM: generating client stub"
                );
                client_boundaries.push(ClientBoundary {
                    rel_path: rel_path.clone(),
                    exports: exports.clone(),
                    ref_ids: ref_ids.clone(),
                });
                let stub = crate::rsc_stubs::generate_client_stub(&rel_path, &exports, bid);
                source_modules.push(EsmSourceModule {
                    specifier,
                    source: stub,
                });
                continue; // Don't follow imports from client boundary modules
            }
        }

        // Extract imports and resolve them
        let (local_imports, bare_imports) =
            extract_and_resolve_imports(&source, &path, project_root, known_dep_specifiers);

        // Build import resolution map (original specifier → absolute path)
        let mut import_map: HashMap<String, String> = HashMap::new();
        for (imp_specifier, resolved_path) in &local_imports {
            import_map.insert(
                imp_specifier.clone(),
                resolved_path.to_string_lossy().to_string(),
            );
        }

        // Track bare specifier imports for extra dep IIFE
        for (imp_specifier, named, has_default) in &bare_imports {
            if known_dep_specifiers.contains(imp_specifier.as_str()) {
                continue;
            }
            let entry = dep_imports
                .entry(imp_specifier.clone())
                .or_insert_with(|| DepImport {
                    specifier: imp_specifier.clone(),
                    named_exports: HashSet::new(),
                    has_default: false,
                });
            entry.named_exports.extend(named.iter().cloned());
            entry.has_default |= has_default;
        }

        // Extract inline server actions before OXC transform, append $$typeof after.
        let action_suffix = extract_server_actions_if_needed(
            &mut source,
            &path,
            project_root,
            build_id,
            &mut extracted_actions,
        );

        let mut transformed = transform_to_esm(&source, &oxc_filename)?;
        if !action_suffix.is_empty() {
            transformed.push_str(&action_suffix);
        }

        // Rewrite import specifiers to absolute paths
        let rewritten = rewrite_imports_to_absolute(&transformed, &import_map);

        source_modules.push(EsmSourceModule {
            specifier,
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

    // Extra deps are bundled by the caller (startup.rs) via rolldown, not stubbed here.

    Ok(CollectedModules {
        source_modules,
        extra_dep_imports: dep_imports.into_values().collect(),
        client_boundaries,
        extracted_actions,
    })
}

/// Extract inline server actions from source if present, returning JS to append.
fn extract_server_actions_if_needed(
    source: &mut String,
    path: &Path,
    project_root: &Path,
    build_id: Option<&str>,
    extracted_actions: &mut Vec<ExtractedServerAction>,
) -> String {
    let bid = match build_id {
        Some(b) if source.contains("use server") => b,
        _ => return String::new(),
    };
    let rel_path = path
        .strip_prefix(project_root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string();
    let result = match crate::server_action_extract::extract_inline_server_actions(source, path) {
        Some(r) => r,
        None => return String::new(),
    };
    *source = result.source;
    let mut suffix = String::new();
    for action in &result.actions {
        let id = crate::server_action_manifest::server_action_id(&rel_path, &action.name, bid);
        suffix.push_str(&format!(
            "\n{n}.$$typeof = Symbol.for(\"react.server.reference\");\n{n}.$$id = \"{id}\";\n{n}.$$bound = null;\n",
            n = action.name,
        ));
        extracted_actions.push(ExtractedServerAction {
            rel_path: rel_path.clone(),
            action_name: action.name.clone(),
            action_id: id,
        });
    }
    tracing::debug!(path = %rel_path, count = result.actions.len(), "ESM: extracted inline server actions");
    suffix
}

/// Specifiers that are pre-bundled as dep ESM modules.
/// The import graph walker skips these (they're not user source files).
pub fn dep_specifiers(has_app: bool) -> HashSet<String> {
    let mut set: HashSet<String> = [
        "react",
        "react/jsx-runtime",
        "react/jsx-dev-runtime",
        "react-dom/server",
    ]
    .into_iter()
    .map(String::from)
    .collect();

    if has_app {
        set.insert("react-server-dom-webpack/server".to_string());
        set.insert("react-server-dom-webpack/client".to_string());
    }

    // Node.js built-in polyfills and framework stubs are registered as dep modules.
    // The import graph walker should skip these — they're already handled.
    if let Ok(runtime_dir) = crate::build_utils::runtime_server_dir() {
        for (specifier, _) in crate::build_utils::node_polyfill_aliases(&runtime_dir) {
            set.insert(specifier);
        }
    }

    set
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

        // Skip specifiers handled by pre-registered modules (rex/*, next/*, react, etc.)
        if known_specifiers.contains(specifier)
            || specifier.starts_with("rex/")
            || specifier.starts_with("next/")
        {
            continue;
        }

        if specifier.starts_with('.') || specifier.starts_with('/') {
            // Relative/absolute import — resolve to local file
            if let Some(resolved) =
                crate::rsc_graph::resolve_import(file_path, specifier, project_root)
            {
                local_imports.push((specifier.to_string(), resolved));
            }
        } else if let Some(resolved) = resolve_tsconfig_alias(specifier, project_root) {
            // Bare specifier matching a tsconfig path alias (e.g. @/components/*)
            local_imports.push((specifier.to_string(), resolved));
        } else {
            // True bare npm specifier — register as dep stub
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

fn is_image_asset(path: &Path) -> bool {
    const EXTS: &[&str] = &[
        "png", "jpg", "jpeg", "gif", "webp", "ico", "avif", "bmp", "tiff", "svg",
    ];
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| EXTS.contains(&ext.to_lowercase().as_str()))
}

fn is_mdx_file(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some("mdx")
}

/// Resolve a bare specifier through tsconfig path aliases only.
/// Returns None if the specifier doesn't match any alias.
fn resolve_tsconfig_alias(specifier: &str, project_root: &Path) -> Option<PathBuf> {
    let mut aliases: Vec<_> = crate::build_utils::tsconfig_path_aliases(project_root)
        .into_iter()
        .collect();
    aliases.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    for (prefix, targets) in &aliases {
        if specifier.starts_with(prefix.as_str()) {
            if let Some(Some(target)) = targets.first() {
                let rest = &specifier[prefix.len()..];
                let candidate = PathBuf::from(format!("{target}{rest}"));
                if let Some(resolved) = crate::rsc_graph::try_resolve_path(&candidate) {
                    return Some(resolved);
                }
            }
        }
    }
    None
}

/// Extract export names from a source file (used for client reference stubs).
fn extract_export_names(source: &str, filename: &str) -> Vec<String> {
    let allocator = oxc_allocator::Allocator::default();
    let source_type = source_type_from_filename(filename);
    let parsed = oxc_parser::Parser::new(&allocator, source, source_type).parse();
    let mut exports = Vec::new();

    for stmt in &parsed.program.body {
        match stmt {
            oxc_ast::ast::Statement::ExportDefaultDeclaration(_) => {
                exports.push("default".to_string());
            }
            oxc_ast::ast::Statement::ExportNamedDeclaration(export) => {
                if let Some(decl) = &export.declaration {
                    match decl {
                        oxc_ast::ast::Declaration::FunctionDeclaration(f) => {
                            if let Some(id) = &f.id {
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
                        oxc_ast::ast::Declaration::ClassDeclaration(c) => {
                            if let Some(id) = &c.id {
                                exports.push(id.name.to_string());
                            }
                        }
                        _ => {}
                    }
                }
                for specifier in &export.specifiers {
                    exports.push(specifier.exported.name().to_string());
                }
            }
            _ => {}
        }
    }

    // Default to "default" if no exports found (common for components)
    if exports.is_empty() {
        exports.push("default".to_string());
    }

    exports
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
    fn dep_specifiers_pages_only() {
        let specs = dep_specifiers(false);
        assert!(specs.contains("react"));
        assert!(specs.contains("react/jsx-runtime"));
        assert!(!specs.contains("react-server-dom-webpack/server"));
    }

    #[test]
    fn dep_specifiers_with_app() {
        let specs = dep_specifiers(true);
        assert!(specs.contains("react"));
        assert!(specs.contains("react-server-dom-webpack/server"));
    }
}
