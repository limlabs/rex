use anyhow::Result;
use oxc_span::GetSpan;
use rex_router::ScanResult;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::debug;

use crate::font_google::{self, ProcessedFont};

/// Font sources we recognize in import declarations.
const FONT_IMPORT_SOURCES: &[&str] = &["next/font/google", "rex/font/google", "@next/font/google"];

/// Configuration extracted from a font function call.
#[derive(Debug, Clone)]
pub(crate) struct FontConfig {
    pub family: String,
    pub weights: Vec<String>,
    pub display: String,
    pub variable: Option<String>,
    pub fallback: Vec<String>,
    #[allow(dead_code)] // parsed for future subset-specific font optimization
    pub subsets: Vec<String>,
}

/// Result of font pre-processing.
pub(crate) struct FontProcessing {
    /// Map of original page abs_path → modified page path (with font calls replaced)
    pub page_overrides: HashMap<PathBuf, PathBuf>,
    /// Combined @font-face + utility class CSS content
    pub font_css: String,
    /// Font filenames to preload (served from /_rex/static/)
    pub font_preloads: Vec<String>,
}

/// Info about a single font import declaration.
struct FontImportInfo {
    import_span: (u32, u32),
    calls: Vec<FontCallInfo>,
}

/// Info about a single font function call (e.g., `Inter({ ... })`).
struct FontCallInfo {
    call_span: (u32, u32),
    config: FontConfig,
}

/// Pre-process font imports in all pages and _app.
///
/// Scans source files for `next/font/google` / `rex/font/google` imports,
/// downloads font files from Google Fonts at build time, generates
/// `@font-face` CSS, and rewrites source to replace font function calls
/// with static objects containing `className`, `style`, and `variable`.
pub(crate) async fn process_fonts(
    scan: &ScanResult,
    output_dir: &Path,
    build_id: &str,
    project_root: &Path,
    existing_overrides: &HashMap<PathBuf, PathBuf>,
) -> Result<FontProcessing> {
    let temp_dir = output_dir.join("_fonts");
    fs::create_dir_all(&temp_dir)?;

    let cache_dir = project_root.join(".rex").join("font-cache");
    fs::create_dir_all(&cache_dir)?;

    let mut page_overrides: HashMap<PathBuf, PathBuf> = HashMap::new();
    let mut all_font_css = String::new();
    let mut all_preloads: Vec<String> = Vec::new();
    let mut processed_fonts: HashMap<String, ProcessedFont> = HashMap::new();

    // Collect source files: (original_abs_path, effective_path, label)
    let mut sources: Vec<(&PathBuf, PathBuf, &str)> = Vec::new();
    for route in &scan.routes {
        let effective = existing_overrides
            .get(&route.abs_path)
            .cloned()
            .unwrap_or_else(|| route.abs_path.clone());
        sources.push((&route.abs_path, effective, &route.pattern));
    }
    if let Some(app) = &scan.app {
        let effective = existing_overrides
            .get(&app.abs_path)
            .cloned()
            .unwrap_or_else(|| app.abs_path.clone());
        sources.push((&app.abs_path, effective, "_app"));
    }

    for (original_path, effective_path, label) in &sources {
        let source = fs::read_to_string(effective_path)?;
        let font_imports = find_font_imports(&source, effective_path)?;
        if font_imports.is_empty() {
            continue;
        }

        debug!(page = %label, fonts = font_imports.len(), "Processing font imports");

        let mut replacements: Vec<(u32, u32, String)> = Vec::new();

        for import_info in &font_imports {
            replacements.push((
                import_info.import_span.0,
                import_info.import_span.1,
                String::new(),
            ));

            for call in &import_info.calls {
                let font = font_google::process_single_font(
                    &call.config.family,
                    &call.config.weights,
                    &call.config.display,
                    call.config.variable.as_deref(),
                    &call.config.fallback,
                    output_dir,
                    build_id,
                    &cache_dir,
                    &mut processed_fonts,
                )
                .await?;

                let replacement = font_google::build_font_object(
                    &call.config.family,
                    &call.config.fallback,
                    call.config.variable.as_deref(),
                    &font.scoped_family,
                );
                replacements.push((call.call_span.0, call.call_span.1, replacement));
            }
        }

        if replacements.is_empty() {
            continue;
        }

        // Apply replacements from back to front
        let mut modified = source.clone();
        replacements.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
        for (start, end, replacement) in &replacements {
            modified.replace_range(*start as usize..*end as usize, replacement);
        }

        let source_dir = effective_path.parent().unwrap_or(Path::new("."));
        modified = crate::css_modules::absolutize_relative_imports(&modified, source_dir);

        let hash = font_google::short_hash(effective_path.to_string_lossy().as_bytes());
        let filename = effective_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let modified_path = temp_dir.join(format!("{hash}_{filename}"));
        fs::write(&modified_path, &modified)?;

        page_overrides.insert((*original_path).clone(), modified_path);
    }

    for font in processed_fonts.values() {
        all_font_css.push_str(&font.css);
        all_preloads.extend(font.preload_files.iter().cloned());
    }

    Ok(FontProcessing {
        page_overrides,
        font_css: all_font_css,
        font_preloads: all_preloads,
    })
}

