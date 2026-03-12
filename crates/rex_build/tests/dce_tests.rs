//! Tests for dead code elimination (dce.rs) — covers edge cases not hit by inline tests.
#![allow(clippy::unwrap_used)]

use oxc_span::SourceType;
use rex_build::dce::strip_server_exports;

fn strip(source: &str) -> String {
    strip_server_exports(source, SourceType::tsx()).unwrap()
}

#[test]
fn strips_non_exported_variable_used_only_by_gssp() {
    // Covers: top-level VariableDeclaration removal (lines 175-183)
    let source = r#"
const DB_URL = "postgres://localhost/test";

export async function getServerSideProps() {
    const url = DB_URL;
    return { props: { url } };
}

export default function Page({ url }) {
    return <div>{url}</div>;
}
"#;
    let result = strip(source);
    assert!(
        !result.contains("DB_URL"),
        "dead variable should be removed"
    );
    assert!(!result.contains("getServerSideProps"));
    assert!(result.contains("export default function Page"));
}

#[test]
fn strips_chain_of_dead_helpers() {
    // Tests iterative dead symbol discovery (fixpoint loop)
    let source = r#"
function innerHelper() { return 42; }
function outerHelper() { return innerHelper(); }

export async function getServerSideProps() {
    return { props: { val: outerHelper() } };
}

export default function Page({ val }) {
    return <div>{val}</div>;
}
"#;
    let result = strip(source);
    assert!(!result.contains("innerHelper"));
    assert!(!result.contains("outerHelper"));
    assert!(!result.contains("getServerSideProps"));
    assert!(result.contains("export default function Page"));
}

#[test]
fn handles_windows_line_endings() {
    // Covers: \r\n trailing whitespace handling (lines 338-339)
    let source =
        "export function getServerSideProps() {\r\n    return { props: {} };\r\n}\r\n\r\nexport default function Page() {\r\n    return <div/>;\r\n}\r\n";
    let result = strip(source);
    assert!(!result.contains("getServerSideProps"));
    assert!(result.contains("export default function Page"));
}

#[test]
fn strips_namespace_import_used_only_by_gssp() {
    // Covers: ImportNamespaceSpecifier path (line 301)
    let source = r#"
import * as db from './database';

export async function getServerSideProps() {
    const data = db.query('SELECT *');
    return { props: { data } };
}

export default function Page({ data }) {
    return <div>{data}</div>;
}
"#;
    let result = strip(source);
    assert!(
        !result.contains("import * as db"),
        "namespace import should be removed"
    );
    assert!(!result.contains("getServerSideProps"));
}

#[test]
fn preserves_type_only_imports() {
    // Covers: import_kind.is_type() check (line 151-152)
    let source = r#"
import type { ServerContext } from './types';
import { db } from './database';

export async function getServerSideProps(ctx: ServerContext) {
    const data = db.query('SELECT *');
    return { props: { data } };
}

export default function Page({ data }) {
    return <div>{data}</div>;
}
"#;
    let result = strip(source);
    assert!(!result.contains("getServerSideProps"));
    // Type-only imports should be preserved
    assert!(
        result.contains("import type"),
        "type-only import should be preserved"
    );
}

#[test]
fn strips_gssp_with_trailing_whitespace() {
    // Covers: trailing whitespace consumption in apply_removals (lines 332-334)
    let source = "export async function getServerSideProps() {    \n    return { props: {} };    \n}    \n\nexport default function Page() {\n    return <div/>;\n}\n";
    let result = strip(source);
    assert!(!result.contains("getServerSideProps"));
    assert!(result.contains("export default function Page"));
}

#[test]
fn strips_exported_dead_variable_declaration() {
    // Covers: ExportNamedDeclaration with VariableDeclaration dead check (lines 199-204)
    let source = r#"
export const serverConfig = { timeout: 5000 };

export async function getServerSideProps() {
    const cfg = serverConfig;
    return { props: { timeout: cfg.timeout } };
}

export default function Page({ timeout }) {
    return <div>{timeout}</div>;
}
"#;
    let result = strip(source);
    assert!(!result.contains("getServerSideProps"));
    assert!(
        !result.contains("serverConfig"),
        "dead exported variable should be removed"
    );
}
