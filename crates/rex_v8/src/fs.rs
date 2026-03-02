use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Sentinel prefix for error returns from fs callbacks.
/// The JS shim checks for this prefix and throws a proper Error.
pub const FS_ERROR_PREFIX: &str = "__REX_FS_ERR__";

/// Resolve a user-supplied path against the project root, ensuring it stays within the sandbox.
///
/// - Relative paths resolve against project_root
/// - Absolute paths are allowed only if they're within project_root
/// - Canonicalization defeats `../` traversal and symlink escapes
fn resolve_sandboxed_path(
    project_root: &Path,
    requested: &str,
) -> std::result::Result<PathBuf, String> {
    // Canonicalize project root to resolve symlinks (e.g., /var → /private/var on macOS)
    let canonical_root = project_root.canonicalize().map_err(|e| {
        format!(
            "EACCES: cannot resolve project root '{}': {}",
            project_root.display(),
            e
        )
    })?;

    let raw = Path::new(requested);
    let candidate = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        canonical_root.join(raw)
    };

    // For existing paths, canonicalize and check prefix
    if candidate.exists() {
        let canonical = candidate
            .canonicalize()
            .map_err(|e| format!("EACCES: cannot resolve path '{}': {}", requested, e))?;
        if canonical.starts_with(&canonical_root) {
            return Ok(canonical);
        }
        return Err(format!("EACCES: path '{}' escapes project root", requested));
    }

    // For non-existent paths (writes/mkdir): canonicalize the deepest existing ancestor
    let mut ancestor = candidate.as_path();
    loop {
        if let Some(parent) = ancestor.parent() {
            if parent.exists() {
                let canonical_parent = parent.canonicalize().map_err(|e| {
                    format!("EACCES: cannot resolve parent of '{}': {}", requested, e)
                })?;
                if canonical_parent.starts_with(&canonical_root) {
                    // Re-join the remaining components onto the canonical parent
                    let suffix = candidate
                        .strip_prefix(parent)
                        .unwrap_or(candidate.as_path());
                    return Ok(canonical_parent.join(suffix));
                }
                return Err(format!("EACCES: path '{}' escapes project root", requested));
            }
            ancestor = parent;
        } else {
            return Err(format!("ENOENT: no existing ancestor for '{}'", requested));
        }
    }
}

/// Map `std::io::ErrorKind` to Node.js-style error codes.
fn io_error_to_node_code(e: &std::io::Error) -> &'static str {
    match e.kind() {
        std::io::ErrorKind::NotFound => "ENOENT",
        std::io::ErrorKind::PermissionDenied => "EACCES",
        std::io::ErrorKind::AlreadyExists => "EEXIST",
        std::io::ErrorKind::InvalidInput => "EINVAL",
        std::io::ErrorKind::NotADirectory => "ENOTDIR",
        std::io::ErrorKind::IsADirectory => "EISDIR",
        _ => "EIO",
    }
}

/// Format an fs error as a sentinel string that the JS shim will parse and throw.
fn fs_error_string(code: &str, message: &str) -> String {
    // Format: __REX_FS_ERR__{"code":"ENOENT","message":"..."}
    format!(
        "{}{}",
        FS_ERROR_PREFIX,
        serde_json::json!({"code": code, "message": message})
    )
}

/// Set return value to an error sentinel string.
fn set_error(scope: &mut v8::PinScope, rv: &mut v8::ReturnValue, code: &str, message: &str) {
    let err = fs_error_string(code, message);
    let v = v8::String::new(scope, &err).expect("V8 string alloc");
    rv.set(v.into());
}

/// Helper: extract project root from the first arg (passed by JS shim).
/// Returns (project_root_string, remaining_args_start_index).
fn extract_root(scope: &mut v8::PinScope, args: &v8::FunctionCallbackArguments) -> Option<String> {
    if args.length() < 1 {
        return None;
    }
    let val = args.get(0);
    if val.is_undefined() || val.is_null() {
        return None;
    }
    Some(val.to_rust_string_lossy(scope))
}

