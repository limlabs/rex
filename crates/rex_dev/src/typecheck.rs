use crate::hmr::{HmrBroadcast, TscDiagnostic};
use std::io::BufRead;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use tracing::debug;

/// Long-lived `tsc --watch` process for dev mode.
/// Killed automatically via `Drop`.
pub struct TscProcess {
    child: Child,
}

impl Drop for TscProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Find tsc binary: local node_modules first, then PATH.
pub fn find_tsc(root: &Path) -> Option<std::path::PathBuf> {
    let local = root.join("node_modules/.bin/tsc");
    if local.exists() {
        return Some(local);
    }

    if Command::new("tsc")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
    {
        return Some(std::path::PathBuf::from("tsc"));
    }

    None
}

/// Parse a tsc diagnostic line: `file(line,col): error TSxxxx: message`
fn parse_tsc_diagnostic(line: &str) -> Option<TscDiagnostic> {
    // Format: path/to/file.ts(10,5): error TS2322: Type '...' is not assignable
    let paren = line.find('(')?;
    let close_paren = line[paren..].find(')')? + paren;
    let file = line[..paren].to_string();

    let coords = &line[paren + 1..close_paren];
    let mut parts = coords.split(',');
    let line_num: u32 = parts.next()?.parse().ok()?;
    let col: u32 = parts.next()?.parse().ok()?;

    // After "): " comes "error TSxxxx: message"
    let rest = line.get(close_paren + 2..)?.trim_start();
    let rest = rest.strip_prefix("error ")?;

    let colon = rest.find(':')?;
    let code = rest[..colon].to_string();
    let message = rest[colon + 1..].trim().to_string();

    Some(TscDiagnostic {
        file,
        line: line_num,
        col,
        code,
        message,
    })
}

/// Start a tsc --watch process and spawn a reader thread that broadcasts
/// type errors via HMR. Returns `None` if tsc is not found.
pub fn spawn_tsc_watcher(project_root: &Path, hmr: HmrBroadcast) -> Option<TscProcess> {
    let tsc = find_tsc(project_root)?;

    let mut child = Command::new(&tsc)
        .arg("--noEmit")
        .arg("--watch")
        .arg("--preserveWatchOutput")
        .current_dir(project_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let stdout = child.stdout.take()?;

    std::thread::spawn(move || {
        let reader = std::io::BufReader::new(stdout);
        let mut errors: Vec<TscDiagnostic> = Vec::new();

        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };

            // Strip ANSI escape codes that tsc --watch emits
            let clean = strip_ansi(&line);

            // tsc emits "Found N error(s)." or "Found 0 errors." at the end of each check
            if clean.starts_with("Found ") && clean.contains(" error") {
                if errors.is_empty() {
                    debug!("tsc: no type errors");
                    hmr.send_tsc_clear();
                } else {
                    debug!(count = errors.len(), "tsc: type errors found");
                    hmr.send_tsc_errors(std::mem::take(&mut errors));
                }
                continue;
            }

            // Try parsing as diagnostic line
            if let Some(diag) = parse_tsc_diagnostic(&clean) {
                errors.push(diag);
            }
        }
    });

    Some(TscProcess { child })
}

/// Strip ANSI escape sequences from a string.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip until we hit a letter (end of escape sequence)
            for c2 in chars.by_ref() {
                if c2.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn parse_diagnostic_line() {
        let line = "pages/index.tsx(10,5): error TS2322: Type 'string' is not assignable to type 'number'.";
        let diag = parse_tsc_diagnostic(line).unwrap();
        assert_eq!(diag.file, "pages/index.tsx");
        assert_eq!(diag.line, 10);
        assert_eq!(diag.col, 5);
        assert_eq!(diag.code, "TS2322");
        assert_eq!(
            diag.message,
            "Type 'string' is not assignable to type 'number'."
        );
    }

    #[test]
    fn parse_non_diagnostic() {
        assert!(parse_tsc_diagnostic("Starting compilation...").is_none());
        assert!(parse_tsc_diagnostic("Found 0 errors.").is_none());
        assert!(parse_tsc_diagnostic("").is_none());
    }

    #[test]
    fn strip_ansi_codes() {
        let input = "\x1b[96mpages/index.tsx\x1b[0m:\x1b[93m10\x1b[0m:\x1b[93m5\x1b[0m";
        let clean = strip_ansi(input);
        assert_eq!(clean, "pages/index.tsx:10:5");
    }

    #[test]
    fn strip_ansi_preserves_clean_string() {
        let input = "no ansi here";
        assert_eq!(strip_ansi(input), input);
    }
}
