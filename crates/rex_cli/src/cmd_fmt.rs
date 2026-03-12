use anyhow::Result;
use rayon::prelude::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::cmd_lint::is_ignored;
use crate::display::*;

pub(crate) fn cmd_fmt(root: PathBuf, check: bool) -> Result<()> {
    let root = std::fs::canonicalize(&root)?;
    let options = load_format_options(&root);
    let ignore_patterns = load_ignore_patterns(&root);
    let mut files = discover_source_files(&root);

    if !ignore_patterns.is_empty() {
        files.retain(|f| !is_ignored(f, &root, &ignore_patterns));
    }

    if files.is_empty() {
        eprintln!();
        eprintln!("  {} {}", dim("◆ rex fmt"), dim("(oxfmt)"));
        eprintln!();
        eprintln!(
            "  {} {}",
            dim("No source files found in"),
            dim(&root.display().to_string())
        );
        eprintln!();
        return Ok(());
    }

    eprintln!();
    eprintln!("  {} {}", magenta_bold("◆ rex fmt"), dim("(oxfmt)"));
    eprintln!();

    let changed_count = AtomicUsize::new(0);
    let error_count = AtomicUsize::new(0);
    let unformatted: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

    files.par_iter().for_each(|path| {
        let source = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("  {} {}: {e}", dim("skip"), path.display());
                error_count.fetch_add(1, Ordering::Relaxed);
                return;
            }
        };

        let formatted = match format_source(&source, path, &options) {
            Ok(f) => f,
            Err(_) => {
                let rel = path.strip_prefix(&root).unwrap_or(path);
                eprintln!("  {} {} (parse error)", dim("skip"), rel.display());
                error_count.fetch_add(1, Ordering::Relaxed);
                return;
            }
        };

        if formatted != source {
            if check {
                if let Ok(mut list) = unformatted.lock() {
                    list.push(path.clone());
                }
            } else {
                match std::fs::write(path, &formatted) {
                    Ok(()) => {
                        changed_count.fetch_add(1, Ordering::Relaxed);
                        let rel = path.strip_prefix(&root).unwrap_or(path);
                        eprintln!("  {} {}", dim("fmt"), rel.display());
                    }
                    Err(e) => {
                        eprintln!("  {} {}: {e}", dim("error"), path.display());
                        error_count.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }
    });

    let changed = changed_count.load(Ordering::Relaxed);
    let errors = error_count.load(Ordering::Relaxed);

    if check {
        let unformatted = unformatted.into_inner().unwrap_or_default();
        if unformatted.is_empty() {
            eprintln!(
                "  {} {}",
                green_bold("✓"),
                green_bold("All files formatted")
            );
            eprintln!();
            Ok(())
        } else {
            for path in &unformatted {
                let rel = path.strip_prefix(&root).unwrap_or(path);
                eprintln!("  {} {}", dim("unformatted"), rel.display());
            }
            eprintln!();
            eprintln!(
                "  {} {}",
                bold(&format!("{} file(s) need formatting", unformatted.len())),
                dim("(run `rex fmt` to fix)")
            );
            eprintln!();
            std::process::exit(1);
        }
    } else {
        if changed == 0 && errors == 0 {
            eprintln!(
                "  {} {}",
                green_bold("✓"),
                green_bold("All files formatted")
            );
        } else if changed > 0 {
            eprintln!();
            eprintln!(
                "  {} {}",
                green_bold("✓"),
                green_bold(&format!("Formatted {changed} file(s)"))
            );
        }
        if errors > 0 {
            eprintln!(
                "  {} {}",
                dim("⚠"),
                dim(&format!("{errors} file(s) skipped"))
            );
        }
        eprintln!();
        Ok(())
    }
}

fn discover_source_files(root: &std::path::Path) -> Vec<PathBuf> {
    let extensions: &[&str] = &["ts", "tsx", "js", "jsx"];
    let skip_dirs: &[&str] = &["node_modules", ".rex", ".git", "dist", "target", ".next"];

    let mut files = Vec::new();

    // Scan pages/ and styles/ directories
    let scan_dirs = ["pages", "styles", "components", "lib", "utils", "src"];
    for dir_name in &scan_dirs {
        let dir = root.join(dir_name);
        if dir.is_dir() {
            walk_dir(&dir, extensions, skip_dirs, &mut files);
        }
    }

    // Also pick up root-level config files (e.g., next.config.js)
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if extensions.contains(&ext) {
                        files.push(path);
                    }
                }
            }
        }
    }

    files.sort();
    files.dedup();
    files
}

fn walk_dir(
    dir: &std::path::Path,
    extensions: &[&str],
    skip_dirs: &[&str],
    files: &mut Vec<PathBuf>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !skip_dirs.contains(&name) {
                walk_dir(&path, extensions, skip_dirs, files);
            }
        } else if path.is_file() {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if extensions.contains(&ext) {
                    files.push(path);
                }
            }
        }
    }
}

