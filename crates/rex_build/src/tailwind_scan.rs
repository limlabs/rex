use anyhow::Result;
use rex_core::RexConfig;
use rex_router::ScanResult;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Directories never worth scanning for Tailwind candidates.
const SKIP_DIRS: &[&str] = &["node_modules", ".rex", ".git", ".next", "public", "dist"];

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

    // Scan the entire project root for source files (components/, lib/, src/, etc.)
    // This matches Tailwind v4's automatic content detection behavior.
    scan_project_root(&config.project_root, &mut files);

    // Honor Tailwind v4 `@source` directives in the project's CSS entry files.
    // Unlike the project-root walk above, these may reference files OUTSIDE
    // `project_root` (e.g. a sibling monorepo package) — exactly the case the
    // root walk misses (issue #246).
    collect_source_directive_files(scan, &mut files);

    // Deduplicate
    let mut seen = HashSet::new();
    files.retain(|f| seen.insert(f.clone()));

    files
}

/// Collect source files referenced by Tailwind v4 `@source "<path>"` directives
/// in the project's CSS entry files.
///
/// Tailwind's built-in V8 compiler is handed a pre-scanned candidate list, so it
/// never performs its own filesystem scan of `@source` paths — that scan has to
/// happen here. Paths are resolved relative to the CSS file's own directory (NOT
/// `project_root`) and are added without any project-root restriction. The
/// file-path forms are honored:
///   - `@source "../shared/components"`     → recursive directory walk
///   - `@source "../shared/components/"`    → recursive directory walk
///   - `@source "../shared/**/*.{ts,tsx}"`  → walk the pre-glob base directory
///   - `@source "../shared/Button.tsx"`     → single file
///
/// `@source inline("…")` is intentionally skipped: its candidates are embedded in
/// the CSS and emitted by the Tailwind engine itself. `@source not "…"`
/// exclusions are also left as no-ops.
fn collect_source_directive_files(scan: &ScanResult, files: &mut Vec<PathBuf>) {
    let css_paths = match crate::tailwind::collect_all_css_import_paths(scan) {
        Ok(p) => p,
        Err(_) => return,
    };
    for css_path in &css_paths {
        let content = match fs::read_to_string(css_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        if !crate::tailwind::needs_tailwind(&content) {
            continue;
        }
        let css_dir = css_path.parent().unwrap_or_else(|| Path::new("."));
        for body in parse_source_directives(&content) {
            add_source_directive(css_dir, &body, files);
        }
    }
}

/// Extract the body of each `@source` at-rule (the text between `@source` and the
/// terminating `;`). Commented-out directives are not special-cased — a stray
/// match merely scans a few extra files, which is harmless given the candidate
/// scanner is intentionally over-inclusive.
fn parse_source_directives(css: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = css;
    while let Some(idx) = rest.find("@source") {
        let after = &rest[idx + "@source".len()..];
        // Require whitespace right after `@source` so we don't match `@sources`.
        if !after.chars().next().is_some_and(char::is_whitespace) {
            rest = after;
            continue;
        }
        let end = after.find(';').unwrap_or(after.len());
        let body = after[..end].trim();
        if !body.is_empty() {
            out.push(body.to_string());
        }
        rest = &after[end..];
    }
    out
}

/// Resolve one `@source` directive body to source files and add them to `files`.
fn add_source_directive(css_dir: &Path, body: &str, files: &mut Vec<PathBuf>) {
    let body = body.trim();
    // Exclusions (`not …`) and inline safelists (`inline(…)`) are handled by the
    // Tailwind engine itself; the candidate scanner skips them.
    if body.starts_with("not") || body.starts_with("inline") {
        return;
    }
    let path = match unquote(body) {
        Some(p) => p,
        None => return,
    };
    let (base, has_glob) = split_glob_base(path);
    let resolved = css_dir.join(base);
    if has_glob || resolved.is_dir() {
        scan_directory_filtered(&resolved, files, SKIP_DIRS);
    } else if resolved.is_file() {
        files.push(resolved);
    }
}

/// Strip a single- or double-quoted wrapper, returning the inner string.
fn unquote(s: &str) -> Option<&str> {
    let s = s.trim();
    let quote = match s.chars().next()? {
        c @ ('"' | '\'') => c,
        _ => return None,
    };
    let inner = &s[quote.len_utf8()..];
    let end = inner.find(quote)?;
    Some(&inner[..end])
}

/// Split a `@source` path into its non-glob base directory and whether a glob
/// pattern followed. Mirrors Tailwind v4: everything before the first path
/// segment containing a glob metacharacter is the base directory that gets
/// scanned.
fn split_glob_base(path: &str) -> (String, bool) {
    let mut base = Vec::new();
    for segment in path.split('/') {
        if segment.contains(['*', '?', '[', ']', '{', '}']) {
            return (base.join("/"), true);
        }
        base.push(segment);
    }
    (base.join("/"), false)
}

/// Scan the project root for source files, skipping non-source directories.
///
/// This ensures components outside `app/` and `pages/` (e.g. `components/`, `lib/`,
/// `src/`) are included in Tailwind candidate scanning — matching Tailwind v4's
/// automatic content detection.
fn scan_project_root(root: &std::path::Path, files: &mut Vec<PathBuf>) {
    scan_directory_filtered(root, files, SKIP_DIRS);
}

/// Recursively find source files in a directory, skipping directories in `skip`.
fn scan_directory_filtered(dir: &std::path::Path, files: &mut Vec<PathBuf>, skip: &[&str]) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !skip.iter().any(|s| *s == &*name) {
                scan_directory_filtered(&path, files, skip);
            }
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
    // Split on delimiters that can't be part of CSS class names. Parentheses are
    // a special case: they delimit tokens only OUTSIDE arbitrary-value brackets.
    // That keeps call-expression syntax splitting cleanly (`cn("flex", …)` yields
    // `cn` + `flex`, not `cn(`) while letting arbitrary values that embed CSS
    // functions survive intact — e.g. `text-[var(--color-accent)]`,
    // `w-[calc(100%-2rem)]`, `bg-[image:theme(colors.red)]`. Without this, the
    // built-in compiler would never emit those utilities (issue: parenthesized
    // arbitrary values dropped during candidate scanning).
    let mut token_start: Option<usize> = None;
    let mut bracket_depth: u32 = 0;
    for (i, c) in source.char_indices() {
        match c {
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            _ => {}
        }
        let delimits = is_delimiter(c) || (bracket_depth == 0 && matches!(c, '(' | ')'));
        if delimits {
            if let Some(start) = token_start.take() {
                insert_candidate(&source[start..i], candidates);
            }
        } else if token_start.is_none() {
            token_start = Some(i);
        }
    }
    if let Some(start) = token_start {
        insert_candidate(&source[start..], candidates);
    }
}

