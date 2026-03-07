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

/// Server runtime TypeScript files to compile to JavaScript at build time.
const SERVER_RUNTIME_TS: &[&str] = &["ssr-runtime", "mcp-runtime", "middleware-runtime"];

/// Individual polyfill modules, concatenated in order to produce v8-polyfills.js.
/// Order matters: text-encoding before buffer, buffer before crypto.
const V8_POLYFILL_MODULES: &[&str] = &[
    "process",
    "timers",
    "message-channel",
    "text-encoding",
    "performance",
    "url",
    "streams",
    "abort",
    "buffer",
    "crypto",
    "formdata",
];

fn strip_typescript(source: &str) -> String {
    let allocator = oxc_allocator::Allocator::default();
    let source_type = oxc_span::SourceType::ts();
    let mut ret = oxc_parser::Parser::new(&allocator, source, source_type).parse();

    let semantic = oxc_semantic::SemanticBuilder::new()
        .build(&ret.program)
        .semantic;

    let options = oxc_transformer::TransformOptions::default();
    let transformer =
        oxc_transformer::Transformer::new(&allocator, Path::new("input.ts"), &options);
    transformer.build_with_scoping(semantic.into_scoping(), &mut ret.program);

    oxc_codegen::Codegen::new().build(&ret.program).code
}

fn compile_server_runtime(manifest_dir: &Path, out_dir: &Path) {
    let runtime_dir = manifest_dir.join("../../runtime/server");

    for name in SERVER_RUNTIME_TS {
        let ts_path = runtime_dir.join(format!("{name}.ts"));
        let ts_source = fs::read_to_string(&ts_path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", ts_path.display()));
        let js = strip_typescript(&ts_source);
        fs::write(out_dir.join(format!("{name}.js")), js)
            .unwrap_or_else(|e| panic!("failed to write {name}.js: {e}"));

        println!("cargo:rerun-if-changed=../../runtime/server/{name}.ts");
    }

    // Concatenate polyfill modules into v8-polyfills.js
    let polyfills_dir = runtime_dir.join("polyfills");
    let mut combined = String::new();
    for name in V8_POLYFILL_MODULES {
        let ts_path = polyfills_dir.join(format!("{name}.ts"));
        let ts_source = fs::read_to_string(&ts_path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", ts_path.display()));
        let js = strip_typescript(&ts_source);
        combined.push_str(&js);
        combined.push('\n');
        println!("cargo:rerun-if-changed=../../runtime/server/polyfills/{name}.ts");
    }
    fs::write(out_dir.join("v8-polyfills.js"), combined)
        .unwrap_or_else(|e| panic!("failed to write v8-polyfills.js: {e}"));
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    let out_dir = Path::new(&out_dir);

    // Compile server runtime TS → JS
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    compile_server_runtime(Path::new(&manifest_dir), out_dir);
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
