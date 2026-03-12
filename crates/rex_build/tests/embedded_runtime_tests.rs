#![allow(clippy::unwrap_used)]

use rex_build::embedded_runtime;

#[test]
fn extract_creates_runtime_directories() {
    let base = embedded_runtime::extract().unwrap();
    assert!(base.join("server").exists());
    assert!(base.join("client").exists());
}

#[test]
fn extract_creates_server_files() {
    let base = embedded_runtime::extract().unwrap();
    let server = base.join("server");
    assert!(server.join("head.ts").exists());
    assert!(server.join("link.ts").exists());
    assert!(server.join("router.ts").exists());
    assert!(server.join("document.ts").exists());
    assert!(server.join("image.ts").exists());
    assert!(server.join("middleware.ts").exists());
    assert!(server.join("fs.ts").exists());
    assert!(server.join("fs-promises.ts").exists());
    assert!(server.join("path.ts").exists());
}

#[test]
fn extract_creates_client_files() {
    let base = embedded_runtime::extract().unwrap();
    let client = base.join("client");
    assert!(client.join("link.ts").exists());
    assert!(client.join("head.ts").exists());
    assert!(client.join("use-router.ts").exists());
    assert!(client.join("image.ts").exists());
}

#[test]
fn server_dir_returns_valid_path() {
    let dir = embedded_runtime::server_dir().unwrap();
    assert!(dir.exists());
    assert!(dir.join("head.ts").exists());
}

#[test]
fn client_dir_returns_valid_path() {
    let dir = embedded_runtime::client_dir().unwrap();
    assert!(dir.exists());
    assert!(dir.join("link.ts").exists());
}

#[test]
fn extract_is_idempotent() {
    let first = embedded_runtime::extract().unwrap();
    let second = embedded_runtime::extract().unwrap();
    assert_eq!(first, second);
}
