//! E2E tests for `rex init` — verifies that a freshly-initialized project
//! builds and serves without errors (e.g. "React is not defined").
//!
//! Run with: cargo test -p rex_e2e --test init_e2e -- --ignored
//!
//! Prerequisites:
//!   - `cargo build` (debug or release)

#[allow(clippy::unwrap_used)]
mod init {
    use std::io::{Read, Write};
    use std::net::TcpStream;
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

        panic!(
            "Rex binary not found. Run `cargo build` or `cargo build --release` first.\n\
             Or set REX_BIN=/path/to/rex"
        );
    }

    fn find_free_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    }

    #[tokio::test]
    #[ignore]
    async fn e2e_init_project_starts_without_error() {
        let bin = rex_binary();

        // Create a temp directory for the test project
        let tmp = std::env::temp_dir().join(format!("rex-init-e2e-{}", std::process::id()));
        if tmp.exists() {
            std::fs::remove_dir_all(&tmp).unwrap();
        }
        std::fs::create_dir_all(&tmp).unwrap();

        let project_name = "test-project";
        let project_dir = tmp.join(project_name);

        // Run `rex init test-project` inside the temp directory
        let init_status = Command::new(&bin)
            .arg("init")
            .arg(project_name)
            .current_dir(&tmp)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .status()
            .unwrap();

        assert!(
            init_status.success(),
            "rex init should exit successfully"
        );
        assert!(
            project_dir.join("pages/index.tsx").exists(),
            "rex init should create pages/index.tsx"
        );
        assert!(
            project_dir.join("tsconfig.json").exists(),
            "rex init should create tsconfig.json"
        );

        // Start `rex dev` in the initialized project
        let port = find_free_port();
        let mut child = Command::new(&bin)
            .arg("dev")
            .arg("--root")
            .arg(&project_dir)
            .arg("--port")
            .arg(port.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        // Wait for server to be HTTP-ready (polls until we get an HTTP response)
        let deadline = Instant::now() + Duration::from_secs(60);
        let addr = format!("127.0.0.1:{port}");
        let mut server_ready = false;
        loop {
            if Instant::now() > deadline {
                break;
            }
            if let Ok(mut stream) =
                TcpStream::connect_timeout(&addr.parse().unwrap(), Duration::from_millis(500))
            {
                stream
                    .set_read_timeout(Some(Duration::from_millis(500)))
                    .ok();
                let req = format!("GET / HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n");
                if stream.write_all(req.as_bytes()).is_ok() {
                    let mut buf = [0u8; 32];
                    if let Ok(n) = stream.read(&mut buf) {
                        if n > 0 && String::from_utf8_lossy(&buf[..n]).contains("HTTP/") {
                            server_ready = true;
                            break;
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(200));
        }

        if !server_ready {
            // Collect stderr for diagnostics before killing
            #[cfg(unix)]
            #[allow(unsafe_code)]
            unsafe {
                libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
            }
            #[cfg(not(unix))]
            {
                child.kill().ok();
            }
            let output = child.wait_with_output().unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            let _ = std::fs::remove_dir_all(&tmp);
            panic!("Server failed to start within 60s.\nstderr:\n{stderr}");
        }

        // Fetch the index page and verify it returns 200 with SSR content
        let url = format!("http://127.0.0.1:{port}/");
        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), 200, "Index page should return 200");

        let body = resp.text().await.unwrap();
        assert!(
            body.contains("Welcome to Rex"),
            "Index page should contain SSR-rendered 'Welcome to Rex', got:\n{body}"
        );
        assert!(
            body.contains("<div id=\"__rex\">"),
            "Index page should contain __rex root div"
        );

        // Shut down the server
        #[cfg(unix)]
        #[allow(unsafe_code)]
        unsafe {
            libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
        }
        #[cfg(not(unix))]
        {
            child.kill().ok();
        }
        let _ = child.wait();

        // Clean up
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
