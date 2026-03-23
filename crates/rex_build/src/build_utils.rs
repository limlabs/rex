use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

// Re-export detection functions from page_exports for backward compatibility
pub(crate) use crate::page_exports::{detect_data_strategy, extract_middleware_matchers};

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

/// Generate a build ID based on current timestamp
pub fn generate_build_id() -> String {
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

/// Node.js polyfill aliases for server-side bundles (pages-router + RSC).
pub fn node_polyfill_aliases(runtime_dir: &Path) -> Vec<(String, Vec<Option<String>>)> {
    // Node.js builtins: each entry generates both bare and `node:` prefixed aliases
    let node_builtins: &[(&str, &str)] = &[
        ("process", "process.ts"),
        ("fs", "fs.ts"),
        ("fs/promises", "fs-promises.ts"),
        ("path", "path.ts"),
        ("buffer", "buffer.ts"),
        ("crypto", "crypto.ts"),
        ("events", "events.cjs"),
        ("net", "net.ts"),
        ("tls", "tls.ts"),
        ("dns", "dns.ts"),
        ("os", "os.ts"),
        ("stream", "stream.ts"),
        ("string_decoder", "string_decoder.ts"),
        ("util", "util.ts"),
        ("url", "url-module.ts"),
        ("stream/web", "stream-web.ts"),
        ("child_process", "child_process.ts"),
        ("assert", "assert.ts"),
        ("module", "module.ts"),
        ("http", "http.ts"),
        ("https", "https.ts"),
        ("zlib", "zlib.ts"),
        ("worker_threads", "worker_threads.ts"),
        ("http2", "http2.ts"),
        ("tty", "tty.ts"),
        ("readline", "readline.ts"),
        ("querystring", "querystring.ts"),
    ];
    let mk = |s: &str, f: &str| {
        (
            s.to_string(),
            vec![Some(runtime_dir.join(f).to_string_lossy().to_string())],
        )
    };
    let mut aliases: Vec<_> = Vec::new();
    for &(name, file) in node_builtins {
        aliases.push(mk(name, file));
        aliases.push(mk(&format!("node:{name}"), file));
    }
    // Non-node: aliases
    for &(name, file) in &[
        ("cloudflare:sockets", "cloudflare-sockets.ts"),
        ("file-type", "file-type.ts"),
        ("file-type/core", "file-type.ts"),
        ("file-type/core.js", "file-type.ts"),
        ("sharp", "sharp.ts"),
    ] {
        aliases.push(mk(name, file));
    }
    let next_mappings: &[(&str, &str)] = &[
        ("next/link", "next-link.ts"),
        ("next/image", "next-image.ts"),
        ("next/head", "head.ts"),
        ("next/router", "next-router.ts"),
        ("next/navigation", "next-navigation.ts"),
        ("next/headers", "next-headers.ts"),
        ("next/cache", "next-cache.ts"),
        ("next/server", "next-server.ts"),
        ("next/font/google", "next-font.ts"),
        ("next/font/local", "next-font.ts"),
        ("next/dynamic", "next-dynamic.ts"),
        ("@vercel/og", "empty.ts"),
        ("next/og", "empty.ts"),
    ];
    aliases.extend(
        next_mappings
            .iter()
            .filter(|(_, f)| runtime_dir.join(f).exists())
            .map(|(s, f)| mk(s, f)),
    );
    aliases
}

/// Parse tsconfig.json `paths` from the project root and return rolldown-compatible
/// resolve aliases. Handles wildcard patterns (e.g. `"@/*": ["./src/*"]`).
///
/// Returns an empty Vec if tsconfig.json doesn't exist or has no paths.
pub(crate) fn tsconfig_path_aliases(project_root: &Path) -> Vec<(String, Vec<Option<String>>)> {
    let tsconfig_path = project_root.join("tsconfig.json");
    let content = match fs::read_to_string(&tsconfig_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    // Strip single-line comments (tsconfig allows them)
    let stripped: String = content
        .lines()
        .map(|line| {
            if let Some(idx) = line.find("//") {
                // Only strip if not inside a string
                let before = &line[..idx];
                if before.matches('"').count() % 2 == 0 {
                    return before;
                }
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n");

    let parsed: serde_json::Value = match serde_json::from_str(&stripped) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let paths = match parsed
        .get("compilerOptions")
        .and_then(|co| co.get("paths"))
        .and_then(|p| p.as_object())
    {
        Some(p) => p,
        None => return Vec::new(),
    };

    let base_url = parsed
        .get("compilerOptions")
        .and_then(|co| co.get("baseUrl"))
        .and_then(|b| b.as_str())
        .unwrap_or(".");
    let base_dir = project_root.join(base_url);

    let mut aliases = Vec::new();
    for (key, value) in paths {
        let targets = match value.as_array() {
            Some(arr) => arr,
            None => continue,
        };
        let target = match targets.first().and_then(|t| t.as_str()) {
            Some(t) => t,
            None => continue,
        };

        if key.ends_with("/*") && target.ends_with("/*") {
            // Wildcard: "@/*" → "./src/*" becomes "@" → "{base}/src"
            let alias_key = key[..key.len() - 2].to_string();
            let alias_target = base_dir
                .join(&target[..target.len() - 2])
                .to_string_lossy()
                .to_string();
            aliases.push((alias_key, vec![Some(alias_target)]));
        } else {
            // Exact: "@payload-config" → "./payload.config.ts"
            aliases.push((
                key.clone(),
                vec![Some(base_dir.join(target).to_string_lossy().to_string())],
            ));
        }
    }

    // Always map /public → {project_root}/public (Next.js convention for
    // absolute-path asset imports like `/public/image.svg`)
    let public_dir = project_root.join("public");
    if public_dir.exists() {
        aliases.push((
            "/public".to_string(),
            vec![Some(public_dir.to_string_lossy().to_string())],
        ));
    }

    aliases
}

/// Get the path to the server runtime files.
/// These are embedded in the source tree at runtime/server/.
pub fn runtime_server_dir() -> Result<PathBuf> {
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rex_core::{PageType, Route};
    use std::path::PathBuf;

    fn make_route(pattern: &str, file_path: &str) -> Route {
        Route {
            pattern: pattern.to_string(),
            file_path: PathBuf::from(file_path),
            abs_path: PathBuf::from(file_path),
            dynamic_segments: vec![],
            page_type: PageType::Regular,
            specificity: 0,
        }
    }

    #[test]
    fn test_route_to_chunk_name_index() {
        let route = make_route("/", "index.tsx");
        assert_eq!(route_to_chunk_name(&route), "index");
    }

    #[test]
    fn test_route_to_chunk_name_nested() {
        let route = make_route("/blog/:slug", "blog/[slug].tsx");
        assert_eq!(route_to_chunk_name(&route), "blog-_slug_");
    }

    #[test]
    fn test_route_to_chunk_name_deep_nested() {
        let route = make_route("/docs/api/:path*", "docs/api/[...path].tsx");
        assert_eq!(route_to_chunk_name(&route), "docs-api-_...path_");
    }

    #[test]
    fn test_find_route_for_chunk_found() {
        let routes = vec![
            make_route("/", "index.tsx"),
            make_route("/about", "about.tsx"),
            make_route("/blog/:slug", "blog/[slug].tsx"),
        ];
        let found = find_route_for_chunk("about", &routes);
        assert!(found.is_some());
        assert_eq!(found.expect("route should exist").pattern, "/about");
    }

    #[test]
    fn test_find_route_for_chunk_not_found() {
        let routes = vec![make_route("/", "index.tsx")];
        assert!(find_route_for_chunk("nonexistent", &routes).is_none());
    }

    #[test]
    fn test_generate_build_id_format() {
        let id = generate_build_id();
        assert_eq!(id.len(), 16, "build ID should be 16 hex chars");
        assert!(
            id.chars().all(|c| c.is_ascii_hexdigit()),
            "build ID should contain only hex chars"
        );
    }

    #[test]
    fn test_runtime_server_dir_exists() {
        let dir = runtime_server_dir().unwrap();
        assert!(dir.exists());
        assert!(dir.join("head.ts").exists());
    }

    #[test]
    fn test_runtime_client_dir_exists() {
        let dir = runtime_client_dir().unwrap();
        assert!(dir.exists());
        assert!(dir.join("link.ts").exists());
    }

    #[test]
    fn test_embedded_runtime_has_all_polyfill_alias_targets() {
        // Extract embedded runtime to temp dir (simulates installed binary)
        let base = crate::embedded_runtime::extract().unwrap();
        let server_dir = base.join("server");

        // Generate aliases using the extracted dir
        let aliases = node_polyfill_aliases(&server_dir);

        let mut missing = Vec::new();
        for (specifier, targets) in &aliases {
            for target in targets.iter().flatten() {
                let path = PathBuf::from(target);
                if !path.exists() {
                    missing.push(format!("  {specifier} -> {target}"));
                }
            }
        }

        assert!(
            missing.is_empty(),
            "Embedded runtime is missing files referenced by node_polyfill_aliases().\n\
             Add these to embedded_runtime.rs SERVER_FILES:\n{}",
            missing.join("\n")
        );
    }

    #[test]
    fn test_embedded_runtime_covers_all_server_files() {
        // Get the source runtime/server/ directory
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let source_dir = manifest_dir.join("../../runtime/server");
        if !source_dir.exists() {
            return; // Skip if not in source tree
        }

        // Extract embedded runtime
        let base = crate::embedded_runtime::extract().unwrap();
        let embedded_dir = base.join("server");

        let mut missing = Vec::new();
        for entry in fs::read_dir(&source_dir).unwrap() {
            let entry = entry.unwrap();
            if !entry.file_type().unwrap().is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if !embedded_dir.join(&name).exists() {
                missing.push(name);
            }
        }

        assert!(
            missing.is_empty(),
            "Files in runtime/server/ not embedded in embedded_runtime.rs:\n  {}",
            missing.join("\n  ")
        );
    }
}
