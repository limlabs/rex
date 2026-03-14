use anyhow::Result;
use std::path::PathBuf;

use crate::display::*;

pub(crate) fn cmd_lint(
    root: PathBuf,
    fix: bool,
    deny_warnings: bool,
    paths: Vec<PathBuf>,
) -> Result<()> {
    use oxc_linter::{
        ConfigStore, ConfigStoreBuilder, ExternalPluginStore, FixKind, LintOptions, LintRunner,
        LintServiceOptions, Linter, Oxlintrc,
    };
    use std::ffi::OsStr;
    use std::sync::Arc;

    let root = std::fs::canonicalize(&root)?;

    // Load config: .oxlintrc.json if present, otherwise use Rex defaults
    let config_path = root.join(".oxlintrc.json");
    let oxlintrc = if config_path.exists() {
        Oxlintrc::from_file(&config_path)
            .map_err(|e| anyhow::anyhow!("failed to parse .oxlintrc.json: {e}"))?
    } else {
        Oxlintrc::from_string(default_oxlintrc())
            .map_err(|e| anyhow::anyhow!("failed to parse default oxlintrc: {e}"))?
    };

    // Build config store
    let mut external_plugin_store = ExternalPluginStore::default();
    let config_builder =
        ConfigStoreBuilder::from_oxlintrc(false, oxlintrc, None, &mut external_plugin_store, None)
            .map_err(|e| anyhow::anyhow!("failed to build lint config: {e}"))?;

    let base_config = config_builder
        .build(&mut external_plugin_store)
        .map_err(|e| anyhow::anyhow!("failed to build lint config: {e}"))?;

    let config_store = ConfigStore::new(base_config, Default::default(), external_plugin_store);

    // Create linter
    let fix_kind = if fix { FixKind::SafeFix } else { FixKind::None };
    let linter = Linter::new(LintOptions::default(), config_store, None).with_fix(fix_kind);

    // Determine lint targets
    let lint_dirs: Vec<PathBuf> = if paths.is_empty() {
        let pages_dir = root.join("pages");
        if pages_dir.is_dir() {
            vec![pages_dir]
        } else {
            vec![root.clone()]
        }
    } else {
        paths
            .into_iter()
            .map(|p| if p.is_absolute() { p } else { root.join(p) })
            .collect()
    };

    // Discover source files (respecting .gitignore + hardcoded skip dirs)
    let gitignore = load_gitignore_patterns(&root);
    let mut files: Vec<PathBuf> = Vec::new();
    for dir in &lint_dirs {
        if dir.is_file() {
            files.push(dir.clone());
        } else if dir.is_dir() {
            walk_lint_dir(dir, &root, &gitignore, &mut files);
        }
    }

    if files.is_empty() {
        eprintln!();
        eprintln!("  {} {}", magenta_bold("◆ rex lint"), dim("(oxlint)"));
        eprintln!();
        eprintln!("  {} {}", dim("No source files found to lint"), dim(""));
        eprintln!();
        return Ok(());
    }

    eprintln!();
    eprintln!("  {} {}", magenta_bold("◆ rex lint"), dim("(oxlint)"));
    eprintln!();

    // Build LintRunner and execute
    let service_options = LintServiceOptions::new(root.clone().into_boxed_path());

    let lint_runner = LintRunner::builder(service_options, linter)
        .with_fix_kind(fix_kind)
        .build()
        .map_err(|e| anyhow::anyhow!("failed to build lint runner: {e}"))?;

    let file_paths: Vec<Arc<OsStr>> = files
        .iter()
        .map(|p| Arc::from(p.as_os_str().to_owned()))
        .collect();

    let (tx, rx) = std::sync::mpsc::channel::<Vec<oxc_diagnostics::Error>>();
    let lint_runner = lint_runner
        .lint_files(&file_paths, tx)
        .map_err(|e| anyhow::anyhow!("lint failed: {e}"))?;

    // Collect and display diagnostics
    let mut error_count: usize = 0;
    let mut warning_count: usize = 0;

    let _ = &lint_runner; // keep runner alive for fix writing

    for errors in rx {
        for error in &errors {
            let severity = error.severity().unwrap_or(oxc_diagnostics::Severity::Error);
            match severity {
                oxc_diagnostics::Severity::Error => error_count += 1,
                oxc_diagnostics::Severity::Warning => warning_count += 1,
                _ => {}
            }
            // Print diagnostics using miette-style formatting
            eprintln!("{error:?}");
        }
    }

    let total = error_count + warning_count;

    if total == 0 {
        eprintln!("  {} {}", green_bold("✓"), green_bold("No lint errors"));
        eprintln!();
        return Ok(());
    }

    eprintln!();
    if error_count > 0 {
        eprintln!(
            "  {} {}",
            bold(&format!("{error_count} error(s)")),
            if warning_count > 0 {
                format!("and {} warning(s)", warning_count)
            } else {
                String::new()
            }
        );
    } else {
        eprintln!("  {}", bold(&format!("{warning_count} warning(s)")));
    }
    eprintln!();

    if error_count > 0 || (deny_warnings && warning_count > 0) {
        std::process::exit(1);
    }

    Ok(())
}

