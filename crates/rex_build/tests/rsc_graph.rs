//! Integration tests for the RSC module graph analyzer.

#![allow(clippy::unwrap_used)]

use rex_build::rsc_graph::{
    analyze_module_graph, has_use_client_directive, has_use_server_directive,
};
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

#[test]
fn detects_use_server_directive() {
    let source = r#"
"use server";

export async function increment(n: number): Promise<number> {
    return n + 1;
}
"#;
    assert!(has_use_server_directive(source, oxc_span::SourceType::ts()));
}

#[test]
fn no_use_server_for_regular_module() {
    let source = r#"
export function add(a: number, b: number) { return a + b; }
"#;
    assert!(!has_use_server_directive(
        source,
        oxc_span::SourceType::ts()
    ));
}

#[test]
fn use_server_must_be_directive() {
    // "use server" as a regular string expression (not a directive)
    let source = r#"
const x = "use server";
export function add(a: number, b: number) { return a + b; }
"#;
    assert!(!has_use_server_directive(
        source,
        oxc_span::SourceType::ts()
    ));
}

#[test]
fn use_client_and_use_server_conflict() {
    let dir = setup_temp_dir();
    let root = dir.path();

    fs::write(
        root.join("conflict.tsx"),
        "\"use client\";\n\"use server\";\nexport default function Foo() { return null; }\n",
    )
    .unwrap();

    let entries = vec![root.join("conflict.tsx")];
    let result = analyze_module_graph(&entries, root);
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("\"use client\""));
    assert!(err_msg.contains("\"use server\""));
}

#[test]
fn graph_walk_continues_through_use_server() {
    let dir = setup_temp_dir();
    let root = dir.path();

    // Server page imports a "use server" actions module
    fs::write(
        root.join("page.tsx"),
        "import { increment } from './actions';\nexport default function Page() { return null; }\n",
    )
    .unwrap();

    // "use server" module imports a helper
    fs::write(
        root.join("actions.ts"),
        "\"use server\";\nimport { db } from './db';\nexport async function increment(n: number) { return db.inc(n); }\n",
    )
    .unwrap();

    // Server-only helper (should be in the graph because "use server" is server code)
    fs::write(
        root.join("db.ts"),
        "export const db = { inc: (n: number) => n + 1 };\n",
    )
    .unwrap();

    let entries = vec![root.join("page.tsx")];
    let graph = analyze_module_graph(&entries, root).unwrap();

    // All three modules should be in the graph
    assert_eq!(graph.modules.len(), 3);

    let actions_canonical = root.join("actions.ts").canonicalize().unwrap();
    let actions = graph.modules.get(&actions_canonical).unwrap();
    assert!(actions.is_server);
    assert!(!actions.is_client);

    let db_canonical = root.join("db.ts").canonicalize().unwrap();
    assert!(graph.modules.contains_key(&db_canonical));
}

#[test]
fn server_action_modules_method() {
    let dir = setup_temp_dir();
    let root = dir.path();

    fs::write(
        root.join("page.tsx"),
        "import { inc } from './actions';\nexport default function Page() { return null; }\n",
    )
    .unwrap();

    fs::write(
        root.join("actions.ts"),
        "\"use server\";\nexport async function inc(n: number) { return n + 1; }\n",
    )
    .unwrap();

    let entries = vec![root.join("page.tsx")];
    let graph = analyze_module_graph(&entries, root).unwrap();

    let sa_modules = graph.server_action_modules();
    assert_eq!(sa_modules.len(), 1);
    assert!(sa_modules[0].path.ends_with("actions.ts"));
    assert!(sa_modules[0].exports.contains(&"inc".to_string()));
}

#[test]
fn client_component_importing_use_server_module() {
    let dir = setup_temp_dir();
    let root = dir.path();

    // Server component page imports a client component
    fs::write(
        root.join("page.tsx"),
        "import Counter from './Counter';\nexport default function Page() { return null; }\n",
    )
    .unwrap();

    // Client component imports from a "use server" module
    fs::write(
        root.join("Counter.tsx"),
        "\"use client\";\nimport { increment } from './actions';\nexport default function Counter() { return null; }\n",
    )
    .unwrap();

    // "use server" module
    fs::write(
        root.join("actions.ts"),
        "\"use server\";\nexport async function increment(n: number) { return n + 1; }\n",
    )
    .unwrap();

    let entries = vec![root.join("page.tsx")];
    let graph = analyze_module_graph(&entries, root).unwrap();

    // page.tsx, Counter.tsx (client boundary), and actions.ts (server action) should all be in graph
    assert_eq!(graph.modules.len(), 3);

    let actions_canonical = root.join("actions.ts").canonicalize().unwrap();
    let actions = graph.modules.get(&actions_canonical).unwrap();
    assert!(actions.is_server);
    assert_eq!(actions.exports, vec!["increment"]);

    // server_action_modules should include actions.ts
    let sa_modules = graph.server_action_modules();
    assert_eq!(sa_modules.len(), 1);
    assert!(sa_modules[0].path.ends_with("actions.ts"));
}

#[test]
fn jsx_file_analyzed() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(
        root.join("comp.jsx"),
        "export default function Comp() { return <div>Hello</div>; }\n",
    )
    .unwrap();

    let entries = vec![root.join("comp.jsx")];
    let graph = analyze_module_graph(&entries, root).unwrap();
    assert_eq!(graph.modules.len(), 1);
    let comp = graph.modules.values().next().unwrap();
    assert!(comp.exports.contains(&"default".to_string()));
}

#[test]
fn class_export_detected() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(
        root.join("widget.ts"),
        "\"use client\";\nexport class Widget {}\n",
    )
    .unwrap();

    let entries = vec![root.join("widget.ts")];
    let graph = analyze_module_graph(&entries, root).unwrap();
    let widget = graph.modules.values().next().unwrap();
    assert!(widget.exports.contains(&"Widget".to_string()));
}

#[test]
fn reexport_specifier_detected() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("utils.ts"), "export const helper = 42;\n").unwrap();
    fs::write(root.join("index.ts"), "export { helper } from './utils';\n").unwrap();

    let entries = vec![root.join("index.ts")];
    let graph = analyze_module_graph(&entries, root).unwrap();
    let index_mod = graph
        .modules
        .values()
        .find(|m| m.path.ends_with("index.ts"))
        .unwrap();
    assert!(
        index_mod.exports.contains(&"helper".to_string()),
        "Re-export specifier should be detected, got: {:?}",
        index_mod.exports
    );
}

#[test]
fn unknown_extension_analyzed() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("mod.mjs"), "export const value = 1;\n").unwrap();

    let entries = vec![root.join("mod.mjs")];
    let graph = analyze_module_graph(&entries, root).unwrap();
    let mjs_mod = graph.modules.values().next().unwrap();
    assert!(mjs_mod.exports.contains(&"value".to_string()));
}
