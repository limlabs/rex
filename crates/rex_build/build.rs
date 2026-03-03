use std::fs;
use std::path::Path;
use std::process::Command;

const REACT_VERSION: &str = "19.2.4";
const SCHEDULER_VERSION: &str = "0.27.0";

const PACKAGES: &[(&str, &str)] = &[
    ("react", REACT_VERSION),
    ("react-dom", REACT_VERSION),
    ("scheduler", SCHEDULER_VERSION),
];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    let out_dir = Path::new(&out_dir);
    let node_modules = out_dir.join("node_modules");
    let stamp = out_dir.join(".react-version");

    let expected = format!("react@{REACT_VERSION} scheduler@{SCHEDULER_VERSION}");

    // Skip download on incremental rebuilds when stamp matches
    if stamp.exists() {
        if let Ok(existing) = fs::read_to_string(&stamp) {
            if existing.trim() == expected {
                return;
            }
        }
    }

    // Clean slate
    if node_modules.exists() {
        fs::remove_dir_all(&node_modules).expect("failed to remove old node_modules");
    }

    for &(name, version) in PACKAGES {
        let pkg_dir = node_modules.join(name);
        fs::create_dir_all(&pkg_dir).expect("failed to create package dir");

        let url = format!("https://registry.npmjs.org/{name}/-/{name}-{version}.tgz");

        let status = Command::new("sh")
            .arg("-c")
            .arg(format!(
                "curl -sfL '{url}' | tar xz --strip-components=1 -C '{}'",
                pkg_dir.display()
            ))
            .status()
            .unwrap_or_else(|e| panic!("failed to spawn curl|tar for {name}: {e}"));

        assert!(
            status.success(),
            "failed to download {name}@{version} from {url}"
        );

        trim_package(&pkg_dir, name);
    }

    fs::write(&stamp, &expected).expect("failed to write stamp");
}

/// Remove files we don't need in the embedded binary.
fn trim_package(dir: &Path, name: &str) {
    // Common: docs / license
    for f in [
        "LICENSE",
        "LICENSE.md",
        "README.md",
        "README",
        "CHANGELOG.md",
    ] {
        let _ = fs::remove_file(dir.join(f));
    }

    match name {
        "react-dom" => {
            // Entry-point files we don't ship
            for f in [
                "test-utils.js",
                "profiling.js",
                "profiling.react-server.js",
                "static.js",
                "static.browser.js",
                "static.edge.js",
                "static.node.js",
                "server.bun.js",
                "server.edge.js",
            ] {
                let _ = fs::remove_file(dir.join(f));
            }

            // CJS bundles we don't need
            remove_cjs_matching(
                dir,
                &["profiling", "test-utils", ".edge.", ".bun.", "static"],
            );
        }
        "scheduler" => {
            for f in ["unstable_mock.js", "unstable_post_task.js"] {
                let _ = fs::remove_file(dir.join(f));
            }
            remove_cjs_matching(dir, &["unstable", "native"]);
        }
        _ => {} // react is already lean
    }
}

fn remove_cjs_matching(dir: &Path, patterns: &[&str]) {
    let cjs = dir.join("cjs");
    let entries = match fs::read_dir(&cjs) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let fname = entry.file_name();
        let fname = fname.to_string_lossy();
        if patterns.iter().any(|p| fname.contains(p)) {
            let _ = fs::remove_file(entry.path());
        }
    }
}
