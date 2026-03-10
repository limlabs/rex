use crate::css_collect::extract_css_imports;
use anyhow::Result;
use rex_core::RexConfig;
use rex_router::ScanResult;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::info;

/// Check if a CSS file contains Tailwind directives (v4 or v3).
pub fn needs_tailwind(content: &str) -> bool {
    content.lines().any(|line| {
        let t = line.trim();
        t.starts_with("@import \"tailwindcss\"")
            || t.starts_with("@import 'tailwindcss'")
            || t.starts_with("@tailwind ")
    })
}

/// Find the tailwindcss CLI binary in the project's node_modules.
pub fn find_tailwind_bin(project_root: &Path) -> Option<PathBuf> {
    let local = project_root.join("node_modules/.bin/tailwindcss");
    if local.exists() {
        return Some(local);
    }
    None
}

/// Run a one-shot Tailwind CSS compilation.
fn run_tailwind(bin: &Path, input: &Path, output: &Path, project_root: &Path) -> Result<()> {
    let status = Command::new(bin)
        .arg("-i")
        .arg(input)
        .arg("-o")
        .arg(output)
        .arg("--minify")
        .current_dir(project_root)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .status()?;

    if !status.success() {
        anyhow::bail!("tailwindcss exited with status {status}");
    }
    Ok(())
}

/// Collect all CSS import paths from _app, pages, and app/ routes (reusing extract_css_imports).
pub fn collect_all_css_import_paths(scan: &ScanResult) -> Result<Vec<PathBuf>> {
    let mut all = Vec::new();
    if let Some(app) = &scan.app {
        all.extend(extract_css_imports(&app.abs_path)?);
    }
    for route in &scan.routes {
        all.extend(extract_css_imports(&route.abs_path)?);
    }
    // Also scan app/ router files (layouts and pages) for CSS imports
    if let Some(app_scan) = &scan.app_scan {
        let mut seen = std::collections::HashSet::new();
        for route in &app_scan.routes {
            if seen.insert(route.page_path.clone()) {
                all.extend(extract_css_imports(&route.page_path)?);
            }
            for layout in &route.layout_chain {
                if seen.insert(layout.clone()) {
                    all.extend(extract_css_imports(layout)?);
                }
            }
        }
    }
    Ok(all)
}

