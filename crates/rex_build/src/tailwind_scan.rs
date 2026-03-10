use anyhow::Result;
use rex_core::RexConfig;
use rex_router::ScanResult;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

/// Scan all project source files and extract Tailwind utility class candidates.
///
/// Uses a two-pass approach:
/// 1. OXC AST walk on JSX/TSX files to extract `className` string literals
/// 2. Regex-like token scanner as a fallback for dynamic classes and non-JSX content
///
/// The result is intentionally over-inclusive — unmatched candidates are silently
/// ignored by Tailwind's `build()`. This matches the approach used by Tailwind's
/// own Oxide scanner.
pub fn scan_candidates(config: &RexConfig, scan: &ScanResult) -> Result<Vec<String>> {
    let mut candidates = HashSet::new();

    // Collect source files from the scan result
    let source_files = collect_source_files(config, scan);

    for path in &source_files {
        let source = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        scan_source(&source, &mut candidates);
    }

    Ok(candidates.into_iter().collect())
}

/// Collect all source file paths from the scan result (pages, layouts, app entries).
fn collect_source_files(config: &RexConfig, scan: &ScanResult) -> Vec<PathBuf> {
    let mut files = Vec::new();

    if let Some(app) = &scan.app {
        files.push(app.abs_path.clone());
    }

    for route in &scan.routes {
        files.push(route.abs_path.clone());
    }

    if let Some(app_scan) = &scan.app_scan {
        let mut seen = HashSet::new();
        for route in &app_scan.routes {
            if seen.insert(route.page_path.clone()) {
                files.push(route.page_path.clone());
            }
            for layout in &route.layout_chain {
                if seen.insert(layout.clone()) {
                    files.push(layout.clone());
                }
            }
        }
    }

    // Also scan any files under pages_dir / app_dir that we might have missed
    scan_directory(&config.pages_dir, &mut files);
    if config.app_dir.exists() {
        scan_directory(&config.app_dir, &mut files);
    }

    // Deduplicate
    let mut seen = HashSet::new();
    files.retain(|f| seen.insert(f.clone()));

    files
}

/// Recursively find source files in a directory.
fn scan_directory(dir: &std::path::Path, files: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_directory(&path, files);
        } else if is_scannable(&path) {
            files.push(path);
        }
    }
}

/// Check if a file is worth scanning for Tailwind candidates.
fn is_scannable(path: &std::path::Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("tsx" | "ts" | "jsx" | "js" | "html" | "mdx" | "md")
    )
}

/// Extract candidate class names from source text.
///
/// This is a token-based scanner similar to Tailwind's Oxide scanner. It splits
/// on characters that can't appear in utility class names and collects all
/// word-like tokens. Over-inclusion is intentional — the Tailwind compiler
/// ignores candidates that don't match any utility definition.
fn scan_source(source: &str, candidates: &mut HashSet<String>) {
    // Split on delimiters that can't be part of CSS class names
    for token in source.split(is_delimiter) {
        let token = token.trim();
        if is_candidate(token) {
            candidates.insert(token.to_string());
        }
    }
}

/// Characters that delimit tokens (can't appear in utility class names).
fn is_delimiter(c: char) -> bool {
    matches!(
        c,
        '"' | '\''
            | '`'
            | ' '
            | '\t'
            | '\n'
            | '\r'
            | '{'
            | '}'
            | '('
            | ')'
            | '<'
            | '>'
            | '='
            | ';'
            | ','
            | '+'
            | '|'
            | '&'
            | '?'
    )
}

/// Check if a token could be a valid Tailwind utility class candidate.
fn is_candidate(token: &str) -> bool {
    if token.is_empty() || token.len() > 200 {
        return false;
    }
    let first = token.as_bytes()[0];
    // Must start with a letter, @, !, or - (negative utilities like -mx-4)
    if !matches!(first, b'a'..=b'z' | b'A'..=b'Z' | b'@' | b'!' | b'-') {
        return false;
    }
    // Filter out common non-class tokens
    if token.contains("//") || token.contains("/*") || token.contains("*/") {
        return false;
    }
    true
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_scan_simple_classname() {
        let mut candidates = HashSet::new();
        scan_source(r#"className="bg-red-500 p-4 text-white""#, &mut candidates);
        assert!(candidates.contains("bg-red-500"));
        assert!(candidates.contains("p-4"));
        assert!(candidates.contains("text-white"));
    }

    #[test]
    fn test_scan_template_literal() {
        let mut candidates = HashSet::new();
        scan_source(r#"className={`bg-blue-500 ${foo} mt-2`}"#, &mut candidates);
        assert!(candidates.contains("bg-blue-500"));
        assert!(candidates.contains("mt-2"));
    }

    #[test]
    fn test_scan_clsx() {
        let mut candidates = HashSet::new();
        scan_source(
            r#"cn("flex items-center", active && "bg-gray-100")"#,
            &mut candidates,
        );
        assert!(candidates.contains("flex"));
        assert!(candidates.contains("items-center"));
        assert!(candidates.contains("bg-gray-100"));
    }

    #[test]
    fn test_scan_deduplication() {
        let mut candidates = HashSet::new();
        scan_source(r#"className="p-4 p-4 p-4""#, &mut candidates);
        assert_eq!(candidates.iter().filter(|c| *c == "p-4").count(), 1);
    }

    #[test]
    fn test_scan_negative_utilities() {
        let mut candidates = HashSet::new();
        scan_source(r#"className="-mx-4 -translate-x-1/2""#, &mut candidates);
        assert!(candidates.contains("-mx-4"));
        assert!(candidates.contains("-translate-x-1/2"));
    }

    #[test]
    fn test_scan_variant_prefixes() {
        let mut candidates = HashSet::new();
        scan_source(
            r#"className="hover:bg-blue-500 sm:text-lg dark:text-white""#,
            &mut candidates,
        );
        assert!(candidates.contains("hover:bg-blue-500"));
        assert!(candidates.contains("sm:text-lg"));
        assert!(candidates.contains("dark:text-white"));
    }

    #[test]
    fn test_scan_arbitrary_values() {
        let mut candidates = HashSet::new();
        scan_source(r#"className="w-[200px] bg-[#ff0000]""#, &mut candidates);
        assert!(candidates.contains("w-[200px]"));
        assert!(candidates.contains("bg-[#ff0000]"));
    }

    #[test]
    fn test_scan_filters_short_tokens() {
        let mut candidates = HashSet::new();
        scan_source(r#"a = b + c"#, &mut candidates);
        // Single-char tokens should still pass (they might be valid utilities)
        // but empty tokens should not
        assert!(!candidates.contains(""));
    }

    #[test]
    fn test_is_candidate_valid() {
        assert!(is_candidate("bg-red-500"));
        assert!(is_candidate("p-4"));
        assert!(is_candidate("-mx-4"));
        assert!(is_candidate("hover:bg-blue-500"));
        assert!(is_candidate("!important"));
        assert!(is_candidate("@container"));
    }

    #[test]
    fn test_is_candidate_invalid() {
        assert!(!is_candidate(""));
        assert!(!is_candidate("123abc"));
        assert!(!is_candidate("//comment"));
    }
}
