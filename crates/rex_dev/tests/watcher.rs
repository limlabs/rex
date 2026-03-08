#![allow(clippy::unwrap_used)]

use rex_dev::watcher::{start_watcher, FileEvent, FileEventKind};
use std::fs;
use std::time::Duration;
use tempfile::TempDir;

/// Helper: create a temp project with pages/ and app/ dirs, start the watcher,
/// and return (tmpdir, receiver). The tmpdir must be kept alive for the test.
///
/// Canonicalizes the temp dir path because macOS symlinks /tmp → /private/tmp,
/// which breaks path prefix checks in the watcher.
fn setup_watcher() -> (
    TempDir,
    std::path::PathBuf,
    std::sync::mpsc::Receiver<FileEvent>,
) {
    let tmp = TempDir::new().unwrap();
    // Canonicalize to resolve /tmp → /private/tmp on macOS
    let root = tmp.path().canonicalize().unwrap();
    let pages_dir = root.join("pages");
    let app_dir = root.join("app");
    fs::create_dir_all(&pages_dir).unwrap();
    fs::create_dir_all(&app_dir).unwrap();

    let rx = start_watcher(&root, &pages_dir, &app_dir).unwrap();
    // Give FSEvents time to register the watcher (macOS needs ~500ms)
    std::thread::sleep(Duration::from_millis(500));
    (tmp, root, rx)
}

/// Drain all events from the receiver with a timeout.
fn collect_events(rx: &std::sync::mpsc::Receiver<FileEvent>, timeout: Duration) -> Vec<FileEvent> {
    let mut events = Vec::new();
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match rx.recv_timeout(remaining) {
            Ok(event) => events.push(event),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    events
}

#[test]
fn pages_dir_tsx_triggers_page_modified() {
    let (_tmp, root, rx) = setup_watcher();
    let page = root.join("pages/index.tsx");
    fs::write(&page, "export default function Home() {}").unwrap();

    let events = collect_events(&rx, Duration::from_secs(3));
    assert!(
        events
            .iter()
            .any(|e| matches!(e.kind, FileEventKind::PageModified)),
        "Expected PageModified for pages/index.tsx, got: {events:?}"
    );
}

#[test]
fn app_dir_page_tsx_triggers_page_modified() {
    let (_tmp, root, rx) = setup_watcher();
    let app_page = root.join("app/page.tsx");
    fs::write(&app_page, "export default function Home() {}").unwrap();

    let events = collect_events(&rx, Duration::from_secs(3));
    assert!(
        events
            .iter()
            .any(|e| matches!(e.kind, FileEventKind::PageModified)),
        "Expected PageModified for app/page.tsx, got: {events:?}"
    );
}

#[test]
fn app_dir_nested_page_triggers_page_modified() {
    let (_tmp, root, rx) = setup_watcher();
    let nested_dir = root.join("app/dashboard");
    fs::create_dir_all(&nested_dir).unwrap();
    std::thread::sleep(Duration::from_millis(200));

    let nested_page = nested_dir.join("page.tsx");
    fs::write(&nested_page, "export default function Dashboard() {}").unwrap();

    let events = collect_events(&rx, Duration::from_secs(3));
    assert!(
        events
            .iter()
            .any(|e| matches!(e.kind, FileEventKind::PageModified)),
        "Expected PageModified for app/dashboard/page.tsx, got: {events:?}"
    );
}

#[test]
fn app_dir_layout_tsx_triggers_page_modified() {
    let (_tmp, root, rx) = setup_watcher();
    let layout = root.join("app/layout.tsx");
    fs::write(
        &layout,
        "export default function Layout({ children }) { return children; }",
    )
    .unwrap();

    let events = collect_events(&rx, Duration::from_secs(3));
    assert!(
        events
            .iter()
            .any(|e| matches!(e.kind, FileEventKind::PageModified)),
        "Expected PageModified for app/layout.tsx, got: {events:?}"
    );
}

#[test]
fn css_file_triggers_css_modified() {
    let (_tmp, root, rx) = setup_watcher();
    let css = root.join("app/globals.css");
    fs::write(&css, "body { margin: 0; }").unwrap();

    let events = collect_events(&rx, Duration::from_secs(3));
    assert!(
        events
            .iter()
            .any(|e| matches!(e.kind, FileEventKind::CssModified)),
        "Expected CssModified for app/globals.css, got: {events:?}"
    );
}

#[test]
fn node_modules_changes_are_ignored() {
    let (_tmp, root, rx) = setup_watcher();
    let nm = root.join("node_modules/react");
    fs::create_dir_all(&nm).unwrap();
    std::thread::sleep(Duration::from_millis(200));

    fs::write(nm.join("index.js"), "module.exports = {}").unwrap();

    let events = collect_events(&rx, Duration::from_secs(2));
    assert!(
        events.is_empty(),
        "Expected no events for node_modules changes, got: {events:?}"
    );
}

#[test]
fn app_dir_page_removal_triggers_page_removed() {
    let (_tmp, root, rx) = setup_watcher();
    let page = root.join("app/page.tsx");
    fs::write(&page, "export default function Home() {}").unwrap();

    // Wait for the create event to settle
    let _ = collect_events(&rx, Duration::from_secs(2));

    // Now remove the file
    fs::remove_file(&page).unwrap();

    let events = collect_events(&rx, Duration::from_secs(3));
    assert!(
        events
            .iter()
            .any(|e| matches!(e.kind, FileEventKind::PageRemoved)),
        "Expected PageRemoved for deleted app/page.tsx, got: {events:?}"
    );
}
