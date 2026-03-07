use std::path::Path;

fn strip_typescript(source: &str) -> String {
    let allocator = oxc_allocator::Allocator::default();
    let source_type = oxc_span::SourceType::ts();
    let ret = oxc_parser::Parser::new(&allocator, source, source_type).parse();

    let mut program = ret.program;

    // Run semantic analysis (required by the transformer)
    let scoping = oxc_semantic::SemanticBuilder::new()
        .build(&program)
        .semantic
        .into_scoping();

    // Run the TypeScript transform to strip type annotations
    let path = Path::new("hmr_client.ts");
    let options = oxc_transformer::TransformOptions::default();
    let transformer = oxc_transformer::Transformer::new(&allocator, path, &options);
    transformer.build_with_scoping(scoping, &mut program);

    oxc_codegen::Codegen::new().build(&program).code
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