/// `__rex_fs_read_file_sync(project_root, path, encoding?)` → String | Uint8Array | error sentinel
fn fs_read_file_sync(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let root = match extract_root(scope, &args) {
        Some(r) => r,
        None => {
            set_error(
                scope,
                &mut rv,
                "EACCES",
                "fs operations require a project root",
            );
            return;
        }
    };

    if args.length() < 2 {
        set_error(
            scope,
            &mut rv,
            "EINVAL",
            "readFileSync requires a path argument",
        );
        return;
    }

    let path_str = args.get(1).to_rust_string_lossy(scope);
    let project_root = Path::new(&root);

    let resolved = match resolve_sandboxed_path(project_root, &path_str) {
        Ok(p) => p,
        Err(msg) => {
            let code = if msg.starts_with("ENOENT") {
                "ENOENT"
            } else {
                "EACCES"
            };
            set_error(scope, &mut rv, code, &msg);
            return;
        }
    };

    // Check for encoding option (arg index 2)
    let wants_utf8 = if args.length() > 2 {
        let opt = args.get(2);
        if opt.is_string() {
            let s = opt.to_rust_string_lossy(scope);
            s == "utf8" || s == "utf-8"
        } else if opt.is_object() {
            let obj = v8::Local::<v8::Object>::try_from(opt).ok();
            obj.and_then(|o| {
                let key = v8::String::new(scope, "encoding")?;
                let val = o.get(scope, key.into())?;
                if val.is_string() {
                    let s = val.to_rust_string_lossy(scope);
                    Some(s == "utf8" || s == "utf-8")
                } else {
                    Some(false)
                }
            })
            .unwrap_or(false)
        } else {
            false
        }
    } else {
        false
    };

    if wants_utf8 {
        match fs::read_to_string(&resolved) {
            Ok(content) => {
                let v = v8::String::new(scope, &content).expect("V8 string alloc");
                rv.set(v.into());
            }
            Err(e) => {
                let code = io_error_to_node_code(&e);
                set_error(scope, &mut rv, code, &format!("{}: {}", e, path_str));
            }
        }
    } else {
        match fs::read(&resolved) {
            Ok(bytes) => {
                let store = v8::ArrayBuffer::new_backing_store_from_vec(bytes).make_shared();
                let ab = v8::ArrayBuffer::with_backing_store(scope, &store);
                let uint8 = v8::Uint8Array::new(scope, ab, 0, ab.byte_length())
                    .expect("V8 Uint8Array alloc");
                rv.set(uint8.into());
            }
            Err(e) => {
                let code = io_error_to_node_code(&e);
                set_error(scope, &mut rv, code, &format!("{}: {}", e, path_str));
            }
        }
    }
}

/// `__rex_fs_write_file_sync(project_root, path, data)` → undefined | error sentinel
fn fs_write_file_sync(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let root = match extract_root(scope, &args) {
        Some(r) => r,
        None => {
            set_error(
                scope,
                &mut rv,
                "EACCES",
                "fs operations require a project root",
            );
            return;
        }
    };

    if args.length() < 3 {
        set_error(
            scope,
            &mut rv,
            "EINVAL",
            "writeFileSync requires path and data arguments",
        );
        return;
    }

    let path_str = args.get(1).to_rust_string_lossy(scope);
    let project_root = Path::new(&root);

    let resolved = match resolve_sandboxed_path(project_root, &path_str) {
        Ok(p) => p,
        Err(msg) => {
            let code = if msg.starts_with("ENOENT") {
                "ENOENT"
            } else {
                "EACCES"
            };
            set_error(scope, &mut rv, code, &msg);
            return;
        }
    };

    let data = args.get(2);
    let bytes: Vec<u8> = if data.is_string() {
        data.to_rust_string_lossy(scope).into_bytes()
    } else if let Ok(uint8) = v8::Local::<v8::Uint8Array>::try_from(data) {
        let len = uint8.byte_length();
        let mut buf = vec![0u8; len];
        uint8.copy_contents(&mut buf);
        buf
    } else if let Ok(ab) = v8::Local::<v8::ArrayBuffer>::try_from(data) {
        let len = ab.byte_length();
        let mut buf = vec![0u8; len];
        if len > 0 {
            let store = ab.get_backing_store();
            for (i, cell) in store.iter().enumerate().take(len) {
                buf[i] = cell.get();
            }
        }
        buf
    } else {
        data.to_rust_string_lossy(scope).into_bytes()
    };

    if let Err(e) = fs::write(&resolved, &bytes) {
        let code = io_error_to_node_code(&e);
        set_error(scope, &mut rv, code, &format!("{}: {}", e, path_str));
    }
}

/// `__rex_fs_readdir_sync(project_root, path)` → JSON string | error sentinel
fn fs_readdir_sync(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let root = match extract_root(scope, &args) {
        Some(r) => r,
        None => {
            set_error(
                scope,
                &mut rv,
                "EACCES",
                "fs operations require a project root",
            );
            return;
        }
    };

    if args.length() < 2 {
        set_error(
            scope,
            &mut rv,
            "EINVAL",
            "readdirSync requires a path argument",
        );
        return;
    }

    let path_str = args.get(1).to_rust_string_lossy(scope);
    let project_root = Path::new(&root);

    let resolved = match resolve_sandboxed_path(project_root, &path_str) {
        Ok(p) => p,
        Err(msg) => {
            let code = if msg.starts_with("ENOENT") {
                "ENOENT"
            } else {
                "EACCES"
            };
            set_error(scope, &mut rv, code, &msg);
            return;
        }
    };

    match fs::read_dir(&resolved) {
        Ok(entries) => {
            let names: Vec<String> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect();
            let json = serde_json::to_string(&names).expect("JSON serialize");
            let v = v8::String::new(scope, &json).expect("V8 string alloc");
            rv.set(v.into());
        }
        Err(e) => {
            let code = io_error_to_node_code(&e);
            set_error(scope, &mut rv, code, &format!("{}: {}", e, path_str));
        }
    }
}

