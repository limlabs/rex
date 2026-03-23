//! OXC-based parsing helpers for ESM source transformation.
//!
//! Extracted from `esm_transform` to keep that module under the 700-line limit.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// A resolved local import: (original_specifier, resolved_absolute_path).
pub(crate) type LocalImport = (String, PathBuf);
/// A bare specifier import: (specifier, named_exports, has_default).
pub(crate) type BareImport = (String, Vec<String>, bool);

pub(crate) fn source_type_from_filename(filename: &str) -> oxc_span::SourceType {
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

/// Extract imports from a source file, resolving local ones to absolute paths.
#[allow(clippy::type_complexity)]
pub(crate) fn extract_and_resolve_imports(
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

/// Resolve a bare specifier through tsconfig path aliases only.
/// Returns None if the specifier doesn't match any alias.
pub(crate) fn resolve_tsconfig_alias(specifier: &str, project_root: &Path) -> Option<PathBuf> {
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
pub(crate) fn extract_export_names(source: &str, filename: &str) -> Vec<String> {
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

/// Check if source has a module-level "use server" directive (before any code).
pub(crate) fn has_module_level_use_server(source: &str) -> bool {
    let trimmed = source.trim_start();
    for line in trimmed.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with("//") || l.starts_with("/*") {
            continue;
        }
        if l == "\"use server\""
            || l == "\"use server\";"
            || l == "'use server'"
            || l == "'use server';"
        {
            return true;
        }
        break; // First non-comment, non-empty line isn't "use server"
    }
    false
}
