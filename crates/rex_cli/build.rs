use std::path::Path;

fn strip_typescript(source: &str) -> String {
    let allocator = oxc_allocator::Allocator::default();
    let ret = oxc_parser::Parser::new(&allocator, source, oxc_span::SourceType::ts()).parse();
    oxc_codegen::Codegen::new().build(&ret.program).code
}

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let out = std::env::var("OUT_DIR").expect("OUT_DIR");
    let runtime_dir = Path::new(&manifest).join("../../runtime");

    let ts = std::fs::read_to_string(runtime_dir.join("hmr_client.ts"))
        .expect("read runtime/hmr_client.ts");
    std::fs::write(Path::new(&out).join("hmr_client.js"), strip_typescript(&ts))
        .expect("write hmr_client.js");

    println!("cargo:rerun-if-changed=../../runtime/hmr_client.ts");
}
