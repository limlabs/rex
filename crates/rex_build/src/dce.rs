//! Dead code elimination for server-only exports.
//!
//! Strips `getServerSideProps` and `getStaticProps` exports (and their
//! exclusively-referenced dependencies) from page source files before
//! client bundling. This prevents server-only code from leaking into
//! client bundles.

use std::collections::HashSet;

use anyhow::Result;
use oxc_allocator::Allocator;
use oxc_ast::ast::{Declaration, ImportDeclarationSpecifier, Program, Statement};
use oxc_ast::AstKind;
use oxc_semantic::{AstNodes, SemanticBuilder};
use oxc_span::{GetSpan, Ident, SourceType, Span};
use oxc_syntax::node::NodeId;
use oxc_syntax::symbol::SymbolId;

/// Names of server-only exports to eliminate from client bundles.
const SERVER_EXPORTS: &[&str] = &["getServerSideProps", "getStaticProps", "getStaticPaths"];

/// Strip server-only exports and their exclusive dependencies from source code.
///
/// Returns the modified source with `getServerSideProps`/`getStaticProps` and
/// any imports/declarations referenced only within them removed.
///
/// Side-effect imports (`import './styles.css'`) are never removed.
pub fn strip_server_exports(source: &str, source_type: SourceType) -> Result<String> {
    let allocator = Allocator::default();
    let ret = oxc_parser::Parser::new(&allocator, source, source_type).parse();

    if !ret.errors.is_empty() {
        // If parsing fails, return source unchanged
        tracing::warn!("DCE: parse errors, skipping strip");
        return Ok(source.to_string());
    }

    let program = allocator.alloc(ret.program);
    let spans = analyze(program, source);

    if spans.is_empty() {
        return Ok(source.to_string());
    }

    Ok(apply_removals(source, &spans))
}

