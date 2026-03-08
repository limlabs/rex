//! Integration tests for the MDX-to-JSX compiler.
#![allow(clippy::unwrap_used)]

use oxc_allocator::Allocator;
use oxc_ast::ast::Statement;
use oxc_span::SourceType;
use rex_mdx::{compile_mdx_with_options, MdxOptions};

/// Convenience wrapper for tests using default options.
fn compile_mdx(source: &str) -> anyhow::Result<String> {
    compile_mdx_with_options(source, &MdxOptions::default())
}

#[test]
fn compile_basic_mdx() {
    let source = "# Hello World\n\nSome **bold** text.\n";
    let result = compile_mdx(source).unwrap();
    assert!(result.contains("createElement(_components.h1"));
    assert!(result.contains("'Hello World'"));
    assert!(result.contains("createElement(_components.strong"));
    assert!(result.contains("'bold'"));
    assert!(result.contains("export default MDXContent"));
}

#[test]
fn compile_mdx_with_imports() {
    let source = "import Foo from './Foo'\n\n# Hello\n\n<Foo bar=\"baz\" />\n";
    let result = compile_mdx(source).unwrap();
    assert!(result.contains("import Foo from './Foo'"));
    assert!(result.contains("createElement(Foo"));
    assert!(result.contains("bar: 'baz'"));
    assert!(result.contains("export default MDXContent"));
}

#[test]
fn compile_mdx_with_export() {
    let source = "export const meta = { title: 'Test' }\n\n# Test Page\n";
    let result = compile_mdx(source).unwrap();
    assert!(result.contains("export const meta"));
    assert!(result.contains("createElement(_components.h1"));
}

#[test]
fn compile_mdx_lists() {
    let source = "- item 1\n- item 2\n- item 3\n";
    let result = compile_mdx(source).unwrap();
    assert!(result.contains("createElement(_components.ul"));
    assert!(result.contains("createElement(_components.li"));
    assert!(result.contains("'item 1'"));
}

#[test]
fn compile_mdx_code_block() {
    let source = "```js\nconsole.log('hello')\n```\n";
    let result = compile_mdx(source).unwrap();
    assert!(result.contains("createElement(_components.pre"));
    assert!(result.contains("createElement(_components.code"));
    assert!(result.contains("language-js"));
}

#[test]
fn compile_mdx_link() {
    let source = "Visit [Example](https://example.com) for more.\n";
    let result = compile_mdx(source).unwrap();
    assert!(result.contains("createElement(_components.a"));
    assert!(result.contains("href: 'https://example.com'"));
    assert!(result.contains("'Example'"));
}

#[test]
fn compile_mdx_jsx_component() {
    let source = "import Alert from './Alert'\n\n# Warning\n\n<Alert type=\"danger\">\n  Be careful!\n</Alert>\n";
    let result = compile_mdx(source).unwrap();
    assert!(result.contains("import Alert from './Alert'"));
    assert!(result.contains("createElement(Alert"));
    assert!(result.contains("type: 'danger'"));
}

#[test]
fn compile_mdx_expression() {
    let source = "The answer is {40 + 2}.\n";
    let result = compile_mdx(source).unwrap();
    assert!(result.contains("40 + 2"));
}

#[test]
fn compile_mdx_image() {
    let source = "![Alt text](image.png)\n";
    let result = compile_mdx(source).unwrap();
    assert!(result.contains("createElement(_components.img"));
    assert!(result.contains("src: 'image.png'"));
    assert!(result.contains("alt: 'Alt text'"));
}

#[test]
fn compile_mdx_blockquote() {
    let source = "> This is a quote\n";
    let result = compile_mdx(source).unwrap();
    assert!(result.contains("createElement(_components.blockquote"));
}

#[test]
fn compile_mdx_thematic_break() {
    let source = "Before\n\n---\n\nAfter\n";
    let result = compile_mdx(source).unwrap();
    assert!(result.contains("createElement(_components.hr"));
}

#[test]
fn compile_empty_mdx() {
    let source = "";
    let result = compile_mdx(source).unwrap();
    assert!(result.contains("export default MDXContent"));
}

#[test]
fn compile_mdx_table() {
    let source = "| Name | Age |\n| --- | --- |\n| Alice | 30 |\n| Bob | 25 |\n";
    let result = compile_mdx(source).unwrap();
    assert!(result.contains("createElement(_components.table"));
    assert!(result.contains("createElement(_components.thead"));
    assert!(result.contains("createElement(_components.tbody"));
    assert!(result.contains("createElement(_components.th"));
    assert!(result.contains("createElement(_components.td"));
    assert!(result.contains("'Alice'"));
}

#[test]
fn compile_mdx_inline_code() {
    let source = "Use `console.log()` to debug.\n";
    let result = compile_mdx(source).unwrap();
    assert!(result.contains("createElement(_components.code"));
    assert!(result.contains("console.log()"));
}

#[test]
fn compiled_mdx_is_valid_jsx() {
    let source = "import Foo from './Foo'\n\n# Hello\n\nSome **bold** text and a [link](/about).\n\n- item 1\n- item 2\n\n<Foo bar=\"baz\" />\n";
    let result = compile_mdx(source).unwrap();

    let allocator = Allocator::default();
    let ret = oxc_parser::Parser::new(&allocator, &result, SourceType::jsx()).parse();
    assert!(
        ret.errors.is_empty(),
        "Compiled MDX should be valid JSX. Errors: {:?}\nCompiled:\n{result}",
        ret.errors
    );
}

