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

/// Collect all CSS import paths from _app and pages (reusing extract_css_imports).
pub fn collect_all_css_import_paths(scan: &ScanResult) -> Result<Vec<PathBuf>> {
    let mut all = Vec::new();
    if let Some(app) = &scan.app {
        all.extend(extract_css_imports(&app.abs_path)?);
    }
    for route in &scan.routes {
        all.extend(extract_css_imports(&route.abs_path)?);
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
                 Install it: npm install tailwindcss",
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
