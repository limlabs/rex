//! Utilities for ESM dev mode: resolve aliases, page sources, and SSR runtime access.

use anyhow::Result;
use rex_core::DataStrategy;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Build page source pairs for ESM dev mode: `(module_name, abs_path)`.
///
/// Used to construct the `page_sources` parameter for `IsolatePool::new_esm()`.
pub fn esm_page_sources(scan: &rex_router::ScanResult) -> Vec<(String, PathBuf)> {
    let mut pages = Vec::new();
    for route in &scan.routes {
        pages.push((route.module_name(), route.abs_path.clone()));
    }
    pages
}

/// Build the resolve aliases HashMap for ESM dev mode (V8 module loader).
///
/// Maps bare specifiers like `rex/head` to absolute paths under the server runtime dir.
pub fn esm_resolve_aliases() -> Result<HashMap<String, PathBuf>> {
    let runtime_dir = crate::build_utils::runtime_server_dir()?;
    let mut aliases = HashMap::new();
    for (spec, file) in [
        ("rex/head", "head.ts"),
        ("rex/link", "link.ts"),
        ("rex/router", "router.ts"),
        ("rex/document", "document.ts"),
        ("rex/image", "image.ts"),
        ("rex/middleware", "middleware.ts"),
        ("next/document", "document.ts"),
    ] {
        let path = runtime_dir.join(file);
        if path.exists() {
            aliases.insert(spec.to_string(), path.canonicalize()?);
        }
    }
    Ok(aliases)
}

/// Get the SSR runtime source. Used by the ESM dev mode startup path.
pub fn ssr_runtime_source() -> &'static str {
    crate::server_bundle::ssr_runtime_source()
}

/// Detect data strategy for a page source file.
///
/// Delegates to the OXC-based AST analysis in `build_utils`. Falls back to
/// `DataStrategy::None` on read or parse errors.
pub fn detect_data_strategy(source_path: &Path) -> DataStrategy {
    crate::build_utils::detect_data_strategy(source_path).unwrap_or(DataStrategy::None)
}
