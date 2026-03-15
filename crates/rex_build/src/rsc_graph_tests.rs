use super::*;
use std::fs;

fn setup_temp_dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn resolve_import_with_extension_guessing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    std::fs::write(root.join("Foo.tsx"), "export default function Foo() {}").unwrap();

    let from = root.join("page.tsx");
    let resolved = resolve_import(&from, "./Foo", root);
    assert!(resolved.is_some());
    assert!(resolved.unwrap().ends_with("Foo.tsx"));
}

#[test]
fn resolve_import_ignores_bare_specifiers() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let from = root.join("page.tsx");
    assert!(resolve_import(&from, "react", root).is_none());
    assert!(resolve_import(&from, "next/link", root).is_none());
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

#[test]
fn function_level_use_server_declaration() {
    let dir = setup_temp_dir();
    let root = dir.path();

    fs::write(
        root.join("page.tsx"),
        r#"
export async function submitForm() {
    "use server";
    return { ok: true };
}

export default function Page() { return null; }
"#,
    )
    .unwrap();

    let entries = vec![root.join("page.tsx")];
    let graph = analyze_module_graph(&entries, root).unwrap();

    let canonical = root.join("page.tsx").canonicalize().unwrap();
    let info = graph.modules.get(&canonical).unwrap();
    assert!(!info.is_server); // no module-level directive
    assert!(info.server_functions.contains(&"submitForm".to_string()));
    assert!(!info.server_functions.contains(&"default".to_string()));
}

#[test]
fn function_level_use_server_arrow() {
    let dir = setup_temp_dir();
    let root = dir.path();

    fs::write(
        root.join("actions.ts"),
        r#"
export const increment = async (n: number) => {
    "use server";
    return n + 1;
};

export const helper = (x: number) => x * 2;
"#,
    )
    .unwrap();

    let entries = vec![root.join("actions.ts")];
    let graph = analyze_module_graph(&entries, root).unwrap();

    let canonical = root.join("actions.ts").canonicalize().unwrap();
    let info = graph.modules.get(&canonical).unwrap();
    assert!(!info.is_server);
    assert!(info.server_functions.contains(&"increment".to_string()));
    assert!(!info.server_functions.contains(&"helper".to_string()));
}

#[test]
fn inline_server_action_modules_method() {
    let dir = setup_temp_dir();
    let root = dir.path();

    fs::write(
        root.join("page.tsx"),
        r#"
export async function submit() {
    "use server";
    return 1;
}
export default function Page() { return null; }
"#,
    )
    .unwrap();

    let entries = vec![root.join("page.tsx")];
    let graph = analyze_module_graph(&entries, root).unwrap();

    let inline = graph.inline_server_action_modules();
    assert_eq!(inline.len(), 1);
    assert!(inline[0].server_functions.contains(&"submit".to_string()));

    // module-level server action modules should be empty
    assert!(graph.server_action_modules().is_empty());
}

#[test]
fn module_level_use_server_overrides_function_level() {
    let dir = setup_temp_dir();
    let root = dir.path();

    // Module-level "use server" — all exports are server actions,
    // function-level detection should be skipped
    fs::write(
        root.join("actions.ts"),
        r#"
"use server";
export async function inc() { return 1; }
export async function dec() { return -1; }
"#,
    )
    .unwrap();

    let entries = vec![root.join("actions.ts")];
    let graph = analyze_module_graph(&entries, root).unwrap();

    let canonical = root.join("actions.ts").canonicalize().unwrap();
    let info = graph.modules.get(&canonical).unwrap();
    assert!(info.is_server);
    assert!(info.server_functions.is_empty()); // not populated when module-level
}

#[test]
fn detects_dynamic_function_cookies_import() {
    let dir = setup_temp_dir();
    let root = dir.path();

    fs::write(
        root.join("page.tsx"),
        r#"
import { cookies } from 'rex/actions';
export default function Page() { return null; }
"#,
    )
    .unwrap();

    let entries = vec![root.join("page.tsx")];
    let graph = analyze_module_graph(&entries, root).unwrap();

    let canonical = root.join("page.tsx").canonicalize().unwrap();
    let info = graph.modules.get(&canonical).unwrap();
    assert!(info.uses_dynamic_functions);
}

#[test]
fn detects_dynamic_function_headers_import() {
    let dir = setup_temp_dir();
    let root = dir.path();

    fs::write(
        root.join("page.tsx"),
        r#"
import { headers } from 'rex/actions';
export default function Page() { return null; }
"#,
    )
    .unwrap();

    let entries = vec![root.join("page.tsx")];
    let graph = analyze_module_graph(&entries, root).unwrap();

    let canonical = root.join("page.tsx").canonicalize().unwrap();
    let info = graph.modules.get(&canonical).unwrap();
    assert!(info.uses_dynamic_functions);
}