/// Analyze the program to find spans that should be removed.
fn analyze<'a>(program: &'a Program<'a>, _source: &str) -> Vec<Span> {
    let sem_ret = SemanticBuilder::new().build(program);
    if !sem_ret.errors.is_empty() {
        tracing::warn!("DCE: semantic analysis errors, skipping strip");
        return vec![];
    }
    let semantic = sem_ret.semantic;
    let scoping = semantic.scoping();
    let nodes = semantic.nodes();

    // Step 1: Find server export symbols and their enclosing statement spans.
    let mut server_symbols: HashSet<SymbolId> = HashSet::new();
    let mut server_stmt_spans: Vec<Span> = Vec::new();

    for &name in SERVER_EXPORTS {
        if let Some(sym_id) = scoping.get_root_binding(Ident::new_const(name)) {
            server_symbols.insert(sym_id);
            let decl_node_id = scoping.symbol_declaration(sym_id);
            if let Some(span) = find_top_level_stmt_span(nodes, decl_node_id) {
                server_stmt_spans.push(span);
            }
        }
    }

    if server_symbols.is_empty() {
        return vec![];
    }

    // Step 2: Iteratively find dead symbols.
    // A symbol is dead if ALL its references are inside dead spans (server
    // exports or other dead declarations). This must iterate to a fixpoint
    // because removing one symbol may make its dependencies dead too.
    let mut dead_symbols: HashSet<SymbolId> = server_symbols.clone();
    let mut dead_spans: Vec<Span> = server_stmt_spans.clone();

    // Build a sorted index of all identifier references for binary-search lookups.
    let ref_index = IdentRefIndex::build(nodes, scoping);

    loop {
        // Collect all symbols referenced from within any dead span.
        let mut candidates: HashSet<SymbolId> = HashSet::new();
        for span in &dead_spans {
            ref_index.collect_in_span(*span, &mut candidates);
        }
        // Remove already-known dead symbols from candidates.
        for &sym_id in &dead_symbols {
            candidates.remove(&sym_id);
        }

        // Check each candidate: if ALL its references are inside dead spans,
        // it's dead too.
        let mut newly_dead: Vec<SymbolId> = Vec::new();
        for &cand in &candidates {
            let ref_ids = scoping.get_resolved_reference_ids(cand);
            let all_in_dead = ref_ids.iter().all(|&rid| {
                let ref_node_span = nodes
                    .get_node(scoping.get_reference(rid).node_id())
                    .kind()
                    .span();
                dead_spans
                    .iter()
                    .any(|s| ref_node_span.start >= s.start && ref_node_span.end <= s.end)
            });
            if all_in_dead {
                newly_dead.push(cand);
            }
        }

        if newly_dead.is_empty() {
            break; // fixpoint reached
        }

        // Add newly dead symbols and their declaration spans.
        for sym_id in newly_dead {
            dead_symbols.insert(sym_id);
            let decl_node_id = scoping.symbol_declaration(sym_id);
            if let Some(span) = find_top_level_stmt_span(nodes, decl_node_id) {
                dead_spans.push(span);
            }
        }
    }

    // Remove server exports from dead_symbols (they're tracked separately in
    // server_stmt_spans already).
    for &sym_id in &server_symbols {
        dead_symbols.remove(&sym_id);
    }

    // Step 4: Build removal spans — server export statements + dead imports/declarations.
    let mut remove_spans: Vec<Span> = server_stmt_spans;

    for stmt in &program.body {
        match stmt {
            Statement::ImportDeclaration(import) => {
                // Never remove side-effect imports (no specifiers).
                let Some(specifiers) = &import.specifiers else {
                    continue;
                };
                if specifiers.is_empty() {
                    continue;
                }
                // Never remove type-only imports.
                if import.import_kind.is_type() {
                    continue;
                }

                let total = specifiers.len();
                let dead_count = specifiers
                    .iter()
                    .filter(|spec| {
                        let local = specifier_local_name(spec);
                        scoping
                            .get_root_binding(Ident::new_const(local))
                            .is_some_and(|sid| dead_symbols.contains(&sid))
                    })
                    .count();

                if dead_count == total {
                    // All specifiers dead → remove entire import.
                    remove_spans.push(import.span);
                }
                // Partially dead imports are kept as-is. In practice, server-only
                // imports rarely share specifiers with client code. Rolldown's own
                // tree-shaking handles the remaining unused specifiers.
            }
            // Top-level variable/function declarations (not exported) that are dead.
            Statement::VariableDeclaration(decl) => {
                let all_dead = decl.declarations.iter().all(|d| {
                    binding_name(d)
                        .and_then(|name| scoping.get_root_binding(Ident::new_const(name)))
                        .is_some_and(|sid| dead_symbols.contains(&sid))
                });
                if all_dead {
                    remove_spans.push(decl.span);
                }
            }
            Statement::FunctionDeclaration(func) => {
                if let Some(id) = &func.id {
                    if scoping
                        .get_root_binding(Ident::new_const(id.name.as_str()))
                        .is_some_and(|sid| dead_symbols.contains(&sid))
                    {
                        remove_spans.push(func.span);
                    }
                }
            }
            // Exported declarations that aren't the server exports themselves
            // but are dead (only referenced from server code).
            Statement::ExportNamedDeclaration(export) => {
                if let Some(decl) = &export.declaration {
                    let all_dead = match decl {
                        Declaration::VariableDeclaration(vd) => vd.declarations.iter().all(|d| {
                            binding_name(d)
                                .and_then(|name| scoping.get_root_binding(Ident::new_const(name)))
                                .is_some_and(|sid| dead_symbols.contains(&sid))
                        }),
                        Declaration::FunctionDeclaration(f) => {
                            f.id.as_ref()
                                .and_then(|id| {
                                    scoping.get_root_binding(Ident::new_const(id.name.as_str()))
                                })
                                .is_some_and(|sid| dead_symbols.contains(&sid))
                        }
                        _ => false,
                    };
                    if all_dead && !is_server_export_span(&remove_spans, export.span) {
                        remove_spans.push(export.span);
                    }
                }
            }
            _ => {}
        }
    }

    // Sort and deduplicate.
    remove_spans.sort_by_key(|s| s.start);
    remove_spans.dedup_by(|a, b| {
        // Merge overlapping/adjacent spans.
        // In dedup_by, `a` is the later element, `b` is the earlier retained one.
        // They overlap if the later span starts within (or right after) the earlier span.
        if a.start <= b.end {
            b.end = b.end.max(a.end);
            true
        } else {
            false
        }
    });

    remove_spans
}

