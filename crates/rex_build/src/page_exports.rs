//! Page export detection helpers.
//!
//! Functions for scanning page source files for data-strategy exports
//! (`getServerSideProps`, `getStaticProps`, `getStaticPaths`) and middleware
//! matcher extraction.

use anyhow::Result;
use rex_core::DataStrategy;
use std::fs;
use std::path::Path;

/// Detect data strategy by scanning source for exported getServerSideProps / getStaticProps.
pub fn detect_data_strategy(source_path: &Path) -> Result<DataStrategy> {
    let source = fs::read_to_string(source_path)?;
    detect_data_strategy_from_source(&source)
}

/// Detect data strategy from source content (no filesystem access).
///
/// Uses the OXC parser to find exported `getServerSideProps` / `getStaticProps`
/// via proper AST analysis instead of line-by-line string matching.
pub(crate) fn detect_data_strategy_from_source(source: &str) -> Result<DataStrategy> {
    use oxc_ast::ast::{ExportDefaultDeclarationKind, Statement};

    let allocator = oxc_allocator::Allocator::default();
    let source_type = oxc_span::SourceType::tsx();
    let parsed = oxc_parser::Parser::new(&allocator, source, source_type).parse();

    if !parsed.errors.is_empty() {
        return Ok(DataStrategy::None);
    }

    let mut has_gssp = false;
    let mut has_gsp = false;

    for stmt in &parsed.program.body {
        match stmt {
            Statement::ExportNamedDeclaration(export) => {
                // Re-exports: export { getServerSideProps } from '...'
                for spec in &export.specifiers {
                    match spec.exported.name().as_ref() {
                        "getServerSideProps" => has_gssp = true,
                        "getStaticProps" => has_gsp = true,
                        _ => {}
                    }
                }
                // Inline declarations: export function/const getServerSideProps ...
                if let Some(decl) = &export.declaration {
                    for name in exported_decl_names(decl) {
                        match name {
                            "getServerSideProps" => has_gssp = true,
                            "getStaticProps" => has_gsp = true,
                            _ => {}
                        }
                    }
                }
            }
            Statement::ExportDefaultDeclaration(export) => {
                if let ExportDefaultDeclarationKind::Identifier(id) = &export.declaration {
                    match id.name.as_str() {
                        "getServerSideProps" => has_gssp = true,
                        "getStaticProps" => has_gsp = true,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    if has_gssp && has_gsp {
        anyhow::bail!("Page exports both getStaticProps and getServerSideProps");
    }
    if has_gsp {
        return Ok(DataStrategy::GetStaticProps);
    }
    if has_gssp {
        return Ok(DataStrategy::GetServerSideProps);
    }
    Ok(DataStrategy::None)
}

/// Detect whether a page source exports `getStaticPaths`.
pub fn detect_has_static_paths(source_path: &Path) -> Result<bool> {
    let source = fs::read_to_string(source_path)?;
    Ok(detect_has_static_paths_from_source(&source))
}

/// Detect whether source content exports `getStaticPaths` (no filesystem access).
pub(crate) fn detect_has_static_paths_from_source(source: &str) -> bool {
    use oxc_ast::ast::{ExportDefaultDeclarationKind, Statement};

    let allocator = oxc_allocator::Allocator::default();
    let source_type = oxc_span::SourceType::tsx();
    let parsed = oxc_parser::Parser::new(&allocator, source, source_type).parse();

    if !parsed.errors.is_empty() {
        return false;
    }

    for stmt in &parsed.program.body {
        match stmt {
            Statement::ExportNamedDeclaration(export) => {
                for spec in &export.specifiers {
                    if spec.exported.name().as_ref() == "getStaticPaths" {
                        return true;
                    }
                }
                if let Some(decl) = &export.declaration {
                    for name in exported_decl_names(decl) {
                        if name == "getStaticPaths" {
                            return true;
                        }
                    }
                }
            }
            Statement::ExportDefaultDeclaration(export) => {
                if let ExportDefaultDeclarationKind::Identifier(id) = &export.declaration {
                    if id.name.as_str() == "getStaticPaths" {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }

    false
}

/// Extract binding names from an export declaration.
pub(crate) fn exported_decl_names<'a>(decl: &'a oxc_ast::ast::Declaration<'a>) -> Vec<&'a str> {
    use oxc_ast::ast::Declaration;
    match decl {
        Declaration::FunctionDeclaration(f) => {
            f.id.as_ref()
                .map(|id| vec![id.name.as_str()])
                .unwrap_or_default()
        }
        Declaration::VariableDeclaration(v) => v
            .declarations
            .iter()
            .filter_map(|d| match &d.id {
                oxc_ast::ast::BindingPattern::BindingIdentifier(id) => Some(id.name.as_str()),
                _ => None,
            })
            .collect(),
        Declaration::ClassDeclaration(c) => {
            c.id.as_ref()
                .map(|id| vec![id.name.as_str()])
                .unwrap_or_default()
        }
        _ => vec![],
    }
}

/// Extract middleware matcher patterns from middleware source code.
/// Looks for `export const config = { matcher: [...] }` and extracts string literals.
/// Returns empty vec if no matcher found (meaning: run on all paths).
pub(crate) fn extract_middleware_matchers(source: &str) -> Vec<String> {
    use oxc_ast::ast::{
        ArrayExpressionElement, Declaration, Expression, ObjectPropertyKind, PropertyKey, Statement,
    };

    let allocator = oxc_allocator::Allocator::default();
    let source_type = oxc_span::SourceType::mjs();
    let parsed = oxc_parser::Parser::new(&allocator, source, source_type).parse();

    if !parsed.errors.is_empty() {
        return Vec::new();
    }

    for stmt in &parsed.program.body {
        let Statement::ExportNamedDeclaration(export) = stmt else {
            continue;
        };
        let Some(Declaration::VariableDeclaration(var_decl)) = export.declaration.as_ref() else {
            continue;
        };
        for declarator in &var_decl.declarations {
            let oxc_ast::ast::BindingPattern::BindingIdentifier(ref id) = declarator.id else {
                continue;
            };
            if id.name.as_str() != "config" {
                continue;
            }
            let Some(Expression::ObjectExpression(obj)) = declarator.init.as_ref() else {
                continue;
            };
            for prop in &obj.properties {
                let ObjectPropertyKind::ObjectProperty(prop) = prop else {
                    continue;
                };
                let is_matcher = match &prop.key {
                    PropertyKey::StaticIdentifier(id) => id.name.as_str() == "matcher",
                    PropertyKey::StringLiteral(s) => s.value.as_str() == "matcher",
                    _ => false,
                };
                if !is_matcher {
                    continue;
                }
                return match &prop.value {
                    Expression::ArrayExpression(arr) => arr
                        .elements
                        .iter()
                        .filter_map(|el| match el {
                            ArrayExpressionElement::StringLiteral(s) => Some(s.value.to_string()),
                            _ => None,
                        })
                        .collect(),
                    Expression::StringLiteral(s) => vec![s.value.to_string()],
                    _ => Vec::new(),
                };
            }
        }
    }

    Vec::new()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_detect_strategy_gssp() {
        let source = r#"
            import React from 'react';
            export default function Page() { return <div/>; }
            export function getServerSideProps(ctx) { return { props: {} }; }
        "#;
        assert_eq!(
            detect_data_strategy_from_source(source).unwrap(),
            DataStrategy::GetServerSideProps,
        );
    }

    #[test]
    fn test_detect_strategy_gsp() {
        let source = r#"
            export default function Page() { return <div/>; }
            export function getStaticProps() { return { props: {} }; }
        "#;
        assert_eq!(
            detect_data_strategy_from_source(source).unwrap(),
            DataStrategy::GetStaticProps,
        );
    }

    #[test]
    fn test_detect_strategy_none() {
        let source = r#"
            import React from 'react';
            export default function Page() { return <div>Static</div>; }
        "#;
        assert_eq!(
            detect_data_strategy_from_source(source).unwrap(),
            DataStrategy::None,
        );
    }

    #[test]
    fn test_detect_strategy_both_errors() {
        let source = r#"
            export default function Page() { return <div/>; }
            export function getServerSideProps() { return { props: {} }; }
            export function getStaticProps() { return { props: {} }; }
        "#;
        let err = detect_data_strategy_from_source(source).unwrap_err();
        assert!(
            err.to_string()
                .contains("both getStaticProps and getServerSideProps"),
            "expected dual-export error, got: {err}"
        );
    }

    #[test]
    fn test_detect_strategy_export_default_identifier() {
        // Covers the ExportDefaultDeclaration branch
        let source =
            "function getServerSideProps() { return { props: {} }; }\nexport default getServerSideProps;\n";
        assert_eq!(
            detect_data_strategy_from_source(source).unwrap(),
            DataStrategy::GetServerSideProps,
        );
    }

    #[test]
    fn test_detect_strategy_reexport_syntax() {
        let source = r#"
            export default function Page() { return <div/>; }
            export{ getServerSideProps } from './data';
        "#;
        assert_eq!(
            detect_data_strategy_from_source(source).unwrap(),
            DataStrategy::GetServerSideProps,
        );
    }

    #[test]
    fn test_detect_data_strategy_from_file() {
        let tmp = tempfile::TempDir::new().unwrap();

        let gssp_page = tmp.path().join("gssp.tsx");
        fs::write(
            &gssp_page,
            "export default function Page() {}\nexport function getServerSideProps() { return { props: {} }; }\n",
        )
        .unwrap();
        assert_eq!(
            detect_data_strategy(&gssp_page).unwrap(),
            DataStrategy::GetServerSideProps
        );

        let gsp_page = tmp.path().join("gsp.tsx");
        fs::write(
            &gsp_page,
            "export default function Page() {}\nexport function getStaticProps() { return { props: {} }; }\n",
        )
        .unwrap();
        assert_eq!(
            detect_data_strategy(&gsp_page).unwrap(),
            DataStrategy::GetStaticProps
        );

        let none_page = tmp.path().join("none.tsx");
        fs::write(&none_page, "export default function Page() {}\n").unwrap();
        assert_eq!(
            detect_data_strategy(&none_page).unwrap(),
            DataStrategy::None
        );
    }

    #[test]
    fn test_detect_has_static_paths_function() {
        let source = r#"
            export default function Page() { return <div/>; }
            export function getStaticPaths() { return { paths: [], fallback: false }; }
            export function getStaticProps() { return { props: {} }; }
        "#;
        assert!(detect_has_static_paths_from_source(source));
    }

    #[test]
    fn test_detect_has_static_paths_const() {
        let source = r#"
            export default function Page() { return <div/>; }
            export const getStaticPaths = () => ({ paths: [], fallback: false });
        "#;
        assert!(detect_has_static_paths_from_source(source));
    }

    #[test]
    fn test_detect_has_static_paths_absent() {
        let source = r#"
            export default function Page() { return <div/>; }
            export function getStaticProps() { return { props: {} }; }
        "#;
        assert!(!detect_has_static_paths_from_source(source));
    }

    #[test]
    fn test_detect_has_static_paths_reexport() {
        let source = r#"
            export default function Page() { return <div/>; }
            export { getStaticPaths } from './paths';
        "#;
        assert!(detect_has_static_paths_from_source(source));
    }

    #[test]
    fn test_detect_has_static_paths_from_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let page = tmp.path().join("page.tsx");
        fs::write(
            &page,
            "export default function Page() {}\nexport function getStaticPaths() { return { paths: [], fallback: false }; }\n",
        )
        .unwrap();
        assert!(detect_has_static_paths(&page).unwrap());

        let no_gsp_page = tmp.path().join("no_gsp.tsx");
        fs::write(&no_gsp_page, "export default function Page() {}\n").unwrap();
        assert!(!detect_has_static_paths(&no_gsp_page).unwrap());
    }

    #[test]
    fn test_extract_middleware_matchers_array() {
        let source = r#"
export function middleware(request) {}

export const config = {
    matcher: ['/dashboard/:path*', '/api/admin/:path*']
}
"#;
        let matchers = extract_middleware_matchers(source);
        assert_eq!(matchers, vec!["/dashboard/:path*", "/api/admin/:path*"]);
    }

    #[test]
    fn test_extract_middleware_matchers_single_string() {
        let source = r#"
export const config = {
    matcher: '/api/:path*'
}
"#;
        let matchers = extract_middleware_matchers(source);
        assert_eq!(matchers, vec!["/api/:path*"]);
    }

    #[test]
    fn test_extract_middleware_matchers_no_config() {
        let source = r#"
export function middleware(request) {
    return NextResponse.next();
}
"#;
        let matchers = extract_middleware_matchers(source);
        assert!(matchers.is_empty());
    }

    #[test]
    fn test_extract_middleware_matchers_no_matcher() {
        let source = r#"
export function middleware(request) {}
export const config = { runtime: 'edge' }
"#;
        let matchers = extract_middleware_matchers(source);
        assert!(matchers.is_empty());
    }
}
