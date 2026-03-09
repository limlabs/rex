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

/// Extract binding names from an export declaration.
fn exported_decl_names<'a>(decl: &'a oxc_ast::ast::Declaration<'a>) -> Vec<&'a str> {
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

/// Maps Node.js built-in modules + `cloudflare:sockets` to server-side polyfill stubs.
/// Used by both the pages-router server bundle and RSC server/SSR bundles.
pub(crate) fn node_polyfill_aliases(runtime_dir: &Path) -> Vec<(String, Vec<Option<String>>)> {
    let modules: &[(&str, &str)] = &[
        ("process", "process.ts"),
        ("node:process", "process.ts"),
        ("fs", "fs.ts"),
        ("node:fs", "fs.ts"),
        ("fs/promises", "fs-promises.ts"),
        ("node:fs/promises", "fs-promises.ts"),
        ("path", "path.ts"),
        ("node:path", "path.ts"),
        ("buffer", "buffer.ts"),
        ("node:buffer", "buffer.ts"),
        ("crypto", "crypto.ts"),
        ("node:crypto", "crypto.ts"),
        ("events", "events.ts"),
        ("node:events", "events.ts"),
        ("net", "net.ts"),
        ("node:net", "net.ts"),
        ("tls", "tls.ts"),
        ("node:tls", "tls.ts"),
        ("dns", "dns.ts"),
        ("node:dns", "dns.ts"),
        ("os", "os.ts"),
        ("node:os", "os.ts"),
        ("stream", "stream.ts"),
        ("node:stream", "stream.ts"),
        ("string_decoder", "string_decoder.ts"),
        ("node:string_decoder", "string_decoder.ts"),
        ("util", "util.ts"),
        ("node:util", "util.ts"),
        ("url", "url-module.ts"),
        ("node:url", "url-module.ts"),
        ("stream/web", "stream-web.ts"),
        ("node:stream/web", "stream-web.ts"),
        ("child_process", "child_process.ts"),
        ("node:child_process", "child_process.ts"),
        ("assert", "assert.ts"),
        ("node:assert", "assert.ts"),
        ("module", "module.ts"),
        ("node:module", "module.ts"),
        ("http", "http.ts"),
        ("node:http", "http.ts"),
        ("https", "https.ts"),
        ("node:https", "https.ts"),
        ("zlib", "zlib.ts"),
        ("node:zlib", "zlib.ts"),
        ("worker_threads", "worker_threads.ts"),
        ("node:worker_threads", "worker_threads.ts"),
        ("http2", "http2.ts"),
        ("node:http2", "http2.ts"),
        ("cloudflare:sockets", "cloudflare-sockets.ts"),
        // file-type stub — Node.js condition entry has fileTypeFromFile but
        // causes issues with other packages. Provide stub for server bundles.
        ("file-type", "file-type.ts"),
        // sharp — native image library, needs C++ bindings not available in V8
        ("sharp", "sharp.ts"),
    ];

    // next/* → Rex equivalents for Next.js projects (only if files exist)
    let next_mappings: &[(&str, &str)] = &[
        ("next/link", "next-link.ts"),
        ("next/image", "next-image.ts"),
        ("next/head", "head.ts"),
        ("next/router", "next-router.ts"),
        ("next/navigation", "next-navigation.ts"),
        ("next/headers", "next-headers.ts"),
        ("next/cache", "next-cache.ts"),
        ("next/server", "next-server.ts"),
        ("next/font/google", "next-font.ts"),
        ("next/font/local", "next-font.ts"),
        ("next/dynamic", "next-dynamic.ts"),
    ];
    let make_alias = |spec: &str, file: &str| {
        (
            spec.to_string(),
            vec![Some(runtime_dir.join(file).to_string_lossy().to_string())],
        )
    };
    let mut aliases: Vec<_> = modules.iter().map(|(s, f)| make_alias(s, f)).collect();
    for (specifier, file) in next_mappings {
        if runtime_dir.join(file).exists() {
            aliases.push(make_alias(specifier, file));
        }
    }
    aliases
}

/// Parse tsconfig.json `paths` from the project root and return rolldown-compatible
/// resolve aliases. Handles wildcard patterns (e.g. `"@/*": ["./src/*"]`).
///
/// Returns an empty Vec if tsconfig.json doesn't exist or has no paths.
pub(crate) fn tsconfig_path_aliases(project_root: &Path) -> Vec<(String, Vec<Option<String>>)> {
    let tsconfig_path = project_root.join("tsconfig.json");
    let content = match fs::read_to_string(&tsconfig_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    // Strip single-line comments (tsconfig allows them)
    let stripped: String = content
        .lines()
        .map(|line| {
            if let Some(idx) = line.find("//") {
                // Only strip if not inside a string
                let before = &line[..idx];
                if before.matches('"').count() % 2 == 0 {
                    return before;
                }
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n");

    let parsed: serde_json::Value = match serde_json::from_str(&stripped) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let paths = match parsed
        .get("compilerOptions")
        .and_then(|co| co.get("paths"))
        .and_then(|p| p.as_object())
    {
        Some(p) => p,
        None => return Vec::new(),
    };

    let base_url = parsed
        .get("compilerOptions")
        .and_then(|co| co.get("baseUrl"))
        .and_then(|b| b.as_str())
        .unwrap_or(".");
    let base_dir = project_root.join(base_url);

    let mut aliases = Vec::new();
    for (key, value) in paths {
        let targets = match value.as_array() {
            Some(arr) => arr,
            None => continue,
        };
        let target = match targets.first().and_then(|t| t.as_str()) {
            Some(t) => t,
            None => continue,
        };

        if key.ends_with("/*") && target.ends_with("/*") {
            // Wildcard: "@/*" → "./src/*" becomes "@" → "{base}/src"
            let alias_key = key[..key.len() - 2].to_string();
            let alias_target = base_dir
                .join(&target[..target.len() - 2])
                .to_string_lossy()
                .to_string();
            aliases.push((alias_key, vec![Some(alias_target)]));
        } else {
            // Exact: "@payload-config" → "./payload.config.ts"
            aliases.push((
                key.clone(),
                vec![Some(base_dir.join(target).to_string_lossy().to_string())],
            ));
        }
    }

    // Always map /public → {project_root}/public (Next.js convention for
    // absolute-path asset imports like `/public/image.svg`)
    let public_dir = project_root.join("public");
    if public_dir.exists() {
        aliases.push((
            "/public".to_string(),
            vec![Some(public_dir.to_string_lossy().to_string())],
        ));
    }

    aliases
}

/// Get the path to the server runtime files.
/// These are embedded in the source tree at runtime/server/.
pub fn runtime_server_dir() -> Result<PathBuf> {
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
#[allow(clippy::unwrap_used)]
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
    fn test_detect_strategy_gssp_async() {
        let source = r#"
            export default function Page() { return <div/>; }
            export async function getServerSideProps(ctx) { return { props: {} }; }
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
    fn test_extract_middleware_matchers_single_line() {
        let source = r#"
export function middleware(req) { return NextResponse.next(); }
export const config = { matcher: ['/protected'] }
"#;
        let matchers = extract_middleware_matchers(source);
        assert_eq!(matchers, vec!["/protected"]);
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

    #[test]
    fn test_runtime_server_dir_exists() {
        let dir = runtime_server_dir().unwrap();
        assert!(dir.exists());
        assert!(dir.join("head.ts").exists());
    }

    #[test]
    fn test_runtime_client_dir_exists() {
        let dir = runtime_client_dir().unwrap();
        assert!(dir.exists());
        assert!(dir.join("link.ts").exists());
    }
}
