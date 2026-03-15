use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceInfo {
    pub pid: u32,
    pub port: u16,
    pub host: String,
    pub project_dir: PathBuf,
    pub started_at: u64,
}

/// Directory where instance files are stored: `~/.rex/instances/`.
pub fn instances_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".rex").join("instances")
}

/// Register the current process as a running Rex dev instance.
pub fn register_instance(
    port: u16,
    host: &str,
    project_dir: &std::path::Path,
) -> std::io::Result<()> {
    let dir = instances_dir();
    std::fs::create_dir_all(&dir)?;

    let info = InstanceInfo {
        pid: std::process::id(),
        port,
        host: host.to_string(),
        project_dir: project_dir.to_path_buf(),
        started_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before UNIX epoch")
            .as_secs(),
    };

    let path = dir.join(format!("{}.json", info.pid));
    let json = serde_json::to_string_pretty(&info).expect("InstanceInfo is always serializable");
    std::fs::write(path, json)
}

/// Remove the instance file for the current process.
pub fn unregister_instance() {
    let path = instances_dir().join(format!("{}.json", std::process::id()));
    let _ = std::fs::remove_file(path);
}

/// List all live Rex dev instances, pruning stale entries.
pub fn list_instances() -> Vec<InstanceInfo> {
    let dir = instances_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut instances = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let contents = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let info: InstanceInfo = match serde_json::from_str(&contents) {
            Ok(i) => i,
            Err(_) => {
                // Corrupt file — remove it
                let _ = std::fs::remove_file(&path);
                continue;
            }
        };

        if is_process_alive(info.pid) {
            instances.push(info);
        } else {
            // Stale — process no longer running
            let _ = std::fs::remove_file(&path);
        }
    }

    instances
}

/// Check if a process is still alive using `kill -0`.
#[cfg(unix)]
fn is_process_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_process_alive(_pid: u32) -> bool {
    // On non-unix, assume alive — stale entries will be cleaned up eventually.
    true
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn register_and_list() {
        // Use a unique test directory to avoid conflicts
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());

        register_instance(3000, "127.0.0.1", std::path::Path::new("/test/project")).unwrap();

        let instances = list_instances();
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].pid, std::process::id());
        assert_eq!(instances[0].port, 3000);

        unregister_instance();

        let instances = list_instances();
        assert!(instances.is_empty());

        // Restore HOME (best effort)
        std::env::remove_var("HOME");
    }
}
