use anyhow::Result;
use rex_core::DataStrategy;
use std::fs;
use std::path::{Path, PathBuf};

/// Map a route to a chunk name for rolldown entry naming.
pub(crate) fn route_to_chunk_name(route: &rex_core::Route) -> String {
    let module_name = route.module_name();
    let cn = module_name.replace('/', "-").replace(['[', ']'], "_");
    if cn.is_empty() {
        "index".to_string()
    } else {
        cn
    }
}

/// Find the route that matches a given chunk name.
pub(crate) fn find_route_for_chunk<'a>(
    chunk_name: &str,
    routes: &'a [rex_core::Route],
) -> Option<&'a rex_core::Route> {
    routes.iter().find(|r| route_to_chunk_name(r) == chunk_name)
}

/// Detect data strategy by scanning source for exported getServerSideProps / getStaticProps.
pub(crate) fn detect_data_strategy(source_path: &Path) -> Result<DataStrategy> {
    let source = fs::read_to_string(source_path)?;
    detect_data_strategy_from_source(&source)
}

/// Detect data strategy from source content (no filesystem access).
pub(crate) fn detect_data_strategy_from_source(source: &str) -> Result<DataStrategy> {
    let has_gssp = source.lines().any(|l| {
        let t = l.trim();
        t.contains("getServerSideProps") && (t.starts_with("export ") || t.starts_with("export{"))
    });
    let has_gsp = source.lines().any(|l| {
        let t = l.trim();
        t.contains("getStaticProps") && (t.starts_with("export ") || t.starts_with("export{"))
    });
    if has_gssp && has_gsp {
        anyhow::bail!("Page exports both getStaticProps and getServerSideProps");
    }
    if has_gsp {
        return Ok(DataStrategy::GetStaticProps);
    }
    if has_gssp {
        return Ok(DataStrategy::GetServerSideProps);
    }
    Ok(DataStrategy::None)
}

/// Extract middleware matcher patterns from middleware source code.
/// Looks for `export const config = { matcher: [...] }` and extracts string literals.
/// Returns empty vec if no matcher found (meaning: run on all paths).
pub(crate) fn extract_middleware_matchers(source: &str) -> Vec<String> {
    let mut matchers = Vec::new();
    let mut in_config = false;
    let mut in_matcher = false;
    let mut brace_depth: i32 = 0;

    for line in source.lines() {
        let trimmed = line.trim();

        if !in_config {
            if trimmed.contains("export") && trimmed.contains("config") {
                in_config = true;
                // Check if matcher is on the same line
                if let Some(idx) = trimmed.find("matcher") {
                    let after = &trimmed[idx..];
                    extract_strings_from_fragment(after, &mut matchers);
                    if after.contains(']') {
                        return matchers;
                    }
                    in_matcher = true;
                }
                brace_depth =
                    trimmed.matches('{').count() as i32 - trimmed.matches('}').count() as i32;
                if brace_depth <= 0 && trimmed.contains('}') {
                    in_config = false;
                }
            }
        } else {
            brace_depth +=
                trimmed.matches('{').count() as i32 - trimmed.matches('}').count() as i32;

            if !in_matcher {
                if let Some(idx) = trimmed.find("matcher") {
                    let after = &trimmed[idx..];
                    extract_strings_from_fragment(after, &mut matchers);
                    if after.contains(']') {
                        return matchers;
                    }
                    in_matcher = true;
                }
            } else {
                extract_strings_from_fragment(trimmed, &mut matchers);
                if trimmed.contains(']') {
                    return matchers;
                }
            }

            if brace_depth <= 0 {
                in_config = false;
                in_matcher = false;
            }
        }
    }

    matchers
}

/// Extract string literals (single or double quoted) from a code fragment.
fn extract_strings_from_fragment(fragment: &str, out: &mut Vec<String>) {
    let mut chars = fragment.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\'' || ch == '"' {
            let mut s = String::new();
            for c in chars.by_ref() {
                if c == ch {
                    break;
                }
                s.push(c);
            }
            if !s.is_empty() {
                out.push(s);
            }
        }
    }
}

/// Generate a build ID based on current timestamp
pub(crate) fn generate_build_id() -> String {
    use sha2::{Digest, Sha256};
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before UNIX epoch")
        .as_millis();
    let hash = Sha256::digest(timestamp.to_string().as_bytes());
    hex::encode(&hash[..8])
}

/// Get the path to the client runtime files.
/// These are embedded in the source tree at runtime/client/.
pub(crate) fn runtime_client_dir() -> Result<PathBuf> {
    // In dev: relative to the crate source
    // The runtime files are at the workspace root under runtime/client/
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let runtime_dir = manifest_dir.join("../../runtime/client");
    if runtime_dir.exists() {
        return Ok(runtime_dir.canonicalize()?);
    }
    // Fallback: look relative to current dir
    let cwd_runtime = PathBuf::from("runtime/client");
    if cwd_runtime.exists() {
        return Ok(cwd_runtime.canonicalize()?);
    }
    // Distributed binary: extract embedded runtime files to temp dir
    crate::embedded_runtime::client_dir()
}

/// Get the path to the server runtime files.
/// These are embedded in the source tree at runtime/server/.
pub(crate) fn runtime_server_dir() -> Result<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let runtime_dir = manifest_dir.join("../../runtime/server");
    if runtime_dir.exists() {
        return Ok(runtime_dir.canonicalize()?);
    }
    let cwd_runtime = PathBuf::from("runtime/server");
    if cwd_runtime.exists() {
        return Ok(cwd_runtime.canonicalize()?);
    }
    // Distributed binary: extract embedded runtime files to temp dir
    crate::embedded_runtime::server_dir()
}