/// Hardcoded directories that should always be skipped during linting,
/// even when no `.gitignore` is present.
const LINT_SKIP_DIRS: &[&str] = &["node_modules", ".rex", ".git", "dist", "target", ".next"];

/// Walk a directory recursively, collecting lintable source files.
fn walk_lint_dir(
    dir: &std::path::Path,
    root: &std::path::Path,
    gitignore: &[String],
    out: &mut Vec<PathBuf>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if LINT_SKIP_DIRS.contains(&name.as_ref()) || name.starts_with('.') {
                continue;
            }
            if !gitignore.is_empty() && is_ignored(&path, root, gitignore) {
                continue;
            }
            walk_lint_dir(&path, root, gitignore, out);
        } else if path.is_file() {
            if let Some("js" | "jsx" | "ts" | "tsx" | "mjs" | "mts") =
                path.extension().and_then(|e| e.to_str())
            {
                // Skip .d.ts / .d.mts / .d.cts type definition files
                if !path.file_name().is_some_and(|n| {
                    let n = n.to_string_lossy();
                    n.ends_with(".d.ts") || n.ends_with(".d.mts") || n.ends_with(".d.cts")
                }) {
                    if !gitignore.is_empty() && is_ignored(&path, root, gitignore) {
                        continue;
                    }
                    out.push(path);
                }
            }
        }
    }
}

/// Load ignore patterns from `.gitignore` (if present).
fn load_gitignore_patterns(root: &std::path::Path) -> Vec<String> {
    let gitignore_path = root.join(".gitignore");
    let content = match std::fs::read_to_string(&gitignore_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with('!'))
        // Strip leading "/" — .gitignore uses it for root-relative, our is_ignored already does prefix matching
        .map(|l| l.strip_prefix('/').unwrap_or(l).to_string())
        .collect()
}

pub(crate) fn is_ignored(
    path: &std::path::Path,
    root: &std::path::Path,
    patterns: &[String],
) -> bool {
    let rel = match path.strip_prefix(root) {
        Ok(r) => r.to_string_lossy().replace('\\', "/"),
        Err(_) => return false,
    };
    let rel_str = rel.as_str();

    for pattern in patterns {
        // Directory pattern (e.g., "dist/", "pages/api/")
        if let Some(dir) = pattern.strip_suffix('/') {
            if rel_str == dir || rel_str.starts_with(&format!("{dir}/")) {
                return true;
            }
        }
        // Glob-like extension pattern (e.g., "*.min.js")
        else if let Some(suffix) = pattern.strip_prefix('*') {
            if rel_str.ends_with(suffix) {
                return true;
            }
        }
        // Exact match or prefix match for bare directory names (e.g. "dist" matches "dist/foo.ts")
        else if rel_str == pattern || rel_str.starts_with(&format!("{pattern}/")) {
            return true;
        }
    }
    false
}

