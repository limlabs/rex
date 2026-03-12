use super::*;
use std::path::Path;

fn default_options() -> oxc_formatter::FormatOptions {
    oxc_formatter::FormatOptions {
        quote_style: oxc_formatter::QuoteStyle::Single,
        ..Default::default()
    }
}

#[test]
fn test_format_source_single_quotes() {
    let input = "const x = \"hello\";\n";
    let opts = default_options();
    let result = format_source(input, Path::new("test.ts"), &opts).unwrap();
    assert!(
        result.contains("'hello'"),
        "expected single quotes, got: {result}"
    );
}

#[test]
fn test_format_source_semicolons() {
    let input = "const x = 1\n";
    let opts = default_options();
    let result = format_source(input, Path::new("test.ts"), &opts).unwrap();
    assert!(
        result.contains("const x = 1;"),
        "expected semicolons, got: {result}"
    );
}

#[test]
fn test_format_source_tsx() {
    let input = "export default function App() { return <div>hi</div>; }\n";
    let opts = default_options();
    let result = format_source(input, Path::new("test.tsx"), &opts).unwrap();
    assert!(result.contains("<div>"), "expected JSX preserved: {result}");
}

#[test]
fn test_format_source_idempotent() {
    let input = "const x = 'hello';\n";
    let opts = default_options();
    let first = format_source(input, Path::new("test.ts"), &opts).unwrap();
    let second = format_source(&first, Path::new("test.ts"), &opts).unwrap();
    assert_eq!(first, second, "formatting should be idempotent");
}

#[test]
fn test_format_source_parse_error() {
    let input = "const = ;;\n";
    let opts = default_options();
    let result = format_source(input, Path::new("test.ts"), &opts);
    assert!(result.is_err(), "should fail on invalid syntax");
}

#[test]
fn test_discover_source_files_finds_pages() {
    let tmp = tempfile::tempdir().unwrap();
    let pages = tmp.path().join("pages");
    std::fs::create_dir_all(&pages).unwrap();
    std::fs::write(pages.join("index.tsx"), "export default function() {}").unwrap();
    std::fs::write(pages.join("readme.md"), "# hello").unwrap();

    let files = discover_source_files(tmp.path());
    assert_eq!(files.len(), 1);
    assert!(files[0].ends_with("index.tsx"));
}

#[test]
fn test_discover_source_files_skips_node_modules() {
    let tmp = tempfile::tempdir().unwrap();
    let nm = tmp.path().join("pages/node_modules/foo");
    std::fs::create_dir_all(&nm).unwrap();
    std::fs::write(nm.join("bar.ts"), "const x = 1").unwrap();

    let files = discover_source_files(tmp.path());
    assert!(files.is_empty(), "should skip node_modules");
}

#[test]
fn test_discover_source_files_root_configs() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("next.config.js"), "module.exports = {}").unwrap();
    std::fs::write(tmp.path().join("package.json"), "{}").unwrap();

    let files = discover_source_files(tmp.path());
    assert_eq!(files.len(), 1);
    assert!(files[0].ends_with("next.config.js"));
}

#[test]
fn test_walk_dir_extensions() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("a.ts"), "").unwrap();
    std::fs::write(tmp.path().join("b.tsx"), "").unwrap();
    std::fs::write(tmp.path().join("c.css"), "").unwrap();
    std::fs::write(tmp.path().join("d.js"), "").unwrap();

    let extensions: &[&str] = &["ts", "tsx", "js", "jsx"];
    let skip_dirs: &[&str] = &["node_modules"];
    let mut files = Vec::new();
    walk_dir(tmp.path(), extensions, skip_dirs, &mut files);

    assert_eq!(files.len(), 3, "should find .ts, .tsx, .js but not .css");
}

#[test]
fn test_walk_dir_recursive() {
    let tmp = tempfile::tempdir().unwrap();
    let nested = tmp.path().join("a/b/c");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(nested.join("deep.ts"), "").unwrap();

    let extensions: &[&str] = &["ts"];
    let skip_dirs: &[&str] = &[];
    let mut files = Vec::new();
    walk_dir(tmp.path(), extensions, skip_dirs, &mut files);

    assert_eq!(files.len(), 1);
    assert!(files[0].ends_with("deep.ts"));
}

