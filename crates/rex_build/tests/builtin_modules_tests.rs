//! Tests for builtin_modules — covers ensure_internal_packages.
#![allow(clippy::unwrap_used)]

use rex_build::builtin_modules;
use std::fs;

#[test]
fn ensure_internal_packages_extracts_missing() {
    let tmp = tempfile::tempdir().unwrap();
    // Create a node_modules dir (simulating a project with its own deps)
    let nm = tmp.path().join("node_modules");
    fs::create_dir_all(&nm).unwrap();

    // Before: react-server-dom-webpack should not exist
    assert!(!nm.join("react-server-dom-webpack/package.json").exists());

    builtin_modules::ensure_internal_packages(tmp.path()).unwrap();

    // After: react-server-dom-webpack should be extracted
    assert!(nm.join("react-server-dom-webpack/package.json").exists());
}

#[test]
fn ensure_internal_packages_skips_existing() {
    let tmp = tempfile::tempdir().unwrap();
    let nm = tmp.path().join("node_modules");
    fs::create_dir_all(nm.join("react-server-dom-webpack")).unwrap();
    fs::write(nm.join("react-server-dom-webpack/package.json"), "{}").unwrap();

    // Should not error when package already exists
    builtin_modules::ensure_internal_packages(tmp.path()).unwrap();
    // Verify it's still there
    assert!(nm.join("react-server-dom-webpack/package.json").exists());
}

#[test]
fn ensure_builtin_modules_re_extracts_on_version_mismatch() {
    let tmp = tempfile::tempdir().unwrap();
    let nm = tmp.path().join("node_modules");
    fs::create_dir_all(&nm).unwrap();

    // Write a stale version marker
    fs::write(nm.join(".rex-builtin-version"), "0.0.0-stale").unwrap();
    // Create a fake react dir so the version check has something to find
    fs::create_dir_all(nm.join("react")).unwrap();

    let result = builtin_modules::ensure_builtin_modules(tmp.path()).unwrap();
    assert!(result.exists());

    // Version marker should be updated to current
    let version = fs::read_to_string(nm.join(".rex-builtin-version")).unwrap();
    assert_eq!(version.trim(), builtin_modules::EMBEDDED_REACT_VERSION);
}
