#![allow(clippy::unwrap_used)]

use rex_v8::fs::{io_error_to_node_code, resolve_sandboxed_path};
use std::os::unix::fs::symlink;
use tempfile::tempdir;

#[test]
fn test_sandbox_relative_stays_inside() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    std::fs::write(root.join("file.txt"), "hello").unwrap();

    let result = resolve_sandboxed_path(&root, "file.txt");
    assert!(result.is_ok());
    assert!(result.unwrap().starts_with(&root));
}

#[test]
fn test_sandbox_traversal_blocked() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();

    let result = resolve_sandboxed_path(&root, "../../etc/passwd");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("EACCES"));
}

#[test]
fn test_sandbox_absolute_outside_blocked() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();

    let result = resolve_sandboxed_path(&root, "/etc/passwd");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("EACCES"));
}

#[test]
fn test_sandbox_symlink_outside_blocked() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();

    let link_path = root.join("escape");
    symlink("/tmp", &link_path).unwrap();

    let result = resolve_sandboxed_path(&root, "escape");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("EACCES"));
}

#[test]
fn test_sandbox_nonexistent_write_target() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();

    let result = resolve_sandboxed_path(&root, "subdir/newfile.txt");
    assert!(result.is_ok());
    assert!(result.unwrap().starts_with(&root));
}

#[test]
fn test_error_code_mapping() {
    assert_eq!(
        io_error_to_node_code(&std::io::Error::new(std::io::ErrorKind::NotFound, "")),
        "ENOENT"
    );
    assert_eq!(
        io_error_to_node_code(&std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            ""
        )),
        "EACCES"
    );
    assert_eq!(
        io_error_to_node_code(&std::io::Error::new(std::io::ErrorKind::AlreadyExists, "")),
        "EEXIST"
    );
}
