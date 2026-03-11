#![allow(clippy::unwrap_used)]

use rex_build::build_utils::runtime_server_dir;

#[test]
fn node_polyfill_aliases_returns_expected_specifiers() {
    let runtime_dir = runtime_server_dir().unwrap();
    let aliases = rex_build::build_utils::node_polyfill_aliases(&runtime_dir);

    let specs: Vec<&str> = aliases.iter().map(|(s, _)| s.as_str()).collect();

    // Core Node.js modules
    assert!(specs.contains(&"fs"), "missing fs alias");
    assert!(specs.contains(&"node:fs"), "missing node:fs alias");
    assert!(specs.contains(&"path"), "missing path alias");
    assert!(specs.contains(&"node:path"), "missing node:path alias");
    assert!(specs.contains(&"buffer"), "missing buffer alias");
    assert!(specs.contains(&"crypto"), "missing crypto alias");
    assert!(specs.contains(&"events"), "missing events alias");
    assert!(specs.contains(&"http"), "missing http alias");
    assert!(specs.contains(&"https"), "missing https alias");
    assert!(specs.contains(&"url"), "missing url alias");
    assert!(specs.contains(&"stream"), "missing stream alias");
    assert!(specs.contains(&"net"), "missing net alias");
    assert!(specs.contains(&"tls"), "missing tls alias");
    assert!(specs.contains(&"dns"), "missing dns alias");
    assert!(specs.contains(&"os"), "missing os alias");
    assert!(specs.contains(&"util"), "missing util alias");
    assert!(specs.contains(&"querystring"), "missing querystring alias");

    // Next.js compatibility shims
    assert!(specs.contains(&"next/link"), "missing next/link alias");
    assert!(specs.contains(&"next/image"), "missing next/image alias");
    assert!(specs.contains(&"next/router"), "missing next/router alias");
    assert!(
        specs.contains(&"next/navigation"),
        "missing next/navigation alias"
    );
    assert!(
        specs.contains(&"next/headers"),
        "missing next/headers alias"
    );

    // All aliases should have a non-empty path
    for (spec, paths) in &aliases {
        assert!(!paths.is_empty(), "alias {spec} has no paths");
        assert!(paths[0].is_some(), "alias {spec} has None path");
    }
}

#[test]
fn node_polyfill_aliases_paths_exist() {
    let runtime_dir = runtime_server_dir().unwrap();
    let aliases = rex_build::build_utils::node_polyfill_aliases(&runtime_dir);

    for (spec, paths) in &aliases {
        if let Some(Some(path)) = paths.first() {
            assert!(
                std::path::Path::new(path).exists(),
                "alias {spec} points to non-existent file: {path}"
            );
        }
    }
}
