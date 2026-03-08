#![allow(clippy::unwrap_used)]

mod common;

use common::{make_server_bundle, MOCK_REACT_RUNTIME};
use rex_v8::SsrIsolate;

fn make_fs_isolate(project_root: &std::path::Path, gssp_code: &str) -> SsrIsolate {
    rex_v8::init_v8();
    let pages = &[(
        "index",
        "function Index(props) { return React.createElement('div', null, JSON.stringify(props)); }",
        Some(gssp_code),
    )];
    let bundle = format!("{}\n{}", MOCK_REACT_RUNTIME, make_server_bundle(pages));
    let root_str = project_root.to_string_lossy().to_string();
    SsrIsolate::new(&bundle, Some(&root_str)).expect("failed to create fs isolate")
}

#[test]
fn test_fs_read_file_sync_utf8() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    std::fs::write(root.join("data.txt"), "hello from file").unwrap();

    let gssp = r#"function gssp(ctx) {
        var content = globalThis.__rex_fs_read_file_sync(globalThis.__rex_project_root, 'data.txt', 'utf8');
        return { props: { content: content } };
    }"#;
    let mut iso = make_fs_isolate(&root, gssp);

    let result = iso.get_server_side_props("index", "{}").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(
        parsed["props"]["content"].as_str(),
        Some("hello from file"),
        "Should read file content: {result}"
    );
}

#[test]
fn test_fs_path_traversal_blocked() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();

    let gssp = r#"function gssp(ctx) {
        var result = globalThis.__rex_fs_read_file_sync(globalThis.__rex_project_root, '../../etc/passwd', 'utf8');
        if (typeof result === 'string' && result.indexOf('__REX_FS_ERR__') === 0) {
            var err = JSON.parse(result.slice(14));
            return { props: { error: err.code } };
        }
        return { props: { content: result } };
    }"#;
    let mut iso = make_fs_isolate(&root, gssp);

    let result = iso.get_server_side_props("index", "{}").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(
        parsed["props"]["error"].as_str(),
        Some("EACCES"),
        "Should block traversal: {result}"
    );
}

#[test]
fn test_fs_write_and_read_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();

    let gssp = r#"function gssp(ctx) {
        globalThis.__rex_fs_write_file_sync(globalThis.__rex_project_root, 'out.txt', 'round trip data');
        var content = globalThis.__rex_fs_read_file_sync(globalThis.__rex_project_root, 'out.txt', 'utf8');
        return { props: { content: content } };
    }"#;
    let mut iso = make_fs_isolate(&root, gssp);

    let result = iso.get_server_side_props("index", "{}").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(
        parsed["props"]["content"].as_str(),
        Some("round trip data"),
        "Should write and read back: {result}"
    );
}

#[test]
fn test_fs_exists_sync() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    std::fs::write(root.join("exists.txt"), "yes").unwrap();

    let gssp = r#"function gssp(ctx) {
        var yes = globalThis.__rex_fs_exists_sync(globalThis.__rex_project_root, 'exists.txt');
        var no = globalThis.__rex_fs_exists_sync(globalThis.__rex_project_root, 'nope.txt');
        return { props: { exists: yes, missing: no } };
    }"#;
    let mut iso = make_fs_isolate(&root, gssp);

    let result = iso.get_server_side_props("index", "{}").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["props"]["exists"], true);
    assert_eq!(parsed["props"]["missing"], false);
}

#[test]
fn test_fs_readdir_sync() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    std::fs::write(root.join("a.txt"), "").unwrap();
    std::fs::write(root.join("b.txt"), "").unwrap();

    let gssp = r#"function gssp(ctx) {
        var json = globalThis.__rex_fs_readdir_sync(globalThis.__rex_project_root, '.');
        var entries = JSON.parse(json);
        entries.sort();
        return { props: { entries: entries } };
    }"#;
    let mut iso = make_fs_isolate(&root, gssp);

    let result = iso.get_server_side_props("index", "{}").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let entries = parsed["props"]["entries"].as_array().unwrap();
    assert!(
        entries.iter().any(|e| e.as_str() == Some("a.txt")),
        "Should list a.txt: {result}"
    );
    assert!(
        entries.iter().any(|e| e.as_str() == Some("b.txt")),
        "Should list b.txt: {result}"
    );
}

#[test]
fn test_fs_stat_sync() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    std::fs::write(root.join("stat_test.txt"), "hello world").unwrap();
    std::fs::create_dir(root.join("subdir")).unwrap();

    let gssp = r#"function gssp(ctx) {
        var fileJson = globalThis.__rex_fs_stat_sync(globalThis.__rex_project_root, 'stat_test.txt');
        var fileStat = JSON.parse(fileJson);
        var dirJson = globalThis.__rex_fs_stat_sync(globalThis.__rex_project_root, 'subdir');
        var dirStat = JSON.parse(dirJson);
        return { props: { fileIsFile: fileStat.isFile, fileSize: fileStat.size, dirIsDir: dirStat.isDirectory } };
    }"#;
    let mut iso = make_fs_isolate(&root, gssp);

    let result = iso.get_server_side_props("index", "{}").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["props"]["fileIsFile"], true);
    assert_eq!(parsed["props"]["fileSize"], 11);
    assert_eq!(parsed["props"]["dirIsDir"], true);
}

#[test]
fn test_fs_mkdir_recursive() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();

    let gssp = r#"function gssp(ctx) {
        globalThis.__rex_fs_mkdir_sync(globalThis.__rex_project_root, 'a/b/c', { recursive: true });
        var exists = globalThis.__rex_fs_exists_sync(globalThis.__rex_project_root, 'a/b/c');
        return { props: { created: exists } };
    }"#;
    let mut iso = make_fs_isolate(&root, gssp);

    let result = iso.get_server_side_props("index", "{}").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["props"]["created"], true);
}

#[test]
fn test_fs_rm_sync() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    std::fs::write(root.join("to_delete.txt"), "bye").unwrap();
    std::fs::create_dir_all(root.join("rmdir/sub")).unwrap();
    std::fs::write(root.join("rmdir/sub/file.txt"), "nested").unwrap();

    let gssp = r#"function gssp(ctx) {
        globalThis.__rex_fs_unlink_sync(globalThis.__rex_project_root, 'to_delete.txt');
        var fileGone = !globalThis.__rex_fs_exists_sync(globalThis.__rex_project_root, 'to_delete.txt');
        globalThis.__rex_fs_rm_sync(globalThis.__rex_project_root, 'rmdir', { recursive: true });
        var dirGone = !globalThis.__rex_fs_exists_sync(globalThis.__rex_project_root, 'rmdir');
        return { props: { fileGone: fileGone, dirGone: dirGone } };
    }"#;
    let mut iso = make_fs_isolate(&root, gssp);

    let result = iso.get_server_side_props("index", "{}").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["props"]["fileGone"], true);
    assert_eq!(parsed["props"]["dirGone"], true);
}