/// Pre-process Tailwind CSS files. Returns a map of original CSS path → processed output path.
/// If no Tailwind CSS files are found, returns an empty map.
pub fn process_tailwind_css(
    config: &RexConfig,
    scan: &ScanResult,
    output_dir: &Path,
) -> Result<HashMap<PathBuf, PathBuf>> {
    let all_css = collect_all_css_import_paths(scan)?;
    let tw_bin = find_tailwind_bin(&config.project_root);

    let mut mappings = HashMap::new();

    for css_path in &all_css {
        if !css_path.exists() {
            continue;
        }
        let content = fs::read_to_string(css_path)?;
        if !needs_tailwind(&content) {
            continue;
        }
        let bin = tw_bin.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "CSS file {} uses Tailwind directives but tailwindcss is not installed.\n\
                 Install it: npm install @tailwindcss/cli",
                css_path.display()
            )
        })?;

        let stem = css_path.file_stem().unwrap_or_default().to_string_lossy();
        let tw_output = output_dir.join(format!("{stem}.tailwind.css"));
        info!(input = %css_path.display(), "Processing Tailwind CSS");
        run_tailwind(bin, css_path, &tw_output, &config.project_root)?;
        mappings.insert(css_path.clone(), tw_output);
    }

    Ok(mappings)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    #[test]
    fn test_find_tailwind_bin_found() {
        let tmp = TempDir::new().expect("test setup");
        let bin_dir = tmp.path().join("node_modules/.bin");
        fs::create_dir_all(&bin_dir).expect("test setup");
        let bin_path = bin_dir.join("tailwindcss");
        fs::write(&bin_path, "#!/bin/sh\n").expect("test setup");
        fs::set_permissions(&bin_path, fs::Permissions::from_mode(0o755)).expect("test setup");
        assert!(find_tailwind_bin(tmp.path()).is_some());
    }

    #[test]
    fn test_find_tailwind_bin_not_found() {
        let tmp = TempDir::new().expect("test setup");
        assert!(find_tailwind_bin(tmp.path()).is_none());
    }

    #[test]
    fn test_collect_all_css_import_paths_from_app() {
        let tmp = TempDir::new().unwrap();
        let pages_dir = tmp.path().join("pages");
        fs::create_dir_all(&pages_dir).unwrap();

        let css_file = tmp.path().join("globals.css");
        fs::write(&css_file, "@import \"tailwindcss\";").unwrap();

        let app = pages_dir.join("_app.tsx");
        fs::write(
            &app,
            format!(
                "import '{}';\nexport default function App() {{}}\n",
                css_file.display()
            ),
        )
        .unwrap();

        let scan = rex_router::ScanResult {
            app: Some(rex_core::Route {
                pattern: "/_app".to_string(),
                file_path: app.clone(),
                abs_path: app,
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

        let paths = collect_all_css_import_paths(&scan).unwrap();
        assert_eq!(paths.len(), 1);
        assert!(paths[0].ends_with("globals.css"));
    }

    #[test]
    fn test_collect_all_css_import_paths_from_pages() {
        let tmp = TempDir::new().unwrap();
        let pages_dir = tmp.path().join("pages");
        fs::create_dir_all(&pages_dir).unwrap();

        let css_file = pages_dir.join("about.css");
        fs::write(&css_file, "body {}").unwrap();

        let page = pages_dir.join("about.tsx");
        fs::write(
            &page,
            format!(
                "import '{}';\nexport default function About() {{}}\n",
                css_file.display()
            ),
        )
        .unwrap();

        let scan = rex_router::ScanResult {
            app: None,
            routes: vec![rex_core::Route {
                pattern: "/about".to_string(),
                file_path: page.clone(),
                abs_path: page,
                dynamic_segments: vec![],
                page_type: rex_core::PageType::Regular,
                specificity: 0,
            }],
            not_found: None,
            error: None,
            document: None,
            middleware: None,
            mcp_tools: vec![],
            api_routes: vec![],
            app_scan: None,
        };

        let paths = collect_all_css_import_paths(&scan).unwrap();
        assert_eq!(paths.len(), 1);
    }

    #[test]
    fn test_collect_all_css_import_paths_empty() {
        let tmp = TempDir::new().unwrap();
        let page = tmp.path().join("index.tsx");
        fs::write(&page, "export default function Home() {}\n").unwrap();

        let scan = rex_router::ScanResult {
            app: None,
            routes: vec![rex_core::Route {
                pattern: "/".to_string(),
                file_path: page.clone(),
                abs_path: page,
                dynamic_segments: vec![],
                page_type: rex_core::PageType::Regular,
                specificity: 0,
            }],
            not_found: None,
            error: None,
            document: None,
            middleware: None,
            mcp_tools: vec![],
            api_routes: vec![],
            app_scan: None,
        };

        let paths = collect_all_css_import_paths(&scan).unwrap();
        assert!(paths.is_empty());
    }

    #[test]
    fn test_process_tailwind_css_no_tailwind_files() {
        let tmp = TempDir::new().unwrap();
        let pages_dir = tmp.path().join("pages");
        fs::create_dir_all(&pages_dir).unwrap();

        let css_file = pages_dir.join("styles.css");
        fs::write(&css_file, "body { margin: 0; }").unwrap();

        let app = pages_dir.join("_app.tsx");
        fs::write(
            &app,
            format!(
                "import '{}';\nexport default function App() {{}}\n",
                css_file.display()
            ),
        )
        .unwrap();

        let output_dir = tmp.path().join("output");
        fs::create_dir_all(&output_dir).unwrap();

        let config = rex_core::RexConfig {
            project_root: tmp.path().to_path_buf(),
            pages_dir: pages_dir.clone(),
            app_dir: tmp.path().join("app"),
            output_dir: output_dir.clone(),
            port: 3000,
            dev: false,
        };

        let scan = rex_router::ScanResult {
            app: Some(rex_core::Route {
                pattern: "/_app".to_string(),
                file_path: app.clone(),
                abs_path: app,
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

        let mappings = process_tailwind_css(&config, &scan, &output_dir).unwrap();
        assert!(mappings.is_empty(), "no tailwind directives = no mappings");
    }

    #[test]
    fn test_needs_tailwind_v4() {
        assert!(needs_tailwind("@import \"tailwindcss\";\n"));
        assert!(needs_tailwind("  @import \"tailwindcss\";\n"));
        assert!(needs_tailwind("@import 'tailwindcss';\n"));
    }

    #[test]
    fn test_needs_tailwind_v3() {
        assert!(needs_tailwind(
            "@tailwind base;\n@tailwind components;\n@tailwind utilities;\n"
        ));
        assert!(needs_tailwind("  @tailwind utilities;\n"));
    }

    #[test]
    fn test_needs_tailwind_negative() {
        assert!(!needs_tailwind("body { margin: 0; }\n"));
        assert!(!needs_tailwind(".container { max-width: 1200px; }\n"));
        assert!(!needs_tailwind("/* @import \"tailwindcss\" */\nbody {}\n"));
        assert!(!needs_tailwind(""));
    }

    #[test]
    #[ignore] // Requires tailwindcss CLI installed
    fn test_tailwind_processing() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();

        // Create styles dir
        let styles_dir = root.join("styles");
        fs::create_dir_all(&styles_dir).unwrap();

        // Write a Tailwind CSS file
        fs::write(styles_dir.join("globals.css"), "@import \"tailwindcss\";\n").unwrap();

        // Create pages with CSS import
        let pages_dir = root.join("pages");
        fs::create_dir_all(&pages_dir).unwrap();
        fs::write(
            pages_dir.join("_app.tsx"),
            "import '../styles/globals.css';\nexport default function App({ Component, pageProps }) { return <Component {...pageProps} />; }\n",
        )
        .unwrap();
        fs::write(
            pages_dir.join("index.tsx"),
            "export default function Home() { return <div className=\"p-4\">Hello</div>; }\n",
        )
        .unwrap();

        // Must have tailwindcss installed
        let bin = find_tailwind_bin(&root);
        if bin.is_none() {
            eprintln!("tailwindcss not found, skipping integration test");
            return;
        }

        let config = rex_core::RexConfig::new(root).with_dev(false);
        let scan = rex_router::scan_pages(&config.pages_dir).unwrap();
        let output_dir = tmp.path().join("output");
        fs::create_dir_all(&output_dir).unwrap();

        let mappings = process_tailwind_css(&config, &scan, &output_dir).unwrap();
        assert!(
            !mappings.is_empty(),
            "should have processed at least one Tailwind file"
        );

        // The output file should exist and contain actual CSS (not just the directive)
        for output in mappings.values() {
            assert!(output.exists(), "Tailwind output file should exist");
            let content = fs::read_to_string(output).unwrap();
            assert!(
                !content.contains("@import \"tailwindcss\""),
                "should be compiled"
            );
            assert!(!content.is_empty(), "compiled CSS should not be empty");
        }
    }

    #[test]
    fn test_process_tailwind_css_no_bin_errors() {
        let tmp = TempDir::new().unwrap();
        let pages_dir = tmp.path().join("pages");
        fs::create_dir_all(&pages_dir).unwrap();

        let css_file = pages_dir.join("styles.css");
        fs::write(&css_file, "@import \"tailwindcss\";\n").unwrap();

        let app = pages_dir.join("_app.tsx");
        fs::write(
            &app,
            format!(
                "import '{}';\nexport default function App() {{}}\n",
                css_file.display()
            ),
        )
        .unwrap();

        let output_dir = tmp.path().join("output");
        fs::create_dir_all(&output_dir).unwrap();

        let config = rex_core::RexConfig {
            project_root: tmp.path().to_path_buf(),
            pages_dir: pages_dir.clone(),
            app_dir: tmp.path().join("app"),
            output_dir: output_dir.clone(),
            port: 3000,
            dev: false,
        };

        let scan = rex_router::ScanResult {
            app: Some(rex_core::Route {
                pattern: "/_app".to_string(),
                file_path: app.clone(),
                abs_path: app,
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

        let result = process_tailwind_css(&config, &scan, &output_dir);
        assert!(
            result.is_err(),
            "should error when tailwindcss binary not found"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("tailwindcss is not installed") || err.contains("@tailwindcss/cli"),
            "error should mention missing binary"
        );
    }
}
