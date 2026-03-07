use super::*;

/// Create an isolate with fs callbacks enabled and a GSSP that exercises them.
fn make_fs_isolate(project_root: &std::path::Path, gssp_code: &str) -> SsrIsolate {
    crate::init_v8();
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
    assert_eq!(parsed["props"]["fileSize"], 11); // "hello world"
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

#[test]
fn test_process_env_from_rust() {
    // Set a known env var so we can verify it appears in V8
    std::env::set_var("REX_TEST_POLYFILL", "hello_from_rust");

    let mut iso = make_isolate(&[(
        "envtest",
        "function EnvTest() { return React.createElement('p', null, process.env.REX_TEST_POLYFILL || 'MISSING'); }",
        Some("function(ctx) { return { props: { val: process.env.REX_TEST_POLYFILL } }; }"),
    )]);

    // Verify GSSP can read process.env
    let gssp_result = iso.get_server_side_props("envtest", "{}").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&gssp_result).unwrap();
    assert_eq!(parsed["props"]["val"], "hello_from_rust");

    // Verify SSR render can read process.env
    let render = iso.render_page("envtest", "{}").unwrap();
    assert!(
        render.body.contains("hello_from_rust"),
        "SSR body should contain env var value, got: {}",
        render.body
    );

    // Clean up
    std::env::remove_var("REX_TEST_POLYFILL");
}

#[test]
fn test_process_env_is_writable() {
    // Node.js allows assigning to process.env; verify we match that behavior
    let mut iso = make_isolate(&[(
        "writetest",
        "function WriteTest() { process.env.DYNAMIC = 'set_at_runtime'; return React.createElement('p', null, process.env.DYNAMIC); }",
        None,
    )]);
    let render = iso.render_page("writetest", "{}").unwrap();
    assert!(
        render.body.contains("set_at_runtime"),
        "process.env should be writable, got: {}",
        render.body
    );
}

#[test]
fn test_console_log_emits_tracing_event() {
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::layer::SubscriberExt;

    /// Minimal tracing layer that captures log messages.
    struct CaptureLayer {
        messages: Arc<Mutex<Vec<(tracing::Level, String)>>>,
    }

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CaptureLayer {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            struct Visitor(String);
            impl tracing::field::Visit for Visitor {
                fn record_debug(
                    &mut self,
                    field: &tracing::field::Field,
                    value: &dyn std::fmt::Debug,
                ) {
                    if field.name() == "message" {
                        self.0 = format!("{value:?}");
                    }
                }
            }
            let mut visitor = Visitor(String::new());
            event.record(&mut visitor);
            self.messages
                .lock()
                .unwrap()
                .push((*event.metadata().level(), visitor.0));
        }
    }

    let messages = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new("v8::console=info"))
        .with(CaptureLayer {
            messages: messages.clone(),
        });

    let _guard = tracing::subscriber::set_default(subscriber);

    let mut iso = make_isolate(&[(
        "logpage",
        r#"function LogPage() {
            console.log("hello from ssr");
            console.warn("warning from ssr");
            console.error("error from ssr");
            return React.createElement('p', null, 'logged');
        }"#,
        None,
    )]);
    let render = iso.render_page("logpage", "{}").unwrap();
    assert!(render.body.contains("logged"), "page should render");

    let captured = messages.lock().unwrap();
    assert!(
        captured
            .iter()
            .any(|(_, msg)| msg.contains("hello from ssr")),
        "console.log should emit tracing event, captured: {captured:?}"
    );
    assert!(
        captured
            .iter()
            .any(|(level, msg)| *level == tracing::Level::WARN && msg.contains("warning from ssr")),
        "console.warn should emit WARN-level event, captured: {captured:?}"
    );
    assert!(
        captured
            .iter()
            .any(|(level, msg)| *level == tracing::Level::ERROR && msg.contains("error from ssr")),
        "console.error should emit ERROR-level event, captured: {captured:?}"
    );
}
