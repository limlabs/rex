use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Transform a source file to ESM JavaScript using OXC.
///
/// Strips TypeScript, transforms JSX to `React.createElement` calls,
/// and rewrites CSS imports to empty statements.
pub fn transform_to_esm(source: &str, filename: &str) -> Result<String> {
    let allocator = oxc_allocator::Allocator::default();
    let source_type = oxc_span::SourceType::from_path(filename)
        .map_err(|e| anyhow::anyhow!("Unknown source type for {filename}: {e}"))?;

    let mut ret = oxc_parser::Parser::new(&allocator, source, source_type).parse();
    if ret.panicked {
        anyhow::bail!("OXC parser panicked on {filename}");
    }

    let semantic = oxc_semantic::SemanticBuilder::new()
        .build(&ret.program)
        .semantic;

    let options = oxc_transformer::TransformOptions::default();
    let transformer = oxc_transformer::Transformer::new(&allocator, Path::new(filename), &options);
    transformer.build_with_scoping(semantic.into_scoping(), &mut ret.program);

    let code = oxc_codegen::Codegen::new().build(&ret.program).code;

    // Strip CSS imports: `import './foo.css';` and `import styles from './foo.module.css';`
    // CSS is handled via <link> tags in dev mode, not through JS.
    let code = strip_css_imports(&code);

    Ok(code)
}

/// Strip CSS import statements from generated JS.
/// Matches lines like:
///   import "./foo.css";
///   import './foo.css';
///   import styles from "./foo.module.css";
fn strip_css_imports(code: &str) -> String {
    let mut result = String::with_capacity(code.len());
    for line in code.lines() {
        let trimmed = line.trim();
        if is_css_import(trimmed) {
            // Replace with empty line to preserve source map line numbers
            result.push('\n');
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }
    result
}

/// Check if a line is a CSS import statement.
fn is_css_import(line: &str) -> bool {
    if !line.starts_with("import ") {
        return false;
    }
    // Match: import "...css"; or import '...css';
    // Match: import something from "...css"; or import something from '...css';
    let css_extensions = [
        ".css\"", ".css'", ".scss\"", ".scss'", ".sass\"", ".sass'", ".less\"", ".less'",
    ];
    css_extensions
        .iter()
        .any(|ext| line.ends_with(&format!("{ext};")))
}

/// Cached transform entry: source hash + transformed output.
struct CachedTransform {
    source_hash: u64,
    output: String,
}

/// Thread-safe transform cache keyed by absolute file path.
///
/// Shared between server-side V8 ESM loader and client-side dev middleware
/// to avoid transforming the same file twice.
pub struct TransformCache {
    entries: Mutex<HashMap<PathBuf, CachedTransform>>,
}

impl TransformCache {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Transform a file, using the cache if the source hasn't changed.
    /// Returns the transformed JS output.
    pub fn transform(&self, path: &Path, source: &str) -> Result<String> {
        let hash = hash_source(source);
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("input.tsx");

        // Check cache
        {
            let entries = self.entries.lock().expect("TransformCache lock poisoned");
            if let Some(cached) = entries.get(path) {
                if cached.source_hash == hash {
                    return Ok(cached.output.clone());
                }
            }
        }

        // Cache miss — transform
        let output = transform_to_esm(source, filename)
            .with_context(|| format!("Failed to transform {}", path.display()))?;

        // Update cache
        {
            let mut entries = self.entries.lock().expect("TransformCache lock poisoned");
            entries.insert(
                path.to_path_buf(),
                CachedTransform {
                    source_hash: hash,
                    output: output.clone(),
                },
            );
        }

        Ok(output)
    }

    /// Invalidate a single cache entry (e.g. after a file change).
    pub fn invalidate(&self, path: &Path) {
        let mut entries = self.entries.lock().expect("TransformCache lock poisoned");
        entries.remove(path);
    }

    /// Get the cached transform output without re-checking source hash.
    /// Returns None if the path is not in the cache.
    pub fn get_cached(&self, path: &Path) -> Option<String> {
        let entries = self.entries.lock().expect("TransformCache lock poisoned");
        entries.get(path).map(|c| c.output.clone())
    }
}

impl Default for TransformCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Simple hash function for change detection.
fn hash_source(source: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    hasher.finish()
}
