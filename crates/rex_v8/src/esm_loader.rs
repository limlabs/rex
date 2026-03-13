use anyhow::Result;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Registry of ESM modules for a single V8 isolate.
///
/// Manages pre-transformed JS sources and resolves import specifiers.
/// The dep IIFE (React + polyfills) is evaluated as a script (not a module),
/// and synthetic ESM wrapper modules re-export from the globals it sets.
pub struct EsmModuleRegistry {
    /// Pre-bundled dep IIFE (React + polyfills), evaluated once at startup.
    dep_iife: Arc<String>,
    /// Pre-transformed JS source for each user file (abs path → JS).
    sources: HashMap<PathBuf, String>,
    /// Bare specifier → abs path (e.g. "rex/head" → "/path/to/runtime/server/head.ts")
    resolve_aliases: HashMap<String, PathBuf>,
    /// Project root for resolving relative imports.
    project_root: PathBuf,
}

impl EsmModuleRegistry {
    pub fn new(
        dep_iife: Arc<String>,
        sources: HashMap<PathBuf, String>,
        resolve_aliases: HashMap<String, PathBuf>,
        project_root: PathBuf,
    ) -> Self {
        Self {
            dep_iife,
            sources,
            resolve_aliases,
            project_root,
        }
    }

    /// Get the dep IIFE source (for evaluating as a script).
    pub fn dep_iife(&self) -> &str {
        &self.dep_iife
    }

    /// Update the source for a single file (after OXC re-transform).
    pub fn update_source(&mut self, path: PathBuf, source: String) {
        self.sources.insert(path, source);
    }

    /// Get a source by path.
    pub fn get_source(&self, path: &Path) -> Option<&str> {
        self.sources.get(path).map(|s| s.as_str())
    }

    /// Get all sources.
    pub fn sources(&self) -> &HashMap<PathBuf, String> {
        &self.sources
    }

    /// Resolve a module specifier from a referrer.
    ///
    /// Returns the absolute path that the specifier resolves to.
    pub fn resolve(&self, specifier: &str, referrer: &Path) -> Option<PathBuf> {
        // 1. Check resolve aliases (bare specifiers: "react", "rex/head", etc.)
        if let Some(path) = self.resolve_aliases.get(specifier) {
            return Some(path.clone());
        }

        // 2. Relative specifiers (./foo, ../bar)
        if specifier.starts_with("./") || specifier.starts_with("../") {
            let base = referrer.parent().unwrap_or(&self.project_root);
            let resolved = base.join(specifier);
            if resolved.exists() {
                return resolved.canonicalize().ok();
            }
            for ext in &["tsx", "ts", "jsx", "js"] {
                let with_ext = resolved.with_extension(ext);
                if with_ext.exists() {
                    return with_ext.canonicalize().ok();
                }
            }
            for ext in &["tsx", "ts", "jsx", "js"] {
                let index = resolved.join(format!("index.{ext}"));
                if index.exists() {
                    return index.canonicalize().ok();
                }
            }
            return None;
        }

        // 3. CSS imports → empty module sentinel
        if specifier.ends_with(".css")
            || specifier.ends_with(".scss")
            || specifier.ends_with(".sass")
            || specifier.ends_with(".less")
        {
            return Some(PathBuf::from("__empty_css__"));
        }

        None
    }

    /// Build the ESM entry module source that imports all pages and sets up globals.
    ///
    /// Equivalent of the virtual entry in `server_bundle.rs`.
    pub fn build_entry_source(&self, page_sources: &[(String, PathBuf)]) -> String {
        let mut entry = String::new();

        // Reference React from globals (set by dep IIFE)
        entry.push_str(
            "var __rex_createElement = globalThis.__rex_createElement;\n\
             var __rex_renderToString = globalThis.__rex_renderToString;\n\n",
        );

        // Import and register pages
        entry.push_str("globalThis.__rex_pages = {};\n");
        for (i, (module_name, page_path)) in page_sources.iter().enumerate() {
            let path_str = page_path.to_string_lossy().replace('\\', "/");
            entry.push_str(&format!("import * as __page{i} from '{path_str}';\n"));
            entry.push_str(&format!(
                "globalThis.__rex_pages['{module_name}'] = __page{i};\n"
            ));
        }

        entry
    }
}

