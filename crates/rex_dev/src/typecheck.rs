use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use crate::hmr::HmrBroadcast;

/// Find the `tsc` binary, checking local node_modules first, then PATH.
pub fn find_tsc(root: &Path) -> Option<PathBuf> {
    let local = root.join("node_modules/.bin/tsc");
    if local.exists() {
        return Some(local);
    }

    which("tsc")
}

/// Spawn a `tsc --watch --noEmit` process if TypeScript is detected.
pub fn spawn_tsc_watcher(root: &Path, _hmr: HmrBroadcast) -> Option<Child> {
    let tsc = find_tsc(root)?;

    // Only watch if tsconfig.json exists
    if !root.join("tsconfig.json").exists() {
        return None;
    }

    Command::new(&tsc)
        .current_dir(root)
        .args(["--watch", "--noEmit", "--pretty"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()
}

fn which(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|p| p.join(name))
            .find(|p| p.exists())
    })
}
