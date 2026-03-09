use std::fs;
use std::path::Path;
use std::process::Command;

const REACT_VERSION: &str = "19.2.4";
const SCHEDULER_VERSION: &str = "0.27.0";
const TAILWIND_VERSION: &str = "4.2.1";

const PACKAGES: &[(&str, &str)] = &[
    ("react", REACT_VERSION),
    ("react-dom", REACT_VERSION),
    ("react-server-dom-webpack", REACT_VERSION),
    ("scheduler", SCHEDULER_VERSION),
];

/// Server runtime TypeScript files to compile to JavaScript at build time.
const SERVER_RUNTIME_TS: &[&str] = &[
    "ssr-runtime",
    "mcp-runtime",
    "middleware-runtime",
    "app-route-runtime",
];

/// Individual polyfill modules, concatenated in order to produce v8-polyfills.js.
/// Order matters: text-encoding before buffer, buffer before crypto.
const V8_POLYFILL_MODULES: &[&str] = &[
    "process",
    "navigator",
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
    let stamp = out_dir.join(".vendor-stamp");

    let expected = format!(
        "react@{REACT_VERSION} rsdw@{REACT_VERSION} scheduler@{SCHEDULER_VERSION} tailwind@{TAILWIND_VERSION}"
    );

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

    // Download React packages
    for &(name, version) in PACKAGES {
        download_npm_package(&node_modules, name, version);
        trim_package(&node_modules.join(name), name);
    }

    // Download Tailwind CSS and bundle compiler for V8
    download_npm_package(&node_modules, "tailwindcss", TAILWIND_VERSION);
    bundle_tailwind_compiler(out_dir);

    fs::write(&stamp, &expected).expect("failed to write stamp");
}

/// Download an npm package tarball and extract to node_modules/{name}.
fn download_npm_package(node_modules: &Path, name: &str, version: &str) {
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
}

/// Escape a string for embedding as a JS string literal (double-quoted).
fn js_string_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // U+2028 and U+2029 are line terminators in JS but not caught by is_control()
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if c.is_control() => {
                let _ = std::fmt::Write::write_fmt(&mut out, format_args!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Bundle the Tailwind CSS compiler into a self-contained JS file for V8.
///
/// Tailwind v4's npm package ships `dist/lib.js` — a fully self-contained CJS
/// module with zero `require()` calls (~270KB). We wrap it with a `module.exports`
/// shim and embed the CSS files needed by `loadStylesheet` resolution.
fn bundle_tailwind_compiler(out_dir: &Path) {
    let tw_dir = out_dir.join("node_modules/tailwindcss");

    let lib_js = fs::read_to_string(tw_dir.join("dist/lib.js"))
        .unwrap_or_else(|e| panic!("failed to read tailwindcss/dist/lib.js: {e}"));

    let css_index = fs::read_to_string(tw_dir.join("index.css"))
        .unwrap_or_else(|e| panic!("failed to read tailwindcss/index.css: {e}"));
    let css_preflight = fs::read_to_string(tw_dir.join("preflight.css"))
        .unwrap_or_else(|e| panic!("failed to read tailwindcss/preflight.css: {e}"));
    let css_theme = fs::read_to_string(tw_dir.join("theme.css"))
        .unwrap_or_else(|e| panic!("failed to read tailwindcss/theme.css: {e}"));
    let css_utilities = fs::read_to_string(tw_dir.join("utilities.css"))
        .unwrap_or_else(|e| panic!("failed to read tailwindcss/utilities.css: {e}"));

    let wrapper = format!(
        r#"(function() {{
  var module = {{ exports: {{}} }};
  var exports = module.exports;
{lib_js}
  globalThis.__tw_compile = module.exports.compile;
  globalThis.__tw_css = {{
    index: {index},
    preflight: {preflight},
    theme: {theme},
    utilities: {utilities},
  }};
}})();
"#,
        index = js_string_literal(&css_index),
        preflight = js_string_literal(&css_preflight),
        theme = js_string_literal(&css_theme),
        utilities = js_string_literal(&css_utilities),
    );

    fs::write(out_dir.join("tailwind-compiler.js"), wrapper)
        .unwrap_or_else(|e| panic!("failed to write tailwind-compiler.js: {e}"));
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
        "react-server-dom-webpack" => {
            // Remove files we don't use: plugin, node-register, node-loader, static variants
            for f in [
                "plugin.js",
                "node-register.js",
                "static.js",
                "static.browser.js",
                "static.edge.js",
                "static.node.js",
            ] {
                let _ = fs::remove_file(dir.join(f));
            }
            // Remove unused CJS bundles: plugin, node-register, static
            remove_cjs_matching(dir, &["plugin", "node-register", "static"]);
            // Remove ESM loader (Node-only)
            let _ = fs::remove_dir_all(dir.join("esm"));
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