#[test]
fn test_cmd_fmt_write_mode() {
    let tmp = tempfile::tempdir().unwrap();
    let pages = tmp.path().join("pages");
    std::fs::create_dir_all(&pages).unwrap();
    std::fs::write(pages.join("index.ts"), "const x = \"hello\"\n").unwrap();

    cmd_fmt(tmp.path().to_path_buf(), false).unwrap();

    let content = std::fs::read_to_string(pages.join("index.ts")).unwrap();
    assert!(
        content.contains("'hello'"),
        "should have formatted to single quotes: {content}"
    );
    assert!(
        content.contains(';'),
        "should have added semicolons: {content}"
    );
}

#[test]
fn test_cmd_fmt_check_mode_passes_when_formatted() {
    let tmp = tempfile::tempdir().unwrap();
    let pages = tmp.path().join("pages");
    std::fs::create_dir_all(&pages).unwrap();

    let opts = default_options();
    let formatted = format_source("const x = \"hello\";\n", Path::new("t.ts"), &opts).unwrap();
    std::fs::write(pages.join("index.ts"), &formatted).unwrap();

    cmd_fmt(tmp.path().to_path_buf(), true).unwrap();
}

#[test]
fn test_cmd_fmt_empty_dir() {
    let tmp = tempfile::tempdir().unwrap();
    cmd_fmt(tmp.path().to_path_buf(), false).unwrap();
}

#[test]
fn test_cmd_fmt_skips_parse_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let pages = tmp.path().join("pages");
    std::fs::create_dir_all(&pages).unwrap();
    std::fs::write(pages.join("broken.ts"), "const = ;;\n").unwrap();
    std::fs::write(pages.join("good.ts"), "const x = \"hello\"\n").unwrap();

    cmd_fmt(tmp.path().to_path_buf(), false).unwrap();

    let broken = std::fs::read_to_string(pages.join("broken.ts")).unwrap();
    assert_eq!(broken, "const = ;;\n");

    let good = std::fs::read_to_string(pages.join("good.ts")).unwrap();
    assert!(good.contains("'hello'"));
}

#[test]
fn test_discover_multiple_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("pages")).unwrap();
    std::fs::create_dir_all(tmp.path().join("components")).unwrap();
    std::fs::create_dir_all(tmp.path().join("lib")).unwrap();
    std::fs::write(tmp.path().join("pages/index.tsx"), "").unwrap();
    std::fs::write(tmp.path().join("components/btn.tsx"), "").unwrap();
    std::fs::write(tmp.path().join("lib/utils.ts"), "").unwrap();

    let files = discover_source_files(tmp.path());
    assert_eq!(
        files.len(),
        3,
        "should find files in pages, components, lib"
    );
}

// --- Prettier config tests ---

#[test]
fn test_load_format_options_prettierrc() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join(".prettierrc"),
        r#"{ "singleQuote": false }"#,
    )
    .unwrap();

    let opts = load_format_options(tmp.path());
    assert_eq!(opts.quote_style, oxc_formatter::QuoteStyle::Double);
}

#[test]
fn test_load_format_options_prettierrc_json() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join(".prettierrc.json"), r#"{ "tabWidth": 4 }"#).unwrap();

    let opts = load_format_options(tmp.path());
    assert_eq!(opts.indent_width.value(), 4);
}

#[test]
fn test_load_format_options_package_json() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("package.json"),
        r#"{ "name": "test", "prettier": { "singleQuote": true, "tabWidth": 4 } }"#,
    )
    .unwrap();

    let opts = load_format_options(tmp.path());
    assert_eq!(opts.quote_style, oxc_formatter::QuoteStyle::Single);
    assert_eq!(opts.indent_width.value(), 4);
}

#[test]
fn test_load_format_options_precedence() {
    let tmp = tempfile::tempdir().unwrap();
    // .prettierrc should win over package.json
    std::fs::write(tmp.path().join(".prettierrc"), r#"{ "singleQuote": true }"#).unwrap();
    std::fs::write(
        tmp.path().join("package.json"),
        r#"{ "prettier": { "singleQuote": false } }"#,
    )
    .unwrap();

    let opts = load_format_options(tmp.path());
    assert_eq!(opts.quote_style, oxc_formatter::QuoteStyle::Single);
}

#[test]
fn test_load_format_options_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    // No config files at all
    let opts = load_format_options(tmp.path());
    assert_eq!(
        opts.quote_style,
        oxc_formatter::QuoteStyle::Single,
        "should default to single quotes"
    );
    assert_eq!(
        opts.indent_width.value(),
        2,
        "should default to 2-space indent"
    );
}