/// Find font imports and their corresponding function calls in source.
fn find_font_imports(source: &str, source_path: &Path) -> Result<Vec<FontImportInfo>> {
    let source_type = crate::css_collect::source_type_for_path(source_path);
    let allocator = oxc_allocator::Allocator::default();
    let parsed = oxc_parser::Parser::new(&allocator, source, source_type).parse();

    let mut font_names: HashMap<String, String> = HashMap::new();
    let mut import_spans: Vec<(u32, u32, Vec<String>)> = Vec::new();

    for stmt in &parsed.program.body {
        if let oxc_ast::ast::Statement::ImportDeclaration(import) = stmt {
            let src = import.source.value.as_str();
            if !FONT_IMPORT_SOURCES.contains(&src) {
                continue;
            }

            let mut imported_names = Vec::new();
            if let Some(specifiers) = &import.specifiers {
                for spec in specifiers {
                    if let oxc_ast::ast::ImportDeclarationSpecifier::ImportSpecifier(named) = spec {
                        let local = named.local.name.as_str().to_string();
                        let imported = match &named.imported {
                            oxc_ast::ast::ModuleExportName::IdentifierName(id) => {
                                id.name.as_str().to_string()
                            }
                            oxc_ast::ast::ModuleExportName::IdentifierReference(id) => {
                                id.name.as_str().to_string()
                            }
                            oxc_ast::ast::ModuleExportName::StringLiteral(s) => {
                                s.value.as_str().to_string()
                            }
                        };
                        font_names.insert(local.clone(), imported);
                        imported_names.push(local);
                    }
                }
            }

            let span = import.span;
            import_spans.push((span.start, span.end, imported_names));
        }
    }

    if font_names.is_empty() {
        return Ok(Vec::new());
    }

    let mut calls_by_import: HashMap<usize, Vec<FontCallInfo>> = HashMap::new();

    for stmt in &parsed.program.body {
        find_font_calls_in_statement(
            stmt,
            &font_names,
            source,
            &import_spans,
            &mut calls_by_import,
        );
    }

    let mut results = Vec::new();
    for (idx, (start, end, _names)) in import_spans.iter().enumerate() {
        let calls = calls_by_import.remove(&idx).unwrap_or_default();
        results.push(FontImportInfo {
            import_span: (*start, *end),
            calls,
        });
    }

    Ok(results)
}

/// Search a statement for font function calls.
fn find_font_calls_in_statement(
    stmt: &oxc_ast::ast::Statement,
    font_names: &HashMap<String, String>,
    source: &str,
    import_spans: &[(u32, u32, Vec<String>)],
    calls: &mut HashMap<usize, Vec<FontCallInfo>>,
) {
    use oxc_ast::ast::*;

    if let Statement::VariableDeclaration(var_decl) = stmt {
        for decl in &var_decl.declarations {
            if let Some(Expression::CallExpression(call_expr)) = &decl.init {
                check_font_call(call_expr, font_names, source, import_spans, calls);
            }
        }
    }

    if let Statement::ExpressionStatement(expr_stmt) = stmt {
        if let Expression::CallExpression(call_expr) = &expr_stmt.expression {
            check_font_call(call_expr, font_names, source, import_spans, calls);
        }
    }
}

/// Check if a call expression is a font function call and extract its config.
fn check_font_call(
    call_expr: &oxc_ast::ast::CallExpression,
    font_names: &HashMap<String, String>,
    source: &str,
    import_spans: &[(u32, u32, Vec<String>)],
    calls: &mut HashMap<usize, Vec<FontCallInfo>>,
) {
    use oxc_ast::ast::*;

    let callee_name = match &call_expr.callee {
        Expression::Identifier(id) => id.name.as_str(),
        _ => return,
    };

    let font_family = match font_names.get(callee_name) {
        Some(f) => f.clone(),
        None => return,
    };

    let import_idx = match import_spans
        .iter()
        .position(|(_, _, names)| names.contains(&callee_name.to_string()))
    {
        Some(i) => i,
        None => return,
    };

    let config = if let Some(arg) = call_expr.arguments.first() {
        match arg {
            Argument::ObjectExpression(obj) => extract_font_config(&font_family, obj, source),
            _ => default_font_config(font_family),
        }
    } else {
        default_font_config(font_family)
    };

    calls.entry(import_idx).or_default().push(FontCallInfo {
        call_span: (call_expr.span.start, call_expr.span.end),
        config,
    });
}

fn default_font_config(family: String) -> FontConfig {
    FontConfig {
        family,
        weights: vec!["400".to_string()],
        display: "swap".to_string(),
        variable: None,
        fallback: Vec::new(),
        subsets: vec!["latin".to_string()],
    }
}