// Thread-local storage for the compiled module map used in resolve callbacks.
// V8 isolates are thread-pinned, so thread-local is safe.
// These are scaffolding for future true ESM module support (currently using
// script-based evaluation; see compile_and_evaluate_esm in ssr_isolate_esm.rs).
thread_local! {
    static MODULE_MAP: RefCell<HashMap<String, v8::Global<v8::Module>>> =
        RefCell::new(HashMap::new());
}

/// Store a compiled module in the thread-local map for use in resolve callbacks.
#[allow(dead_code)]
pub(crate) fn register_module(specifier: String, module: v8::Global<v8::Module>) {
    MODULE_MAP.with(|m| {
        m.borrow_mut().insert(specifier, module);
    });
}

/// Clear all modules from the thread-local map.
#[allow(dead_code)]
pub(crate) fn clear_module_map() {
    MODULE_MAP.with(|m| {
        m.borrow_mut().clear();
    });
}

/// Compile a JS source string as an ESM module in V8.
///
/// Accepts a `ContextScope`-wrapped `PinScope` from inside `v8::scope_with_context!`.
#[allow(dead_code)]
pub(crate) fn compile_esm_module<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    source: &str,
    name: &str,
) -> Result<v8::Local<'s, v8::Module>> {
    let source_str = v8::String::new(scope, source)
        .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed for module source"))?;
    let name_str = v8::String::new(scope, name)
        .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed for module name"))?;

    let origin = v8::ScriptOrigin::new(
        scope,
        name_str.into(),
        0,
        0,
        false,
        0,
        None,
        false,
        false,
        true, // is_module = true
        None,
    );

    let mut compile_source = v8::script_compiler::Source::new(source_str, Some(&origin));

    v8::script_compiler::compile_module(scope, &mut compile_source)
        .ok_or_else(|| anyhow::anyhow!("Failed to compile ESM module: {name}"))
}

/// Create an empty synthetic module (for CSS imports, etc.)
#[allow(dead_code)]
pub(crate) fn create_empty_module<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    name: &str,
) -> Result<v8::Local<'s, v8::Module>> {
    compile_esm_module(scope, "export default {};\n", name)
}

/// V8 module resolve callback for `Module::instantiate_module`.
///
/// Looks up pre-compiled modules from the thread-local map.
/// All modules must be registered via `register_module()` before
/// `instantiate_module()` is called.
#[allow(dead_code)]
pub(crate) fn resolve_module_callback<'s>(
    _context: v8::Local<'s, v8::Context>,
    _specifier: v8::Local<'s, v8::String>,
    _import_attributes: v8::Local<'s, v8::FixedArray>,
    _referrer: v8::Local<'s, v8::Module>,
) -> Option<v8::Local<'s, v8::Module>> {
    // The specifier Local has lifetime 's tied to the caller's scope.
    // We need to read it — but we can't create a scope here since
    // the callback doesn't provide a scope-creating context.
    //
    // Instead, we use the identity hash of the specifier string
    // to look up modules. But actually, since V8 calls this callback
    // with the same scope active, the Local values are valid.
    //
    // The key insight: we can't call to_rust_string_lossy without a scope.
    // So instead, we store modules by their V8 identity hash and match
    // via the module request specifier strings stored at registration time.
    //
    // Alternative approach: since we control all imports, we pre-resolve
    // everything and use V8's synthetic module API. But the simplest
    // approach is to accept that we need the scope.
    //
    // For V8 v146, the resolve callback runs within the instantiation scope,
    // so the Local values share that scope's lifetime. We cannot safely
    // create a Rust-side scope here. Instead, we match by pointer identity
    // of the v8::String — but this is fragile.
    //
    // PRACTICAL SOLUTION: We'll use a different approach in Phase 4.
    // The SsrIsolate in ESM mode won't use the module resolve callback
    // for dynamic resolution. Instead, it will pre-compile all modules,
    // walk import graphs, and register them all before instantiation.
    // The resolve callback is a fallback that should never fire in practice.
    None
}