#[test]
fn test_load_format_options_all_fields() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join(".prettierrc"),
        r#"{
                "singleQuote": true,
                "jsxSingleQuote": true,
                "tabWidth": 4,
                "useTabs": true,
                "printWidth": 120,
                "semi": false,
                "trailingComma": "none",
                "bracketSpacing": false,
                "bracketSameLine": true,
                "arrowParens": "avoid",
                "endOfLine": "crlf"
            }"#,
    )
    .unwrap();

    let opts = load_format_options(tmp.path());
    assert_eq!(opts.quote_style, oxc_formatter::QuoteStyle::Single);
    assert_eq!(opts.jsx_quote_style, oxc_formatter::QuoteStyle::Single);
    assert_eq!(opts.indent_width.value(), 4);
    assert_eq!(opts.indent_style, oxc_formatter::IndentStyle::Tab);
    assert_eq!(opts.line_width.value(), 120);
    assert_eq!(opts.semicolons, oxc_formatter::Semicolons::AsNeeded);
    assert_eq!(opts.trailing_commas, oxc_formatter::TrailingCommas::None);
    assert!(!opts.bracket_spacing.value());
    assert!(opts.bracket_same_line.value());
    assert_eq!(
        opts.arrow_parentheses,
        oxc_formatter::ArrowParentheses::AsNeeded
    );
    assert_eq!(opts.line_ending, oxc_formatter::LineEnding::Crlf);
}

#[test]
fn test_load_ignore_patterns() {
    let tmp = tempfile::tempdir().unwrap();
    let pages = tmp.path().join("pages");
    let api = pages.join("api");
    std::fs::create_dir_all(&api).unwrap();
    std::fs::write(pages.join("index.ts"), "const x = 1").unwrap();
    std::fs::write(api.join("hello.ts"), "const y = 2").unwrap();

    std::fs::write(
        tmp.path().join(".prettierignore"),
        "# comment\npages/api/\n",
    )
    .unwrap();

    let patterns = load_ignore_patterns(tmp.path());
    assert_eq!(patterns.len(), 1);
    assert_eq!(patterns[0], "pages/api/");

    let files = discover_source_files(tmp.path());
    let filtered: Vec<_> = files
        .into_iter()
        .filter(|f| !is_ignored(f, tmp.path(), &patterns))
        .collect();
    assert_eq!(filtered.len(), 1, "should filter out api files");
    assert!(filtered[0].ends_with("index.ts"));
}

#[test]
fn test_cmd_fmt_check_mode_fails_when_unformatted() {
    let tmp = tempfile::tempdir().unwrap();
    let pages = tmp.path().join("pages");
    std::fs::create_dir_all(&pages).unwrap();
    std::fs::write(pages.join("index.ts"), "const x = \"hello\"\n").unwrap();

    let result = cmd_fmt(tmp.path().to_path_buf(), true);
    assert!(
        result.is_err(),
        "check mode should return Err for unformatted files"
    );
}

#[test]
fn test_walk_dir_skips_symlink_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let pages = root.join("pages");
    std::fs::create_dir_all(&pages).unwrap();
    std::fs::write(pages.join("index.ts"), "const x = 1;").unwrap();

    // Create symlink loop: pages/loop -> pages
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&pages, pages.join("loop")).unwrap();
        let files = discover_source_files(root);
        // Should not stack overflow; symlink dir is skipped
        assert_eq!(files.len(), 1);
    }
}

#[test]
fn test_format_source_with_options() {
    let input = "const x = 'hello';\n";
    let opts = oxc_formatter::FormatOptions {
        quote_style: oxc_formatter::QuoteStyle::Double,
        ..Default::default()
    };
    let result = format_source(input, Path::new("test.ts"), &opts).unwrap();
    assert!(
        result.contains("\"hello\""),
        "expected double quotes with custom options, got: {result}"
    );
}