/// Walk ancestor nodes from a declaration to find the enclosing top-level statement span.
fn find_top_level_stmt_span(nodes: &AstNodes, node_id: NodeId) -> Option<Span> {
    for ancestor_id in nodes.ancestor_ids(node_id) {
        match nodes.get_node(ancestor_id).kind() {
            AstKind::ExportNamedDeclaration(decl) => return Some(decl.span),
            AstKind::ExportDefaultDeclaration(decl) => return Some(decl.span),
            AstKind::Program(_) => {
                // The node itself is a top-level declaration (not exported).
                let node_kind = nodes.get_node(node_id).kind();
                return Some(node_kind.span());
            }
            _ => continue,
        }
    }
    None
}

/// All identifier references in the AST, sorted by start position.
/// Built once and reused for binary-search lookups per dead span.
struct IdentRefIndex {
    /// (start, symbol_id) sorted by start position.
    entries: Vec<(u32, SymbolId)>,
}

impl IdentRefIndex {
    fn build(nodes: &AstNodes, scoping: &oxc_semantic::Scoping) -> Self {
        let mut entries = Vec::new();
        for node in nodes.iter() {
            if let AstKind::IdentifierReference(ident_ref) = node.kind() {
                if let Some(ref_id) = ident_ref.reference_id.get() {
                    let reference = scoping.get_reference(ref_id);
                    if let Some(sym_id) = reference.symbol_id() {
                        entries.push((node.kind().span().start, sym_id));
                    }
                }
            }
        }
        entries.sort_unstable_by_key(|&(start, _)| start);
        Self { entries }
    }

    /// Collect all symbol references within `container_span` into `out`.
    /// Uses binary search to find the start, then scans forward — O(log N + matches).
    fn collect_in_span(&self, container_span: Span, out: &mut HashSet<SymbolId>) {
        let start_idx = self
            .entries
            .partition_point(|&(pos, _)| pos < container_span.start);
        for &(pos, sym_id) in &self.entries[start_idx..] {
            if pos >= container_span.end {
                break;
            }
            out.insert(sym_id);
        }
    }
}

/// Get the local binding name from an import specifier.
fn specifier_local_name<'a>(spec: &'a ImportDeclarationSpecifier<'a>) -> &'a str {
    match spec {
        ImportDeclarationSpecifier::ImportSpecifier(s) => s.local.name.as_str(),
        ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => s.local.name.as_str(),
        ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => s.local.name.as_str(),
    }
}

/// Get the binding name from a variable declarator (simple identifiers only).
fn binding_name<'a>(decl: &'a oxc_ast::ast::VariableDeclarator<'a>) -> Option<&'a str> {
    match &decl.id {
        oxc_ast::ast::BindingPattern::BindingIdentifier(id) => Some(id.name.as_str()),
        _ => None,
    }
}

/// Check if a span is already in the server export removal list.
fn is_server_export_span(spans: &[Span], span: Span) -> bool {
    spans
        .iter()
        .any(|s| s.start == span.start && s.end == span.end)
}

/// Apply span removals to produce the output string.
/// Extends each span to consume trailing whitespace/newlines for clean output.
fn apply_removals(source: &str, spans: &[Span]) -> String {
    let bytes = source.as_bytes();
    let mut output = String::with_capacity(source.len());
    let mut cursor: usize = 0;

    for span in spans {
        let start = span.start as usize;
        let mut end = span.end as usize;

        // Consume trailing whitespace and one newline for clean output.
        while end < bytes.len() && (bytes[end] == b' ' || bytes[end] == b'\t') {
            end += 1;
        }
        if end < bytes.len() && bytes[end] == b'\n' {
            end += 1;
        } else if end + 1 < bytes.len() && bytes[end] == b'\r' && bytes[end + 1] == b'\n' {
            end += 2;
        }

        if start > cursor {
            output.push_str(&source[cursor..start]);
        }
        cursor = end;
    }

    if cursor < source.len() {
        output.push_str(&source[cursor..]);
    }

    output
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn strip(source: &str) -> String {
        strip_server_exports(source, SourceType::tsx()).unwrap()
    }

    #[test]
    fn no_server_exports_unchanged() {
        let source = r#"
export default function Page({ data }) {
    return <div>{data}</div>;
}
"#;
        assert_eq!(strip(source), source);
    }

    #[test]
    fn strips_gssp_function() {
        let source = r#"export async function getServerSideProps(ctx) {
    return { props: { name: "world" } };
}

export default function Page({ name }) {
    return <div>{name}</div>;
}
"#;
        let result = strip(source);
        assert!(!result.contains("getServerSideProps"));
        assert!(result.contains("export default function Page"));
    }

    #[test]
    fn strips_gssp_const_arrow() {
        let source = r#"export const getServerSideProps = async (ctx) => {
    return { props: { name: "world" } };
};