#[test]
fn compiled_mdx_has_default_export() {
    let source = "# Title\n\nContent.\n";
    let result = compile_mdx(source).unwrap();

    let allocator = Allocator::default();
    let ret = oxc_parser::Parser::new(&allocator, &result, SourceType::jsx()).parse();
    let has_default = ret
        .program
        .body
        .iter()
        .any(|stmt| matches!(stmt, Statement::ExportDefaultDeclaration(_)));
    assert!(has_default, "Compiled MDX must have a default export");
}

#[test]
fn compile_mdx_ordered_list_with_start() {
    let source = "5. fifth\n6. sixth\n";
    let result = compile_mdx(source).unwrap();
    assert!(result.contains("createElement(_components.ol"));
    assert!(result.contains("start: 5"));
    let allocator = Allocator::default();
    let ret = oxc_parser::Parser::new(&allocator, &result, SourceType::jsx()).parse();
    assert!(
        ret.errors.is_empty(),
        "Ordered list with start should produce valid JS. Errors: {:?}\nCompiled:\n{result}",
        ret.errors
    );
}

#[test]
fn compile_mdx_user_default_export_not_duplicated() {
    let source = "export default function Layout() {}\n\n# Hello\n";
    let result = compile_mdx(source).unwrap();
    assert!(
        result.contains("MDXWrapper") || result.contains("_userLayout"),
        "Should wrap with user layout. Got:\n{result}"
    );
    let allocator = Allocator::default();
    let ret = oxc_parser::Parser::new(&allocator, &result, SourceType::jsx()).parse();
    let default_count = ret
        .program
        .body
        .iter()
        .filter(|stmt| matches!(stmt, Statement::ExportDefaultDeclaration(_)))
        .count();
    assert!(
        default_count <= 1,
        "Should have at most one default export, got {default_count}.\nCompiled:\n{result}"
    );
}

#[test]
fn compile_mdx_component_overrides() {
    let source = "# Hello\n\nSome text.\n";
    let result = compile_mdx(source).unwrap();
    assert!(
        result.contains("h1: 'h1'"),
        "_components should have h1 default. Got:\n{result}"
    );
    assert!(
        result.contains("p: 'p'"),
        "_components should have p default. Got:\n{result}"
    );
    assert!(result.contains("_components.h1"));
    assert!(result.contains("_components.p"));
}

#[test]
fn compile_mdx_with_mdx_components_file() {
    let source = "# Hello\n";
    let options = MdxOptions {
        mdx_components_path: Some("./mdx-components".to_string()),
    };
    let result = compile_mdx_with_options(source, &options).unwrap();
    assert!(
        result.contains("import { useMDXComponents as _provideComponents }"),
        "Should import useMDXComponents. Got:\n{result}"
    );
    assert!(
        result.contains("_provideComponents()"),
        "Should call _provideComponents. Got:\n{result}"
    );
}

#[test]
fn compile_mdx_user_default_export_wraps_content() {
    let source =
        "export default ({children}) => <div className=\"layout\">{children}</div>\n\n# Hello\n";
    let result = compile_mdx(source).unwrap();
    assert!(
        result.contains("_userLayout"),
        "Should capture user layout. Got:\n{result}"
    );
    assert!(
        result.contains("MDXWrapper"),
        "Should have wrapper component. Got:\n{result}"
    );
    assert!(
        result.contains("createElement(_userLayout"),
        "Should wrap with user layout. Got:\n{result}"
    );
    let allocator = Allocator::default();
    let ret = oxc_parser::Parser::new(&allocator, &result, SourceType::jsx()).parse();
    assert!(
        ret.errors.is_empty(),
        "Layout wrapper should produce valid JS. Errors: {:?}\nCompiled:\n{result}",
        ret.errors
    );
}

#[test]
fn compile_mdx_frontmatter() {
    let source = "---\ntitle: Hello World\ndraft: true\ncount: 42\n---\n\n# Hello\n";
    let result = compile_mdx(source).unwrap();
    assert!(
        result.contains("export const frontmatter"),
        "Should export frontmatter. Got:\n{result}"
    );
    assert!(
        result.contains("title: 'Hello World'"),
        "Should have title. Got:\n{result}"
    );
    assert!(
        result.contains("draft: true"),
        "Should have boolean. Got:\n{result}"
    );
    assert!(
        result.contains("count: 42"),
        "Should have number. Got:\n{result}"
    );
}

#[test]
fn compile_mdx_frontmatter_valid_js() {
    let source = "---\ntitle: My Page\ntags: [react, mdx]\n---\n\n# Content\n";
    let result = compile_mdx(source).unwrap();

    let allocator = Allocator::default();
    let ret = oxc_parser::Parser::new(&allocator, &result, SourceType::jsx()).parse();
    assert!(
        ret.errors.is_empty(),
        "Frontmatter should produce valid JS. Errors: {:?}\nCompiled:\n{result}",
        ret.errors
    );
}