/// `__rex_fs_stat_sync(project_root, path)` → JSON string | error sentinel
fn fs_stat_sync(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let root = match extract_root(scope, &args) {
        Some(r) => r,
        None => {
            set_error(
                scope,
                &mut rv,
                "EACCES",
                "fs operations require a project root",
            );
            return;
        }
    };

    if args.length() < 2 {
        set_error(
            scope,
            &mut rv,
            "EINVAL",
            "statSync requires a path argument",
        );
        return;
    }

    let path_str = args.get(1).to_rust_string_lossy(scope);
    let project_root = Path::new(&root);

    let resolved = match resolve_sandboxed_path(project_root, &path_str) {
        Ok(p) => p,
        Err(msg) => {
            let code = if msg.starts_with("ENOENT") {
                "ENOENT"
            } else {
                "EACCES"
            };
            set_error(scope, &mut rv, code, &msg);
            return;
        }
    };

    match fs::metadata(&resolved) {
        Ok(meta) => {
            let is_file = meta.is_file();
            let is_dir = meta.is_dir();
            let is_symlink = meta.is_symlink();
            let size = meta.len();

            let mtime_ms = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);

            let json = serde_json::json!({
                "isFile": is_file,
                "isDirectory": is_dir,
                "isSymbolicLink": is_symlink,
                "size": size,
                "mtimeMs": mtime_ms,
            });
            let s = serde_json::to_string(&json).expect("JSON serialize");
            let v = v8::String::new(scope, &s).expect("V8 string alloc");
            rv.set(v.into());
        }
        Err(e) => {
            let code = io_error_to_node_code(&e);
            set_error(scope, &mut rv, code, &format!("{}: {}", e, path_str));
        }
    }
}

/// `__rex_fs_mkdir_sync(project_root, path, opts?)` → undefined | error sentinel
fn fs_mkdir_sync(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let root = match extract_root(scope, &args) {
        Some(r) => r,
        None => {
            set_error(
                scope,
                &mut rv,
                "EACCES",
                "fs operations require a project root",
            );
            return;
        }
    };

    if args.length() < 2 {
        set_error(
            scope,
            &mut rv,
            "EINVAL",
            "mkdirSync requires a path argument",
        );
        return;
    }

    let path_str = args.get(1).to_rust_string_lossy(scope);
    let project_root = Path::new(&root);

    let resolved = match resolve_sandboxed_path(project_root, &path_str) {
        Ok(p) => p,
        Err(msg) => {
            let code = if msg.starts_with("ENOENT") {
                "ENOENT"
            } else {
                "EACCES"
            };
            set_error(scope, &mut rv, code, &msg);
            return;
        }
    };

    // Check for { recursive: true } at arg index 2
    let recursive = if args.length() > 2 {
        let opt = args.get(2);
        if let Ok(obj) = v8::Local::<v8::Object>::try_from(opt) {
            let key = v8::String::new(scope, "recursive").expect("V8 string alloc");
            obj.get(scope, key.into())
                .map(|v| v.is_true())
                .unwrap_or(false)
        } else {
            false
        }
    } else {
        false
    };

    let result = if recursive {
        fs::create_dir_all(&resolved)
    } else {
        fs::create_dir(&resolved)
    };

    if let Err(e) = result {
        let code = io_error_to_node_code(&e);
        set_error(scope, &mut rv, code, &format!("{}: {}", e, path_str));
    }
}

/// `__rex_fs_exists_sync(project_root, path)` → boolean
fn fs_exists_sync(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let root = match extract_root(scope, &args) {
        Some(r) => r,
        None => {
            let v = v8::Boolean::new(scope, false);
            rv.set(v.into());
            return;
        }
    };

    if args.length() < 2 {
        let v = v8::Boolean::new(scope, false);
        rv.set(v.into());
        return;
    }

    let path_str = args.get(1).to_rust_string_lossy(scope);
    let project_root = Path::new(&root);

    let exists = match resolve_sandboxed_path(project_root, &path_str) {
        Ok(p) => p.exists(),
        Err(_) => false,
    };

    let v = v8::Boolean::new(scope, exists);
    rv.set(v.into());
}

