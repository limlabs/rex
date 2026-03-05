use anyhow::Result;
use rex_core::DataStrategy;
use std::fs;
use std::path::{Path, PathBuf};

/// Map a route to a chunk name for rolldown entry naming.
pub(crate) fn route_to_chunk_name(route: &rex_core::Route) -> String {
    let module_name = route.module_name();
    let cn = module_name.replace('/', "-").replace(['[', ']'], "_");
    if cn.is_empty() {
        "index".to_string()
    } else {
        cn
    }
}

/// Find the route that matches a given chunk name.
pub(crate) fn find_route_for_chunk<'a>(
    chunk_name: &str,
    routes: &'a [rex_core::Route],
) -> Option<&'a rex_core::Route> {
    routes.iter().find(|r| route_to_chunk_name(r) == chunk_name)
}

/// Detect data strategy by scanning source for exported getServerSideProps / getStaticProps.
pub(crate) fn detect_data_strategy(source_path: &Path) -> Result<DataStrategy> {
    let source = fs::read_to_string(source_path)?;
    detect_data_strategy_from_source(&source)
}

/// Detect data strategy from source content (no filesystem access).
pub(crate) fn detect_data_strategy_from_source(source: &str) -> Result<DataStrategy> {
    let has_gssp = source.lines().any(|l| {
        let t = l.trim();
        t.contains("getServerSideProps") && (t.starts_with("export ") || t.starts_with("export{"))
    });
    let has_gsp = source.lines().any(|l| {
        let t = l.trim();
        t.contains("getStaticProps") && (t.starts_with("export ") || t.starts_with("export{"))
    });
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

/// Generate a build ID based on current timestamp
pub(crate) fn generate_build_id() -> String {
    use sha2::{Digest, Sha256};
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before UNIX epoch")
        .as_millis();
    let hash = Sha256::digest(timestamp.to_string().as_bytes());
    hex::encode(&hash[..8])
}

/// Get the path to the client runtime files.
/// These are embedded in the source tree at runtime/client/.
pub(crate) fn runtime_client_dir() -> Result<PathBuf> {
    // In dev: relative to the crate source
    // The runtime files are at the workspace root under runtime/client/
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let runtime_dir = manifest_dir.join("../../runtime/client");
    if runtime_dir.exists() {
        return Ok(runtime_dir.canonicalize()?);
    }
    // Fallback: look relative to current dir
    let cwd_runtime = PathBuf::from("runtime/client");
    if cwd_runtime.exists() {
        return Ok(cwd_runtime.canonicalize()?);
    }
    // Distributed binary: extract embedded runtime files to temp dir
    crate::embedded_runtime::client_dir()
}

/// Get the path to the server runtime files.
/// These are embedded in the source tree at runtime/server/.
pub(crate) fn runtime_server_dir() -> Result<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let runtime_dir = manifest_dir.join("../../runtime/server");
    if runtime_dir.exists() {
        return Ok(runtime_dir.canonicalize()?);
    }
    let cwd_runtime = PathBuf::from("runtime/server");
    if cwd_runtime.exists() {
        return Ok(cwd_runtime.canonicalize()?);
    }
    // Distributed binary: extract embedded runtime files to temp dir
    crate::embedded_runtime::server_dir()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rex_core::{PageType, Route};
    use std::path::PathBuf;

    fn make_route(pattern: &str, file_path: &str) -> Route {
        Route {
            pattern: pattern.to_string(),
            file_path: PathBuf::from(file_path),
            abs_path: PathBuf::from(file_path),
            dynamic_segments: vec![],
            page_type: PageType::Regular,
            specificity: 0,
        }
    }

    #[test]
    fn test_route_to_chunk_name_index() {
        let route = make_route("/", "index.tsx");
        assert_eq!(route_to_chunk_name(&route), "index");
    }

    #[test]
    fn test_route_to_chunk_name_nested() {
        let route = make_route("/blog/:slug", "blog/[slug].tsx");
        assert_eq!(route_to_chunk_name(&route), "blog-_slug_");
    }

    #[test]
    fn test_route_to_chunk_name_deep_nested() {
        let route = make_route("/docs/api/:path*", "docs/api/[...path].tsx");
        assert_eq!(route_to_chunk_name(&route), "docs-api-_...path_");
    }

    #[test]
    fn test_find_route_for_chunk_found() {
        let routes = vec![
            make_route("/", "index.tsx"),
            make_route("/about", "about.tsx"),
            make_route("/blog/:slug", "blog/[slug].tsx"),
        ];
        let found = find_route_for_chunk("about", &routes);
        assert!(found.is_some());
        assert_eq!(found.expect("route should exist").pattern, "/about");
    }

    #[test]
    fn test_find_route_for_chunk_not_found() {
        let routes = vec![make_route("/", "index.tsx")];
        assert!(find_route_for_chunk("nonexistent", &routes).is_none());
    }

    #[test]
    fn test_generate_build_id_format() {
        let id = generate_build_id();
        assert_eq!(id.len(), 16, "build ID should be 16 hex chars");
        assert!(
            id.chars().all(|c| c.is_ascii_hexdigit()),
            "build ID should contain only hex chars"
        );
    }

    #[test]
    fn test_extract_middleware_matchers_multiline() {
        let source = r#"
export const config = {
    matcher: [
        '/dashboard/:path*',
        '/api/admin/:path*',
        '/settings'
    ]
}
"#;
        let matchers = extract_middleware_matchers(source);
        assert_eq!(
            matchers,
            vec!["/dashboard/:path*", "/api/admin/:path*", "/settings"]
        );
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
    fn test_extract_middleware_matchers_with_comments() {
        let source = r#"
export const config = {
    // Apply to dashboard and API routes
    matcher: [
        '/dashboard/:path*', // admin pages
        '/api/:path*' // API routes
    ]
}
"#;
        let matchers = extract_middleware_matchers(source);
        assert_eq!(matchers, vec!["/dashboard/:path*", "/api/:path*"]);
    }

    #[test]
    fn test_extract_middleware_matchers_trailing_comma() {
        let source = r#"
export const config = {
    matcher: [
        '/dashboard/:path*',
        '/api/:path*',
    ],
}
"#;
        let matchers = extract_middleware_matchers(source);
        assert_eq!(matchers, vec!["/dashboard/:path*", "/api/:path*"]);
    }
}