/// Trim a raw token and add it to the candidate set if it looks like a utility.
fn insert_candidate(token: &str, candidates: &mut HashSet<String>) {
    let token = token.trim();
    if is_candidate(token) {
        candidates.insert(token.to_string());
    }
}

/// Characters that unconditionally delimit tokens (can't appear in utility class
/// names). Parentheses are handled contextually in [`scan_source`] — they are
/// delimiters only outside `[…]` arbitrary values — so they are deliberately
/// absent here.
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
    fn test_scan_arbitrary_values_with_parentheses() {
        // Arbitrary values that embed CSS functions must survive tokenization —
        // the parentheses must NOT split them. Regression for parenthesized
        // arbitrary values being dropped during candidate scanning.
        let mut candidates = HashSet::new();
        scan_source(
            r#"className="text-[var(--color-accent)] w-[calc(100%-2rem)] bg-[image:theme(colors.red)]""#,
            &mut candidates,
        );
        assert!(
            candidates.contains("text-[var(--color-accent)]"),
            "var() arbitrary value must survive, got: {candidates:?}"
        );
        assert!(
            candidates.contains("w-[calc(100%-2rem)]"),
            "calc() arbitrary value must survive, got: {candidates:?}"
        );
        assert!(
            candidates.contains("bg-[image:theme(colors.red)]"),
            "theme() arbitrary value must survive, got: {candidates:?}"
        );
    }

    #[test]
    fn test_scan_variant_arbitrary_value_with_parens() {
        // The real-world pantry-host case: a `hover:` variant on a parenthesized
        // arbitrary value (`hover:text-[var(--color-accent)]`).
        let mut candidates = HashSet::new();
        scan_source(
            r#"<a className="hover:text-[var(--color-accent)]">link</a>"#,
            &mut candidates,
        );
        assert!(
            candidates.contains("hover:text-[var(--color-accent)]"),
            "variant + parenthesized arbitrary value must survive, got: {candidates:?}"
        );
    }

    #[test]
    fn test_scan_clsx_with_conditional_and_parens() {
        // Parentheses outside `[…]` still delimit, so `cn(`/`)` don't pollute the
        // real classes — `flex` and `p-4` come through cleanly.
        let mut candidates = HashSet::new();
        scan_source(r#"cn("flex", x && "p-4")"#, &mut candidates);
        assert!(candidates.contains("flex"));
        assert!(candidates.contains("p-4"));
        // The call expression itself is split on its parens, so the bare
        // function name is the clean token (never `cn(`).
        assert!(!candidates.contains("cn("));
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

    #[test]
    fn test_scan_project_root_includes_components_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let comp_dir = tmp.path().join("components");
        fs::create_dir_all(&comp_dir).unwrap();
        fs::write(
            comp_dir.join("Sidebar.tsx"),
            r#"export default function Sidebar() { return <div className="bg-slate-900">hi</div>; }"#,
        )
        .unwrap();
        // Also create a file in node_modules (should be skipped)
        let nm = tmp.path().join("node_modules").join("pkg");
        fs::create_dir_all(&nm).unwrap();
        fs::write(nm.join("index.js"), "className=\"should-not-appear\"").unwrap();

        let mut files = Vec::new();
        scan_project_root(tmp.path(), &mut files);

        let names: Vec<String> = files.iter().map(|f| f.display().to_string()).collect();
        assert!(
            names.iter().any(|n| n.contains("Sidebar.tsx")),
            "should find components/Sidebar.tsx, got: {names:?}"
        );
        assert!(
            !names.iter().any(|n| n.contains("node_modules")),
            "should skip node_modules, got: {names:?}"
        );
    }

    #[test]
    fn test_parse_source_directives() {
        let css = "@import \"tailwindcss\";\n\
                   @source \"../shared\";\n\
                   @source inline(\"foo bar\");\n\
                   @source not \"../excluded\";\n";
        assert_eq!(
            parse_source_directives(css),
            vec![
                "\"../shared\"".to_string(),
                "inline(\"foo bar\")".to_string(),
                "not \"../excluded\"".to_string(),
            ]
        );
    }

    #[test]
    fn test_parse_source_directives_ignores_non_directive_token() {
        // `@sources` (no whitespace) must not be treated as a directive.
        let css = "@sources nothing;\n@source \"./real\";\n";
        assert_eq!(parse_source_directives(css), vec!["\"./real\"".to_string()]);
    }

    #[test]
    fn test_unquote() {
        assert_eq!(unquote("\"abc\""), Some("abc"));
        assert_eq!(unquote("'abc'"), Some("abc"));
        assert_eq!(unquote("  \"abc\"  "), Some("abc"));
        assert_eq!(unquote("abc"), None);
        assert_eq!(unquote("\"unterminated"), None);
    }

    #[test]
    fn test_split_glob_base() {
        assert_eq!(
            split_glob_base("../shared/src/components"),
            ("../shared/src/components".to_string(), false)
        );
        assert_eq!(
            split_glob_base("../shared/src/**/*.{ts,tsx}"),
            ("../shared/src".to_string(), true)
        );
        assert_eq!(
            split_glob_base("../shared/*.tsx"),
            ("../shared".to_string(), true)
        );
    }

    #[test]
    fn test_add_source_directive_skips_inline_and_not() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut files = Vec::new();
        add_source_directive(tmp.path(), "inline(\"foo bar\")", &mut files);
        add_source_directive(tmp.path(), "not \"./x\"", &mut files);
        assert!(
            files.is_empty(),
            "inline/not directives add no files: {files:?}"
        );
    }

    #[test]
    fn test_add_source_directive_walks_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        let comp = tmp.path().join("shared/components");
        fs::create_dir_all(&comp).unwrap();
        fs::write(comp.join("Card.tsx"), "className=\"text-emerald-600\"").unwrap();
        // node_modules under the source dir must still be skipped.
        let nm = comp.join("node_modules");
        fs::create_dir_all(&nm).unwrap();
        fs::write(nm.join("dep.js"), "className=\"should-not-appear\"").unwrap();

        let css_dir = tmp.path().join("app/styles");
        fs::create_dir_all(&css_dir).unwrap();

        let mut files = Vec::new();
        add_source_directive(&css_dir, "\"../../shared/components\"", &mut files);

        let names: Vec<String> = files.iter().map(|f| f.display().to_string()).collect();
        assert!(
            names.iter().any(|n| n.contains("Card.tsx")),
            "should walk the @source directory, got: {names:?}"
        );
        assert!(
            !names.iter().any(|n| n.contains("node_modules")),
            "should skip node_modules inside @source dir, got: {names:?}"
        );
    }

    #[test]
    fn test_add_source_directive_single_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let comp = tmp.path().join("shared");
        fs::create_dir_all(&comp).unwrap();
        let file = comp.join("Only.tsx");
        fs::write(&file, "className=\"p-7\"").unwrap();

        let css_dir = tmp.path().join("app");
        fs::create_dir_all(&css_dir).unwrap();

        let mut files = Vec::new();
        add_source_directive(&css_dir, "\"../shared/Only.tsx\"", &mut files);
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("Only.tsx"));
    }

    /// Regression for #246: a class used only in a sibling package referenced via
    /// an out-of-root `@source` directive must still end up in the candidate set.
    #[test]
    fn test_scan_candidates_honors_out_of_root_at_source() {
        let tmp = tempfile::TempDir::new().unwrap();
        // project_root = <tmp>/app ; sibling package = <tmp>/shared
        let app = tmp.path().join("app");
        let pages = app.join("pages");
        let styles = app.join("styles");
        fs::create_dir_all(&pages).unwrap();
        fs::create_dir_all(&styles).unwrap();
        let shared_components = tmp.path().join("shared/src/components");
        fs::create_dir_all(&shared_components).unwrap();

        // CSS entry imports Tailwind and points @source at the sibling package.
        fs::write(
            styles.join("globals.css"),
            "@import \"tailwindcss\";\n@source \"../../shared/src/components\";\n",
        )
        .unwrap();

        // _app imports the CSS; it does NOT use the sibling-only class.
        let app_entry = pages.join("_app.tsx");
        fs::write(
            &app_entry,
            "import '../styles/globals.css';\nexport default function App() { return null; }\n",
        )
        .unwrap();

        // The sibling component is the ONLY place this class appears.
        fs::write(
            shared_components.join("Widget.tsx"),
            "export default function Widget() { return <div className=\"bg-fuchsia-700\">hi</div>; }",
        )
        .unwrap();

        let config = rex_core::RexConfig {
            project_root: app.clone(),
            pages_dir: pages.clone(),
            app_dir: app.join("app"),
            output_dir: app.join(".rex"),
            port: 3000,
            dev: false,
        };
        let scan = rex_router::ScanResult {
            app: Some(rex_core::Route {
                pattern: "/_app".to_string(),
                file_path: app_entry.clone(),
                abs_path: app_entry,
                dynamic_segments: vec![],
                page_type: rex_core::PageType::Regular,
                specificity: 0,
            }),
            routes: vec![],
            not_found: None,
            error: None,
            document: None,
            middleware: None,
            mcp_tools: vec![],
            api_routes: vec![],
            app_scan: None,
        };

        let candidates = scan_candidates(&config, &scan).unwrap();
        assert!(
            candidates.iter().any(|c| c == "bg-fuchsia-700"),
            "out-of-root @source should contribute its candidates, got: {candidates:?}"
        );
    }
}