fn default_oxlintrc() -> &'static str {
    r#"{
  "$schema": "https://raw.githubusercontent.com/oxc-project/oxc/main/npm/oxlint/configuration_schema.json",
  "plugins": ["react", "react-hooks", "nextjs", "import"],
  "rules": {
    "react/jsx-no-target-blank": "warn",
    "react/no-unknown-property": "warn",
    "react/react-in-jsx-scope": "off",
    "react-hooks/rules-of-hooks": "error",
    "react-hooks/exhaustive-deps": "warn",
    "nextjs/no-html-link-for-pages": "warn",
    "nextjs/no-img-element": "warn",
    "nextjs/no-head-import-in-document": "warn",
    "nextjs/no-duplicate-head": "warn",
    "import/no-cycle": "warn",
    "no-var": "error"
  },
  "ignorePatterns": [".rex/", "node_modules/"]
}
"#
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn is_ignored_dir_pattern_no_false_prefix_match() {
        let root = Path::new("/project");
        let patterns = vec!["pages/api/".to_string()];

        // Should match: file inside pages/api/
        assert!(is_ignored(
            Path::new("/project/pages/api/hello.ts"),
            root,
            &patterns,
        ));

        // Should NOT match: pages/api-docs/ is a different directory
        assert!(!is_ignored(
            Path::new("/project/pages/api-docs/readme.ts"),
            root,
            &patterns,
        ));

        // Should match: exact directory name
        assert!(is_ignored(
            Path::new("/project/pages/api/nested/route.ts"),
            root,
            &patterns,
        ));
    }

    #[test]
    fn is_ignored_bare_dir_name() {
        let root = Path::new("/project");
        let patterns = vec!["dist".to_string()];

        assert!(is_ignored(
            Path::new("/project/dist/foo.ts"),
            root,
            &patterns
        ));
        assert!(!is_ignored(
            Path::new("/project/dist-old/foo.ts"),
            root,
            &patterns,
        ));
    }

    #[test]
    fn is_ignored_glob_extension() {
        let root = Path::new("/project");
        let patterns = vec!["*.min.js".to_string()];

        assert!(is_ignored(
            Path::new("/project/bundle.min.js"),
            root,
            &patterns,
        ));
        assert!(!is_ignored(
            Path::new("/project/bundle.js"),
            root,
            &patterns
        ));
    }

    #[test]
    fn is_ignored_exact_match() {
        let root = Path::new("/project");
        let patterns = vec!["Makefile".to_string()];

        assert!(is_ignored(Path::new("/project/Makefile"), root, &patterns));
        assert!(!is_ignored(
            Path::new("/project/Makefile.bak"),
            root,
            &patterns,
        ));
    }

    #[test]
    fn is_ignored_outside_root() {
        let root = Path::new("/project");
        let patterns = vec!["dist".to_string()];

        // Path not under root → never ignored
        assert!(!is_ignored(
            Path::new("/other/dist/foo.ts"),
            root,
            &patterns,
        ));
    }

    #[test]
    fn walk_lint_dir_skips_dts_dmts_dcts() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        std::fs::write(root.join("types.d.ts"), "").unwrap();
        std::fs::write(root.join("types.d.mts"), "").unwrap();
        std::fs::write(root.join("types.d.cts"), "").unwrap();
        std::fs::write(root.join("app.tsx"), "const x = 1;").unwrap();
        std::fs::write(root.join("utils.ts"), "const y = 2;").unwrap();

        let mut out = Vec::new();
        walk_lint_dir(root, root, &[], &mut out);

        let names: Vec<String> = out
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();

        assert!(!names.contains(&"types.d.ts".to_string()));
        assert!(!names.contains(&"types.d.mts".to_string()));
        assert!(!names.contains(&"types.d.cts".to_string()));
        assert!(names.contains(&"app.tsx".to_string()));
        assert!(names.contains(&"utils.ts".to_string()));
    }

    #[test]
    fn walk_lint_dir_skips_hardcoded_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        let nm = root.join("node_modules");
        std::fs::create_dir_all(&nm).unwrap();
        std::fs::write(nm.join("dep.js"), "").unwrap();

        let dot_rex = root.join(".rex");
        std::fs::create_dir_all(&dot_rex).unwrap();
        std::fs::write(dot_rex.join("cache.js"), "").unwrap();

        std::fs::write(root.join("index.tsx"), "const x = 1;").unwrap();

        let mut out = Vec::new();
        walk_lint_dir(root, root, &[], &mut out);

        assert_eq!(out.len(), 1);
        assert!(out[0].ends_with("index.tsx"));
    }

    #[test]
    fn load_gitignore_patterns_strips_slash_and_comments() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(".gitignore"),
            "# build output\n/dist\nnode_modules\n\n*.log\n",
        )
        .unwrap();

        let patterns = load_gitignore_patterns(tmp.path());
        assert_eq!(patterns, vec!["dist", "node_modules", "*.log"]);
    }

    #[test]
    fn load_gitignore_patterns_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let patterns = load_gitignore_patterns(tmp.path());
        assert!(patterns.is_empty());
    }

    #[test]
    fn load_gitignore_patterns_skips_negated() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(".gitignore"),
            "dist\n!dist/important.js\nnode_modules\n",
        )
        .unwrap();

        let patterns = load_gitignore_patterns(tmp.path());
        // Negated pattern should be filtered out (unsupported)
        assert_eq!(patterns, vec!["dist", "node_modules"]);
    }
}
