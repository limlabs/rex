//! E2E test: verify ESM fast path is used for HMR source changes.
//!
//! Starts a dedicated `rex dev` server against `fixtures/app-router`,
//! modifies a component file, and checks that:
//! 1. The server log contains "ESM fast path rebuild" (not "Rebuild complete")
//! 2. The page still renders correctly after the change

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn rex_binary() -> PathBuf {
    if let Ok(bin) = std::env::var("REX_BIN") {
        return PathBuf::from(bin);
    }
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let debug = workspace_root.join("target/debug/rex");
    if debug.exists() {
        return debug;
    }
    let release = workspace_root.join("target/release/rex");
    if release.exists() {
        return release;
    }
    panic!("Rex binary not found. Run `cargo build` first.");
}

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("fixtures/app-router")
}

fn find_free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// Wait until the server accepts TCP connections (port is open).
/// Does NOT wait for an HTTP response — lazy init may take a long time in CI.
fn wait_for_server(port: u16, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    let addr: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    loop {
        if Instant::now() > deadline {
            panic!("Server failed to start within {timeout:?} on port {port}");
        }
        if std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[tokio::test]
#[ignore]
async fn e2e_hmr_esm_fast_path_for_source_change() {
    let bin = rex_binary();
    let root = fixture_root();
    let port = find_free_port();
    let component_path = root.join("components/Counter.tsx");

    // Save original content for restoration
    let original = std::fs::read_to_string(&component_path)
        .expect("Failed to read Counter.tsx — run `cd fixtures/app-router && npm install` first");

    // Start server with RUST_LOG capturing info-level logs
    let mut child = Command::new(&bin)
        .arg("dev")
        .arg("--root")
        .arg(&root)
        .arg("--port")
        .arg(port.to_string())
        .env("RUST_LOG", "rex=info")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to start rex");

    // Wait for server to be ready
    wait_for_server(port, Duration::from_secs(30));

    // Make initial request to trigger lazy init (blocks until build + ESM complete).
    // Use a long timeout — in CI the app-router RSC build can take 10-30s.
    let url = format!("http://127.0.0.1:{port}/");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap();
    let resp = client.get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 200, "Initial page load should return 200");

    // Wait for any in-flight rebuilds from the initial build to settle.
    // The watcher may fire events from build output before the ESM state is ready.
    std::thread::sleep(Duration::from_secs(2));

    // Drain stderr accumulated so far (initial build logs) so we only check
    // logs produced AFTER the test file modification.
    let stderr_handle = child.stderr.take().expect("stderr should be piped");
    let (log_tx, log_rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        use std::io::BufRead;
        let reader = std::io::BufReader::new(stderr_handle);
        for line in reader.lines().map_while(Result::ok) {
            let _ = log_tx.send(line);
        }
    });
    // Drain pre-modification logs
    while log_rx.try_recv().is_ok() {}

    // Touch the component file with a whitespace change
    let modified = format!("{original}\n// hmr-test-marker\n");
    std::fs::write(&component_path, &modified).expect("Failed to write Counter.tsx");

    // Wait for the rebuild to happen
    std::thread::sleep(Duration::from_secs(3));

    // Make another request to verify the page still works
    let resp2 = reqwest::get(&url).await.unwrap();
    assert_eq!(
        resp2.status(),
        200,
        "Page should still return 200 after HMR"
    );

    // Collect only post-modification logs
    let mut post_mod_lines = Vec::new();
    while let Ok(line) = log_rx.try_recv() {
        post_mod_lines.push(line);
    }

    // Kill the server
    child.kill().ok();
    child.wait().ok();

    // Restore original file
    std::fs::write(&component_path, &original).expect("Failed to restore Counter.tsx");

    // Check logs from AFTER the file modification only
    let post_mod_output = post_mod_lines.join("\n");
    let used_fast_path = post_mod_output.contains("ESM fast path rebuild");
    let used_full_rebuild = post_mod_output.contains("Rebuild complete");

    if !used_fast_path {
        // Print diagnostics
        let relevant_lines: Vec<&str> = post_mod_output
            .lines()
            .filter(|l| {
                l.contains("ESM")
                    || l.contains("fast path")
                    || l.contains("Rebuild")
                    || l.contains("fallback")
                    || l.contains("not in ESM")
                    || l.contains("File not in")
            })
            .collect();
        eprintln!("=== Relevant server log lines ===");
        for line in &relevant_lines {
            eprintln!("  {line}");
        }
        eprintln!("=================================");
    }

    assert!(
        used_fast_path,
        "Expected ESM fast path to handle the source change, but it wasn't used. \
         Full rebuild used: {used_full_rebuild}. Check server logs above for details."
    );

    assert!(
        !used_full_rebuild,
        "ESM fast path was used but a full rebuild also ran — something is wrong"
    );
}
