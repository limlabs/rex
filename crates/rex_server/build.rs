use std::path::Path;

fn strip_typescript(source: &str) -> String {
    let allocator = oxc_allocator::Allocator::default();
    let source_type = oxc_span::SourceType::ts();
    let mut ret = oxc_parser::Parser::new(&allocator, source, source_type).parse();

    // Run semantic analysis (required by the transformer for scoping info)
    let semantic = oxc_semantic::SemanticBuilder::new()
        .build(&ret.program)
        .semantic;

    // Transform with TypeScript stripping enabled (default TransformOptions includes TS)
    let options = oxc_transformer::TransformOptions::default();
    let transformer =
        oxc_transformer::Transformer::new(&allocator, Path::new("router.ts"), &options);
    transformer.build_with_scoping(semantic.into_scoping(), &mut ret.program);

    // Codegen now produces clean JavaScript (type annotations removed from AST)
    oxc_codegen::Codegen::new().build(&ret.program).code
}

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let out = std::env::var("OUT_DIR").expect("OUT_DIR");
    let runtime_dir = Path::new(&manifest).join("../../runtime");

    let ts = std::fs::read_to_string(runtime_dir.join("client/router.ts"))
        .expect("read runtime/client/router.ts");
    std::fs::write(Path::new(&out).join("router.js"), strip_typescript(&ts))
        .expect("write router.js");

    println!("cargo:rerun-if-changed=../../runtime/client/router.ts");
}