#[test]
fn no_dynamic_functions_for_static_page() {
    let dir = setup_temp_dir();
    let root = dir.path();

    fs::write(
        root.join("page.tsx"),
        r#"
export default function Page() { return <div>Hello</div>; }
"#,
    )
    .unwrap();

    let entries = vec![root.join("page.tsx")];
    let graph = analyze_module_graph(&entries, root).unwrap();

    let canonical = root.join("page.tsx").canonicalize().unwrap();
    let info = graph.modules.get(&canonical).unwrap();
    assert!(!info.uses_dynamic_functions);
}

#[test]
fn has_dynamic_functions_traverses_imports() {
    let dir = setup_temp_dir();
    let root = dir.path();

    // Page imports a layout that uses cookies
    fs::write(
        root.join("page.tsx"),
        r#"
import Layout from './layout';
export default function Page() { return null; }
"#,
    )
    .unwrap();

    fs::write(
        root.join("layout.tsx"),
        r#"
import { cookies } from 'rex/actions';
export default function Layout({ children }) { return <div>{children}</div>; }
"#,
    )
    .unwrap();

    let entries = vec![root.join("page.tsx")];
    let graph = analyze_module_graph(&entries, root).unwrap();

    let page_canonical = root.join("page.tsx").canonicalize().unwrap();
    let layout_canonical = root.join("layout.tsx").canonicalize().unwrap();

    // Page itself doesn't use dynamic functions
    assert!(
        !graph
            .modules
            .get(&page_canonical)
            .unwrap()
            .uses_dynamic_functions
    );
    // Layout does
    assert!(
        graph
            .modules
            .get(&layout_canonical)
            .unwrap()
            .uses_dynamic_functions
    );

    // has_dynamic_functions should detect it through the dependency tree
    assert!(graph.has_dynamic_functions(&[page_canonical]));
}

#[test]
fn has_dynamic_functions_stops_at_client_boundary() {
    let dir = setup_temp_dir();
    let root = dir.path();

    // Server page imports a client component
    fs::write(
        root.join("page.tsx"),
        r#"
import Counter from './Counter';
export default function Page() { return null; }
"#,
    )
    .unwrap();

    // Client component (would use cookies, but shouldn't affect server detection)
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

    let page_canonical = root.join("page.tsx").canonicalize().unwrap();
    assert!(!graph.has_dynamic_functions(&[page_canonical]));
}

#[test]
fn has_dynamic_functions_empty_entries() {
    let graph = ModuleGraph::default();
    assert!(!graph.has_dynamic_functions(&[]));
}

#[test]
fn other_rex_actions_imports_not_dynamic() {
    let dir = setup_temp_dir();
    let root = dir.path();

    // Import redirect from rex/actions — not a dynamic function
    fs::write(
        root.join("page.tsx"),
        r#"
import { redirect } from 'rex/actions';
export default function Page() { return null; }
"#,
    )
    .unwrap();

    let entries = vec![root.join("page.tsx")];
    let graph = analyze_module_graph(&entries, root).unwrap();

    let canonical = root.join("page.tsx").canonicalize().unwrap();
    let info = graph.modules.get(&canonical).unwrap();
    assert!(!info.uses_dynamic_functions);
}

#[test]
fn rex_link_resolved_via_runtime_fallback() {
    let dir = setup_temp_dir();
    let root = dir.path();

    // Server page imports rex/link — no @limlabs/rex in node_modules,
    // so the graph must fall back to the runtime/client/ directory.
    fs::write(
        root.join("page.tsx"),
        r#"
import Link from 'rex/link';
export default function Page() { return <Link href="/">Home</Link>; }
"#,
    )
    .unwrap();

    let entries = vec![root.join("page.tsx")];
    let graph = analyze_module_graph(&entries, root).unwrap();

    // The Link module should be detected as a client component
    let client_modules = graph.client_boundary_modules();
    assert!(
        !client_modules.is_empty(),
        "rex/link should be detected as a client boundary module via runtime fallback"
    );
    let link_mod = client_modules
        .iter()
        .find(|m| m.path.to_string_lossy().contains("link"))
        .expect("should find link module in client boundaries");
    assert!(link_mod.is_client);
}

#[test]
fn mdx_import_does_not_crash() {
    let dir = setup_temp_dir();
    let root = dir.path();

    // Page that imports an MDX file
    fs::write(
        root.join("page.tsx"),
        r#"
import Content from './content.mdx';
export default function Page() { return <Content />; }
"#,
    )
    .unwrap();

    // MDX file with markdown content (not valid JS)
    fs::write(
        root.join("content.mdx"),
        "# Hello World\n\nThis is **markdown** content.\n",
    )
    .unwrap();

    let entries = vec![root.join("page.tsx")];
    let graph = analyze_module_graph(&entries, root).unwrap();

    // Both modules should be in the graph
    assert_eq!(graph.modules.len(), 2);

    // MDX module should have sensible defaults
    let mdx_canonical = root.join("content.mdx").canonicalize().unwrap();
    let mdx_info = graph.modules.get(&mdx_canonical).unwrap();
    assert!(!mdx_info.is_client);
    assert!(!mdx_info.is_server);
    assert!(mdx_info.exports.contains(&"default".to_string()));
    assert!(mdx_info.imports.is_empty());
}
