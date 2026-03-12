use anyhow::Result;
use include_dir::{include_dir, Dir};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::info;

/// React version embedded in the binary.
pub const EMBEDDED_REACT_VERSION: &str = "19.2.4";

/// Embedded node_modules/ directory, downloaded by build.rs and baked into the binary.
static VENDOR_NODE_MODULES: Dir<'_> = include_dir!("$OUT_DIR/node_modules");

/// Returns `true` if a `package.json` exists in the project root.
pub fn has_package_json(project_root: &Path) -> bool {
    project_root.join("package.json").exists()
}

/// Extracts the embedded React packages to `node_modules/` under the project
/// root so that both the bundler and the IDE/TypeScript language server can
/// resolve them via normal `node_modules` lookup.
///
/// Uses a version marker file to skip re-extraction when the embedded version
/// matches what's already on disk.
pub fn ensure_builtin_modules(project_root: &Path) -> Result<PathBuf> {
    let node_modules_dir = project_root.join("node_modules");
    let version_file = node_modules_dir.join(".rex-builtin-version");

    // Skip extraction if version matches and react packages actually exist
    if version_file.exists() && node_modules_dir.join("react").exists() {
        if let Ok(existing) = fs::read_to_string(&version_file) {
            if existing.trim() == EMBEDDED_REACT_VERSION {
                return Ok(node_modules_dir);
            }
        }
    }

    info!(
        "Extracting built-in React {} to {}",
        EMBEDDED_REACT_VERSION,
        node_modules_dir.display()
    );

    // Clean and recreate
    if node_modules_dir.exists() {
        fs::remove_dir_all(&node_modules_dir)?;
    }
    fs::create_dir_all(&node_modules_dir)?;

    // Extract all files from the embedded directory
    extract_dir(&VENDOR_NODE_MODULES, &node_modules_dir)?;

    // Write version marker
    fs::write(&version_file, EMBEDDED_REACT_VERSION)?;

    Ok(node_modules_dir)
}

/// Packages that Rex always provides, even when the project has its own
/// package.json. These are internal deps needed by Rex's RSC runtime.
const REX_INTERNAL_PACKAGES: &[&str] = &["react-server-dom-webpack"];

/// Ensures Rex-internal packages are available in the project's node_modules,
/// extracting them from the embedded binary if missing. Unlike
/// `ensure_builtin_modules`, this does NOT wipe existing node_modules — it only
/// adds missing packages.
pub fn ensure_internal_packages(project_root: &Path) -> Result<()> {
    let node_modules_dir = project_root.join("node_modules");
    for &pkg in REX_INTERNAL_PACKAGES {
        let pkg_dir = node_modules_dir.join(pkg);
        if pkg_dir.join("package.json").exists() {
            continue;
        }
        // Extract from embedded vendor directory
        if let Some(embedded) = VENDOR_NODE_MODULES.get_dir(pkg) {
            info!(
                "Extracting built-in {pkg} to {}",
                node_modules_dir.display()
            );
            fs::create_dir_all(&pkg_dir)?;
            extract_dir(embedded, &node_modules_dir)?;
        }
    }
    Ok(())
}

/// Recursively extracts an embedded directory to a filesystem path.
fn extract_dir(dir: &Dir<'_>, dest: &Path) -> Result<()> {
    for file in dir.files() {
        let file_path = dest.join(file.path());
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&file_path, file.contents())?;
    }

    for subdir in dir.dirs() {
        extract_dir(subdir, dest)?;
    }

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_has_package_json() {
        let tmp = tempfile::tempdir().unwrap();

        // No package.json
        assert!(!has_package_json(tmp.path()));

        // With package.json
        fs::write(tmp.path().join("package.json"), "{}").unwrap();
        assert!(has_package_json(tmp.path()));
    }

    #[test]
    fn test_ensure_builtin_modules_extracts() {
        let tmp = tempfile::tempdir().unwrap();
        let nm = ensure_builtin_modules(tmp.path()).unwrap();

        // Should have created node_modules dir
        assert!(nm.exists());

        // Should contain react/package.json
        assert!(nm.join("react/package.json").exists());
        assert!(nm.join("react-dom/package.json").exists());
        assert!(nm.join("react-server-dom-webpack/package.json").exists());
        assert!(nm.join("scheduler/package.json").exists());

        // Should contain @types for IDE/TypeScript support
        assert!(nm.join("@types/react/package.json").exists());
        assert!(nm.join("@types/react-dom/package.json").exists());

        // Should have version marker
        let version_file = tmp.path().join("node_modules/.rex-builtin-version");
        assert_eq!(
            fs::read_to_string(version_file).unwrap().trim(),
            EMBEDDED_REACT_VERSION
        );
    }

    #[test]
    fn test_ensure_builtin_modules_skips_on_match() {
        let tmp = tempfile::tempdir().unwrap();

        // First extraction
        let nm1 = ensure_builtin_modules(tmp.path()).unwrap();
        let react_pkg = nm1.join("react/package.json");
        let first_modified = fs::metadata(&react_pkg).unwrap().modified().unwrap();

        // Small delay to ensure filesystem timestamp would differ
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Second extraction — should skip
        let nm2 = ensure_builtin_modules(tmp.path()).unwrap();
        let second_modified = fs::metadata(nm2.join("react/package.json"))
            .unwrap()
            .modified()
            .unwrap();

        assert_eq!(first_modified, second_modified);
    }
}
