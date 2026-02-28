use anyhow::Result;
use rex_build::{collect_all_css_import_paths, find_tailwind_bin, needs_tailwind};
use rex_core::RexConfig;
use rex_router::ScanResult;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use tracing::info;

/// Long-lived Tailwind CSS watch process for dev mode.
///
/// Starts `tailwindcss --watch` as a child process. The process watches source files
/// and incrementally rebuilds CSS when changes are detected (~5ms).
/// Killed automatically via `Drop`.
pub struct TailwindProcess {
    child: Child,
    /// Map of original CSS input path → compiled output path.
    pub mappings: HashMap<PathBuf, PathBuf>,
}

impl TailwindProcess {
    /// Detect Tailwind CSS files, run an initial one-shot compile, then start `--watch`.
    /// Returns `None` if no Tailwind CSS files are found.
    pub fn start(config: &RexConfig, scan: &ScanResult) -> Result<Option<Self>> {
        let all_css = collect_all_css_import_paths(scan)?;
        let tw_bin = match find_tailwind_bin(&config.project_root) {
            Some(bin) => bin,
            None => return Ok(None),
        };

        // Find CSS files that need Tailwind processing
        let mut tw_files: Vec<PathBuf> = Vec::new();
        for css_path in &all_css {
            if !css_path.exists() {
                continue;
            }
            let content = std::fs::read_to_string(css_path)?;
            if needs_tailwind(&content) {
                tw_files.push(css_path.clone());
            }
        }

        if tw_files.is_empty() {
            return Ok(None);
        }

        let output_dir = config.client_build_dir();
        std::fs::create_dir_all(&output_dir)?;

        let mut mappings = HashMap::new();

        // For each Tailwind CSS file: one-shot build, then start --watch
        // In practice there's usually one (globals.css), so we only watch the first
        // and track all mappings.
        let mut watch_child: Option<Child> = None;

        for css_path in &tw_files {
            let stem = css_path.file_stem().unwrap_or_default().to_string_lossy();
            let tw_output = output_dir.join(format!("{stem}.tailwind.css"));

            // One-shot initial build
            let status = Command::new(&tw_bin)
                .arg("-i")
                .arg(css_path)
                .arg("-o")
                .arg(&tw_output)
                .arg("--minify")
                .current_dir(&config.project_root)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()?;

            if !status.success() {
                anyhow::bail!(
                    "Initial tailwindcss build failed for {}",
                    css_path.display()
                );
            }

            mappings.insert(css_path.clone(), tw_output.clone());

            // Start --watch for the first file (Tailwind watches all content sources)
            if watch_child.is_none() {
                let child = Command::new(&tw_bin)
                    .arg("-i")
                    .arg(css_path)
                    .arg("-o")
                    .arg(&tw_output)
                    .arg("--watch")
                    .arg("--minify")
                    .current_dir(&config.project_root)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()?;
                watch_child = Some(child);
            }
        }

        let child = watch_child.expect("should have at least one Tailwind file");
        info!(files = tw_files.len(), "Tailwind CSS (watching)");

        Ok(Some(TailwindProcess { child, mappings }))
    }
}

impl Drop for TailwindProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