export default function Page({ name }) {
    return <div>{name}</div>;
}
"#;
        let result = strip(source);
        assert!(!result.contains("getServerSideProps"));
        assert!(result.contains("export default function Page"));
    }

    #[test]
    fn strips_gsp() {
        let source = r#"export async function getStaticProps() {
    return { props: { count: 42 } };
}

export default function Page({ count }) {
    return <div>{count}</div>;
}
"#;
        let result = strip(source);
        assert!(!result.contains("getStaticProps"));
        assert!(result.contains("export default function Page"));
    }

    #[test]
    fn strips_server_only_import() {
        let source = r#"import { fetchData } from './server-utils';

export async function getServerSideProps(ctx) {
    const data = fetchData(ctx.params.id);
    return { props: { data } };
}

export default function Page({ data }) {
    return <div>{data}</div>;
}
"#;
        let result = strip(source);
        assert!(!result.contains("getServerSideProps"));
        assert!(!result.contains("fetchData"));
        assert!(!result.contains("server-utils"));
        assert!(result.contains("export default function Page"));
    }

    #[test]
    fn preserves_shared_import() {
        let source = r#"import { formatDate } from './utils';

export async function getServerSideProps(ctx) {
    const date = formatDate(new Date());
    return { props: { date } };
}

export default function Page({ date }) {
    return <div>{formatDate(date)}</div>;
}
"#;
        let result = strip(source);
        assert!(!result.contains("getServerSideProps"));
        // formatDate is used by both server and client, so the import stays.
        assert!(result.contains("formatDate"));
        assert!(result.contains("utils"));
    }

    #[test]
    fn preserves_side_effect_imports() {
        let source = r#"import './styles.css';
import { db } from './database';

export async function getServerSideProps() {
    const data = db.query('SELECT *');
    return { props: { data } };
}

export default function Page({ data }) {
    return <div>{data}</div>;
}
"#;
        let result = strip(source);
        assert!(!result.contains("getServerSideProps"));
        assert!(!result.contains("database"));
        // Side-effect import preserved.
        assert!(result.contains("import './styles.css'"));
    }

    #[test]
    fn no_server_exports_returns_same() {
        let source = r#"export default function Page() {
    return <div>Hello</div>;
}
"#;
        let result = strip(source);
        assert_eq!(result, source);
    }

    #[test]
    fn preserves_partially_dead_import() {
        let source = r#"import { serverFn, sharedFn } from './utils';

export async function getServerSideProps() {
    const data = serverFn();
    return { props: { data } };
}

export default function Page({ data }) {
    return <div>{sharedFn(data)}</div>;
}
"#;
        let result = strip(source);
        assert!(!result.contains("getServerSideProps"));
        // The import must be preserved because sharedFn is still used.
        assert!(result.contains("import { serverFn, sharedFn } from './utils'"));
        assert!(result.contains("sharedFn"));
    }

    #[test]
    fn strips_server_only_top_level_function() {
        let source = r#"import { db } from './database';

function fetchPosts() {
    return db.query('SELECT * FROM posts');
}

export async function getServerSideProps() {
    const posts = fetchPosts();
    return { props: { posts } };
}

export default function Page({ posts }) {
    return <div>{posts.length}</div>;
}
"#;
        let result = strip(source);
        assert!(!result.contains("getServerSideProps"));
        assert!(!result.contains("fetchPosts"));
        assert!(!result.contains("database"));
        assert!(result.contains("export default function Page"));
    }

    #[test]
    fn strips_fs_import_preserves_default_export() {
        let source = r#"
                import fs from 'fs';
                export default function Home({ content }) {
                    return <div><h1>{content}</h1></div>;
                }
                export function getServerSideProps() {
                    const content = fs.readFileSync('data/message.txt', 'utf8');
                    return { props: { content } };
                }
                "#;
        let result = strip(source);
        assert!(!result.contains("getServerSideProps"));
        assert!(!result.contains("import fs from 'fs'"));
        assert!(
            result.contains("export default function Home"),
            "Default export should be preserved"
        );
    }
}
