//! Tests for builtin_modules — covers ensure_missing_builtin_packages.
#![allow(clippy::unwrap_used)]

use rex_build::builtin_modules;
use std::fs;

#[test]
fn ensure_missing_builtin_packages_extracts_all_missing() {
    let tmp = tempfile::tempdir().unwrap();

    // Before: no node_modules at all
    assert!(!tmp.path().join("node_modules").exists());

    builtin_modules::ensure_missing_builtin_packages(tmp.path()).unwrap();

    let nm = tmp.path().join("node_modules");
    // All embedded packages should be extracted
    assert!(nm.join("react/package.json").exists());
    assert!(nm.join("react-dom/package.json").exists());
    assert!(nm.join("react-server-dom-webpack/package.json").exists());
    assert!(nm.join("scheduler/package.json").exists());
    assert!(nm.join("@types/react/package.json").exists());
    assert!(nm.join("@types/react-dom/package.json").exists());
    assert!(nm.join("@limlabs/rex/package.json").exists());
}

#[test]
fn ensure_missing_builtin_packages_skips_existing() {
    let tmp = tempfile::tempdir().unwrap();
    let nm = tmp.path().join("node_modules");

    // Simulate user-installed react
    fs::create_dir_all(nm.join("react")).unwrap();
    fs::write(
        nm.join("react/package.json"),
        r#"{"name":"react","version":"18.0.0"}"#,
    )
    .unwrap();

    builtin_modules::ensure_missing_builtin_packages(tmp.path()).unwrap();

    // User's react version should be preserved
    let content = fs::read_to_string(nm.join("react/package.json")).unwrap();
    assert!(content.contains("18.0.0"));

    // But missing packages should be extracted
    assert!(nm.join("react-dom/package.json").exists());
    assert!(nm.join("scheduler/package.json").exists());
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