/// Extract font configuration from an ObjectExpression AST node.
fn extract_font_config(
    family: &str,
    obj: &oxc_ast::ast::ObjectExpression,
    source: &str,
) -> FontConfig {
    let mut weights = Vec::new();
    let mut display = "swap".to_string();
    let mut variable = None;
    let mut fallback = Vec::new();
    let mut subsets = vec!["latin".to_string()];

    for prop in &obj.properties {
        if let oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) = prop {
            let key = match &p.key {
                oxc_ast::ast::PropertyKey::StaticIdentifier(id) => id.name.as_str(),
                _ => continue,
            };

            match key {
                "weight" => weights = extract_string_or_array(&p.value, source),
                "display" => {
                    if let Some(val) = extract_single_string(&p.value) {
                        display = val;
                    }
                }
                "variable" => variable = extract_single_string(&p.value),
                "subsets" => subsets = extract_string_or_array(&p.value, source),
                "fallback" => fallback = extract_string_or_array(&p.value, source),
                _ => {}
            }
        }
    }

    if weights.is_empty() {
        weights.push("400".to_string());
    }

    FontConfig {
        family: family.to_string(),
        weights,
        display,
        variable,
        fallback,
        subsets,
    }
}

/// Extract a string or array of strings from an expression.
fn extract_string_or_array(expr: &oxc_ast::ast::Expression, source: &str) -> Vec<String> {
    use oxc_ast::ast::*;
    match expr {
        Expression::StringLiteral(s) => vec![s.value.as_str().to_string()],
        Expression::NumericLiteral(n) => vec![format!("{}", n.value as u32)],
        Expression::ArrayExpression(arr) => arr
            .elements
            .iter()
            .map(|el| {
                if let ArrayExpressionElement::StringLiteral(s) = el {
                    s.value.as_str().to_string()
                } else if let ArrayExpressionElement::NumericLiteral(n) = el {
                    format!("{}", n.value as u32)
                } else {
                    let span = el.span();
                    let text = &source[span.start as usize..span.end as usize];
                    text.trim_matches(|c| c == '\'' || c == '"').to_string()
                }
            })
            .collect(),
        _ => {
            let span = expr.span();
            let text = &source[span.start as usize..span.end as usize];
            vec![text.trim_matches(|c| c == '\'' || c == '"').to_string()]
        }
    }
}

/// Extract a single string value from an expression.
fn extract_single_string(expr: &oxc_ast::ast::Expression) -> Option<String> {
    if let oxc_ast::ast::Expression::StringLiteral(s) = expr {
        Some(s.value.as_str().to_string())
    } else {
        None
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_find_font_imports_basic() {
        let source = r#"import { Inter } from 'next/font/google'
const inter = Inter({ subsets: ['latin'], weight: '400', display: 'swap' })
export default function Home() { return null }
"#;
        let tmp = tempfile::NamedTempFile::with_suffix(".tsx").unwrap();
        std::fs::write(tmp.path(), source).unwrap();

        let imports = find_font_imports(source, tmp.path()).unwrap();
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].calls.len(), 1);
        assert_eq!(imports[0].calls[0].config.family, "Inter");
        assert_eq!(imports[0].calls[0].config.weights, vec!["400"]);
        assert_eq!(imports[0].calls[0].config.display, "swap");
    }

    #[test]
    fn test_find_font_imports_multiple_fonts() {
        let source = r#"import { Inter, Roboto } from 'rex/font/google'
const inter = Inter({ weight: '400' })
const roboto = Roboto({ weight: ['400', '700'], variable: '--font-roboto' })
"#;
        let tmp = tempfile::NamedTempFile::with_suffix(".tsx").unwrap();
        std::fs::write(tmp.path(), source).unwrap();

        let imports = find_font_imports(source, tmp.path()).unwrap();
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].calls.len(), 2);
        assert_eq!(imports[0].calls[0].config.family, "Inter");
        assert_eq!(imports[0].calls[1].config.family, "Roboto");
        assert_eq!(imports[0].calls[1].config.weights, vec!["400", "700"]);
        assert_eq!(
            imports[0].calls[1].config.variable,
            Some("--font-roboto".to_string())
        );
    }

    #[test]
    fn test_find_font_imports_no_fonts() {
        let source = "import React from 'react'\nexport default function Home() { return null }\n";
        let tmp = tempfile::NamedTempFile::with_suffix(".tsx").unwrap();
        std::fs::write(tmp.path(), source).unwrap();

        let imports = find_font_imports(source, tmp.path()).unwrap();
        assert!(imports.is_empty());
    }

    #[test]
    fn test_find_font_imports_at_next_font() {
        let source = r#"import { Inter } from '@next/font/google'
const inter = Inter({ weight: '400' })
"#;
        let tmp = tempfile::NamedTempFile::with_suffix(".tsx").unwrap();
        std::fs::write(tmp.path(), source).unwrap();

        let imports = find_font_imports(source, tmp.path()).unwrap();
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].calls[0].config.family, "Inter");
    }
}
