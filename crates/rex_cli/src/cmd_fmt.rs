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
            anyhow::bail!("{} file(s) need formatting", unformatted.len());
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
            if !skip_dirs.contains(&name) && !path.is_symlink() {
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
        if let Ok(v) = u8::try_from(v) {
            if let Ok(w) = oxc_formatter::IndentWidth::try_from(v) {
                options.indent_width = w;
            }
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
        if let Ok(v) = u16::try_from(v) {
            if let Ok(w) = oxc_formatter::LineWidth::try_from(v) {
                options.line_width = w;
            }
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
#[path = "cmd_fmt_tests.rs"]
mod tests;
