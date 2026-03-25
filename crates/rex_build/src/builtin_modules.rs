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

/// Ensures all embedded packages are available in the project's node_modules,
/// extracting any that are missing. Unlike `ensure_builtin_modules`, this does
/// NOT wipe existing node_modules — it only adds packages the user hasn't
/// installed themselves. This means projects with a `package.json` don't need
/// to `npm install` react, react-dom, etc. — Rex provides them automatically.
pub fn ensure_missing_builtin_packages(project_root: &Path) -> Result<()> {
    let node_modules_dir = project_root.join("node_modules");
    fs::create_dir_all(&node_modules_dir)?;

    for pkg_path in all_embedded_packages() {
        let pkg_dir = node_modules_dir.join(&pkg_path);
        if pkg_dir.join("package.json").exists() {
            continue;
        }
        // Extract from embedded vendor directory
        if let Some(embedded) = VENDOR_NODE_MODULES.get_dir(&pkg_path) {
            info!(
                "Extracting built-in {pkg_path} to {}",
                node_modules_dir.display()
            );
            fs::create_dir_all(&pkg_dir)?;
            extract_dir(embedded, &node_modules_dir)?;
        }
    }
    Ok(())
}

/// Returns all package paths in the embedded vendor directory.
/// Handles scoped packages (e.g. `@types/react`) by descending one level
/// into `@`-prefixed directories.
fn all_embedded_packages() -> Vec<String> {
    let mut packages = Vec::new();
    for dir in VENDOR_NODE_MODULES.dirs() {
        let name = dir.path().to_string_lossy().to_string();
        if name.starts_with('@') {
            // Scoped package scope dir — iterate sub-packages
            for subdir in dir.dirs() {
                packages.push(subdir.path().to_string_lossy().to_string());
            }
        } else {
            packages.push(name);
        }
    }
    packages
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

        // Should contain @limlabs/rex source for rex/* path alias
        assert!(nm.join("@limlabs/rex/package.json").exists());
        assert!(nm.join("@limlabs/rex/src/link.tsx").exists());
        assert!(nm.join("@limlabs/rex/src/head.tsx").exists());

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

    #[test]
    fn test_ensure_missing_builtin_packages_extracts_all() {
        let tmp = tempfile::tempdir().unwrap();

        // Project with package.json but no node_modules
        fs::write(tmp.path().join("package.json"), "{}").unwrap();

        ensure_missing_builtin_packages(tmp.path()).unwrap();

        let nm = tmp.path().join("node_modules");
        assert!(nm.join("react/package.json").exists());
        assert!(nm.join("react-dom/package.json").exists());
        assert!(nm.join("scheduler/package.json").exists());
        assert!(nm.join("react-server-dom-webpack/package.json").exists());
        assert!(nm.join("@types/react/package.json").exists());
        assert!(nm.join("@types/react-dom/package.json").exists());
        assert!(nm.join("@limlabs/rex/package.json").exists());
    }

    #[test]
    fn test_ensure_missing_builtin_packages_preserves_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let nm = tmp.path().join("node_modules");

        // Simulate user-installed react with custom content
        let react_dir = nm.join("react");
        fs::create_dir_all(&react_dir).unwrap();
        fs::write(
            react_dir.join("package.json"),
            r#"{"name":"react","version":"18.0.0"}"#,
        )
        .unwrap();

        ensure_missing_builtin_packages(tmp.path()).unwrap();

        // User's react should be untouched
        let pkg: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(react_dir.join("package.json")).unwrap())
                .unwrap();
        assert_eq!(pkg["version"], "18.0.0");

        // But missing packages should be extracted
        assert!(nm.join("react-dom/package.json").exists());
        assert!(nm.join("scheduler/package.json").exists());
    }
}