/// `__rex_fs_unlink_sync(project_root, path)` → undefined | error sentinel
fn fs_unlink_sync(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let root = match extract_root(scope, &args) {
        Some(r) => r,
        None => {
            set_error(
                scope,
                &mut rv,
                "EACCES",
                "fs operations require a project root",
            );
            return;
        }
    };

    if args.length() < 2 {
        set_error(
            scope,
            &mut rv,
            "EINVAL",
            "unlinkSync requires a path argument",
        );
        return;
    }

    let path_str = args.get(1).to_rust_string_lossy(scope);
    let project_root = Path::new(&root);

    let resolved = match resolve_sandboxed_path(project_root, &path_str) {
        Ok(p) => p,
        Err(msg) => {
            let code = if msg.starts_with("ENOENT") {
                "ENOENT"
            } else {
                "EACCES"
            };
            set_error(scope, &mut rv, code, &msg);
            return;
        }
    };

    if let Err(e) = fs::remove_file(&resolved) {
        let code = io_error_to_node_code(&e);
        set_error(scope, &mut rv, code, &format!("{}: {}", e, path_str));
    }
}

/// `__rex_fs_rm_sync(project_root, path, opts?)` → undefined | error sentinel
fn fs_rm_sync(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let root = match extract_root(scope, &args) {
        Some(r) => r,
        None => {
            set_error(
                scope,
                &mut rv,
                "EACCES",
                "fs operations require a project root",
            );
            return;
        }
    };

    if args.length() < 2 {
        set_error(scope, &mut rv, "EINVAL", "rmSync requires a path argument");
        return;
    }

    let path_str = args.get(1).to_rust_string_lossy(scope);
    let project_root = Path::new(&root);

    let resolved = match resolve_sandboxed_path(project_root, &path_str) {
        Ok(p) => p,
        Err(msg) => {
            let code = if msg.starts_with("ENOENT") {
                "ENOENT"
            } else {
                "EACCES"
            };
            set_error(scope, &mut rv, code, &msg);
            return;
        }
    };

    // Check for { recursive: true, force: true } at arg index 2
    let (recursive, force) = if args.length() > 2 {
        let opt = args.get(2);
        if let Ok(obj) = v8::Local::<v8::Object>::try_from(opt) {
            let r_key = v8::String::new(scope, "recursive").expect("V8 string alloc");
            let f_key = v8::String::new(scope, "force").expect("V8 string alloc");
            let r = obj
                .get(scope, r_key.into())
                .map(|v| v.is_true())
                .unwrap_or(false);
            let f = obj
                .get(scope, f_key.into())
                .map(|v| v.is_true())
                .unwrap_or(false);
            (r, f)
        } else {
            (false, false)
        }
    } else {
        (false, false)
    };

    if force && !resolved.exists() {
        return;
    }

    let result = if resolved.is_dir() && recursive {
        fs::remove_dir_all(&resolved)
    } else if resolved.is_dir() {
        fs::remove_dir(&resolved)
    } else {
        fs::remove_file(&resolved)
    };

    if let Err(e) = result {
        if force && e.kind() == std::io::ErrorKind::NotFound {
            return;
        }
        let code = io_error_to_node_code(&e);
        set_error(scope, &mut rv, code, &format!("{}: {}", e, path_str));
    }
}

/// Register all `__rex_fs_*` callbacks on the V8 global object.
pub fn register_fs_callbacks(
    scope: &mut v8::ContextScope<v8::HandleScope>,
    global: v8::Local<v8::Object>,
) -> Result<()> {
    macro_rules! register_fn {
        ($name:expr, $callback:expr) => {{
            let t = v8::FunctionTemplate::new(scope, $callback);
            let f = t
                .get_function(scope)
                .ok_or_else(|| anyhow::anyhow!("Failed to create {}", $name))?;
            let k = v8::String::new(scope, $name)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed for '{}'", $name))?;
            global.set(scope, k.into(), f.into());
        }};
    }

    register_fn!("__rex_fs_read_file_sync", fs_read_file_sync);
    register_fn!("__rex_fs_write_file_sync", fs_write_file_sync);
    register_fn!("__rex_fs_readdir_sync", fs_readdir_sync);
    register_fn!("__rex_fs_stat_sync", fs_stat_sync);
    register_fn!("__rex_fs_mkdir_sync", fs_mkdir_sync);
    register_fn!("__rex_fs_exists_sync", fs_exists_sync);
    register_fn!("__rex_fs_unlink_sync", fs_unlink_sync);
    register_fn!("__rex_fs_rm_sync", fs_rm_sync);

    debug!("Registered fs callbacks on globalThis");
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
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
}