fn load_format_options(root: &std::path::Path) -> oxc_formatter::FormatOptions {
    let config_files = [".prettierrc", ".prettierrc.json"];

    let json_value: Option<serde_json::Value> = config_files
        .iter()
        .find_map(|name| {
            let path = root.join(name);
            let content = std::fs::read_to_string(&path).ok()?;
            serde_json::from_str(&content).ok()
        })
        .or_else(|| {
            let pkg_path = root.join("package.json");
            let content = std::fs::read_to_string(&pkg_path).ok()?;
            let pkg: serde_json::Value = serde_json::from_str(&content).ok()?;
            pkg.get("prettier").cloned()
        });

    let Some(config) = json_value else {
        return oxc_formatter::FormatOptions {
            quote_style: oxc_formatter::QuoteStyle::Single,
            ..Default::default()
        };
    };

    let mut options = oxc_formatter::FormatOptions::default();

    if let Some(v) = config.get("singleQuote").and_then(|v| v.as_bool()) {
        options.quote_style = if v {
            oxc_formatter::QuoteStyle::Single
        } else {
            oxc_formatter::QuoteStyle::Double
        };
    }

    if let Some(v) = config.get("jsxSingleQuote").and_then(|v| v.as_bool()) {
        options.jsx_quote_style = if v {
            oxc_formatter::QuoteStyle::Single
        } else {
            oxc_formatter::QuoteStyle::Double
        };
    }

    if let Some(v) = config.get("tabWidth").and_then(|v| v.as_u64()) {
        if let Ok(w) = oxc_formatter::IndentWidth::try_from(v as u8) {
            options.indent_width = w;
        }
    }

    if let Some(v) = config.get("useTabs").and_then(|v| v.as_bool()) {
        options.indent_style = if v {
            oxc_formatter::IndentStyle::Tab
        } else {
            oxc_formatter::IndentStyle::Space
        };
    }

    if let Some(v) = config.get("printWidth").and_then(|v| v.as_u64()) {
        if let Ok(w) = oxc_formatter::LineWidth::try_from(v as u16) {
            options.line_width = w;
        }
    }

    if let Some(v) = config.get("semi").and_then(|v| v.as_bool()) {
        options.semicolons = if v {
            oxc_formatter::Semicolons::Always
        } else {
            oxc_formatter::Semicolons::AsNeeded
        };
    }

    if let Some(v) = config.get("trailingComma").and_then(|v| v.as_str()) {
        options.trailing_commas = match v {
            "all" => oxc_formatter::TrailingCommas::All,
            "none" => oxc_formatter::TrailingCommas::None,
            "es5" => oxc_formatter::TrailingCommas::Es5,
            _ => options.trailing_commas,
        };
    }

    if let Some(v) = config.get("bracketSpacing").and_then(|v| v.as_bool()) {
        options.bracket_spacing = oxc_formatter::BracketSpacing::from(v);
    }

    if let Some(v) = config.get("bracketSameLine").and_then(|v| v.as_bool()) {
        options.bracket_same_line = oxc_formatter::BracketSameLine::from(v);
    }

    if let Some(v) = config.get("arrowParens").and_then(|v| v.as_str()) {
        options.arrow_parentheses = match v {
            "avoid" => oxc_formatter::ArrowParentheses::AsNeeded,
            "always" => oxc_formatter::ArrowParentheses::Always,
            _ => options.arrow_parentheses,
        };
    }

    if let Some(v) = config.get("endOfLine").and_then(|v| v.as_str()) {
        options.line_ending = match v {
            "lf" => oxc_formatter::LineEnding::Lf,
            "crlf" => oxc_formatter::LineEnding::Crlf,
            "cr" => oxc_formatter::LineEnding::Cr,
            _ => options.line_ending,
        };
    }

    options
}

fn load_ignore_patterns(root: &std::path::Path) -> Vec<String> {
    let ignore_path = root.join(".prettierignore");
    let content = match std::fs::read_to_string(&ignore_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.to_string())
        .collect()
}

fn format_source(
    source: &str,
    path: &std::path::Path,
    options: &oxc_formatter::FormatOptions,
) -> Result<String> {
    let source_type = oxc_span::SourceType::from_path(path)
        .map_err(|e| anyhow::anyhow!("unsupported file type: {e}"))?;
    let allocator = oxc_allocator::Allocator::default();
    let parse_options = oxc_parser::ParseOptions {
        preserve_parens: false,
        ..Default::default()
    };
    let parsed = oxc_parser::Parser::new(&allocator, source, source_type)
        .with_options(parse_options)
        .parse();
    if !parsed.errors.is_empty() {
        anyhow::bail!("parse error");
    }
    Ok(oxc_formatter::Formatter::new(&allocator, options.clone()).build(&parsed.program))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
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
}
