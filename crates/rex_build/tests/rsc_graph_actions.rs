//! Integration tests for RSC module graph — server actions and dynamic functions.

#![allow(clippy::unwrap_used)]

use rex_build::rsc_graph::{analyze_module_graph, ModuleGraph};
use std::fs;

fn setup_temp_dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
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
