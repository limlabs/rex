//! V8 native ESM module registry.
//!
//! Core registry for compiling, storing, and resolving V8 ESM modules.
//! Each module is identified by a canonical specifier (absolute path or
//! bare specifier like "react"). Content hashing enables module invalidation
//! for HMR: when a file changes, its hash changes, so V8 sees a new module.
//!
//! The resolve callback uses a thread-local map since V8 callbacks are bare
//! function pointers that cannot capture state.

use anyhow::Result;
use std::cell::RefCell;
use std::collections::HashMap;
use tracing::debug;

// Thread-local module map used by the V8 resolve callback.
// V8's `ResolveModuleCallback` is a bare function pointer — it can't capture
// `&self`. We store compiled modules here so the callback can look them up.
thread_local! {
    static MODULE_MAP: RefCell<HashMap<String, v8::Global<v8::Module>>> =
        RefCell::new(HashMap::new());
}

// Thread-local storage for synthetic module globalThis keys.
// Maps module identity hash → globalThis property name where the namespace object is stored.
thread_local! {
    static SYNTHETIC_KEYS: RefCell<HashMap<i32, String>> = RefCell::new(HashMap::new());
}

// Thread-local storage mapping module identity hash → specifier (absolute path).
// Used by the resolve callback to resolve relative imports against the referrer.
thread_local! {
    static MODULE_PATHS: RefCell<HashMap<i32, String>> = RefCell::new(HashMap::new());
}

/// Registry for V8 native ESM modules within a single isolate.
///
/// Manages the lifecycle of compiled modules:
/// compile → store → resolve → instantiate → evaluate.
/// Each module is keyed by its canonical specifier (typically an absolute path).
pub struct EsmModuleRegistry {
    /// Content hash per specifier, used to detect changes for HMR.
    hashes: HashMap<String, String>,
}

impl Default for EsmModuleRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl EsmModuleRegistry {
    pub fn new() -> Self {
        Self {
            hashes: HashMap::new(),
        }
    }

    /// Compile an ESM source string into a V8 module and store it in the registry.
    ///
    /// The `specifier` is the canonical module identifier (e.g., absolute path).
    /// The module is stored in the thread-local map for the resolve callback.
    pub fn compile_module(
        &mut self,
        scope: &mut v8::PinScope,
        specifier: &str,
        source: &str,
    ) -> Result<()> {
        let content_hash = simple_hash(source);

        let resource_name = v8::String::new(scope, specifier)
            .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed for specifier"))?;
        let source_str = v8::String::new(scope, source)
            .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed for module source"))?;

        let origin = v8::ScriptOrigin::new(
            scope,
            resource_name.into(),
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

        let mut v8_source = v8::script_compiler::Source::new(source_str, Some(&origin));

        let module =
            v8::script_compiler::compile_module(scope, &mut v8_source).ok_or_else(|| {
                // Try to get a more helpful error
                let tc_msg = format!("Failed to compile module: {specifier}");
                anyhow::anyhow!(tc_msg)
            })?;

        // Store identity hash → specifier for relative import resolution
        let identity_hash = module.get_identity_hash().get();
        MODULE_PATHS.with(|paths| {
            paths
                .borrow_mut()
                .insert(identity_hash, specifier.to_string());
        });

        let global_module = v8::Global::new(scope, module);

        // Store in thread-local map for resolve callback
        MODULE_MAP.with(|map| {
            map.borrow_mut()
                .insert(specifier.to_string(), global_module);
        });

        self.hashes.insert(specifier.to_string(), content_hash);

        Ok(())
    }

    /// Create a synthetic module that wraps values from a globalThis property.
    ///
    /// Used for dependency wrappers: e.g., after evaluating a React IIFE that sets
    /// `globalThis.__rex_React`, create a synthetic "react" module whose exports
    /// come from that global.
    ///
    /// `globals_expr` is a JS expression that evaluates to an object whose
    /// properties become the module's named exports.
    pub fn create_synthetic_module(
        &mut self,
        scope: &mut v8::PinScope,
        specifier: &str,
        export_names: &[&str],
        globals_expr: &str,
    ) -> Result<()> {
        let module_name = v8::String::new(scope, specifier)
            .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;

        let v8_export_names: Vec<v8::Local<v8::String>> = export_names
            .iter()
            .map(|name| {
                v8::String::new(scope, name)
                    .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed for export: {name}"))
            })
            .collect::<Result<Vec<_>>>()?;

        // Store the globals expression in a globalThis property so the eval callback
        // can access it. Use a sanitized key derived from the specifier.
        let store_key = format!(
            "__rex_synth_{}",
            specifier.replace(['/', '-', '.', '@'], "_")
        );
        let store_script = format!("globalThis['{}'] = {}", store_key, globals_expr);

        // Evaluate the store script to make the namespace object available
        {
            v8::tc_scope!(tc, scope);
            let code = v8::String::new(tc, &store_script)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
            let script = v8::Script::compile(tc, code, None)
                .ok_or_else(|| anyhow::anyhow!("Failed to compile synthetic module setup"))?;
            script.run(tc).ok_or_else(|| {
                let msg = tc
                    .exception()
                    .map(|e| e.to_rust_string_lossy(tc))
                    .unwrap_or_else(|| "Unknown error".into());
                anyhow::anyhow!("Failed to set up synthetic module {specifier}: {msg}")
            })?;
        }

        // Create the synthetic module
        let module = v8::Module::create_synthetic_module(
            scope,
            module_name,
            &v8_export_names,
            synthetic_module_eval_callback,
        );

        let identity_hash = module.get_identity_hash().get();

        // Store identity_hash → store_key mapping so the eval callback knows
        // where to find the namespace object for this module.
        SYNTHETIC_KEYS.with(|keys| {
            keys.borrow_mut().insert(identity_hash, store_key);
        });

        let global_module = v8::Global::new(scope, module);

        MODULE_MAP.with(|map| {
            map.borrow_mut()
                .insert(specifier.to_string(), global_module);
        });

        Ok(())
    }

    /// Instantiate and evaluate a module (and all its dependencies via resolve callback).
    ///
    /// The module must have been compiled and stored via `compile_module()`.
    /// All dependencies must also be in the registry before calling this.
    pub fn instantiate_and_evaluate(
        &self,
        scope: &mut v8::PinScope,
        specifier: &str,
    ) -> Result<()> {
        let module = MODULE_MAP.with(|map| {
            map.borrow()
                .get(specifier)
                .map(|g| v8::Local::new(scope, g))
        });

        let module =
            module.ok_or_else(|| anyhow::anyhow!("Module not found in registry: {specifier}"))?;

        // Instantiate (links all imports via resolve callback)
        {
            v8::tc_scope!(tc, scope);
            let result = module.instantiate_module(tc, resolve_callback);
            match result {
                Some(true) => {}
                _ => {
                    let exception = tc
                        .exception()
                        .map(|e| e.to_rust_string_lossy(tc))
                        .unwrap_or_else(|| "Unknown instantiation error".into());
                    return Err(anyhow::anyhow!(
                        "Failed to instantiate module {specifier}: {exception}"
                    ));
                }
            }
        }

        // Evaluate
        {
            v8::tc_scope!(tc, scope);
            let result = module.evaluate(tc);
            match result {
                Some(val) => {
                    // If the result is a promise, check its state
                    if let Ok(promise) = v8::Local::<v8::Promise>::try_from(val) {
                        match promise.state() {
                            v8::PromiseState::Rejected => {
                                let rejection = promise.result(tc);
                                let mut exception = rejection.to_rust_string_lossy(tc);
                                // Try to extract stack trace and source location
                                if let Ok(err_obj) = v8::Local::<v8::Object>::try_from(rejection) {
                                    if let Some(stack_key) = v8::String::new(tc, "stack") {
                                        if let Some(stack) = err_obj.get(tc, stack_key.into()) {
                                            let stack_str = stack.to_rust_string_lossy(tc);
                                            if !stack_str.is_empty() && stack_str != "undefined" {
                                                exception = stack_str;
                                            }
                                        }
                                    }
                                }
                                // Also try V8's message API for source location
                                let msg = v8::Exception::create_message(tc, rejection);
                                let resource = msg
                                    .get_script_resource_name(tc)
                                    .map(|v| v.to_rust_string_lossy(tc))
                                    .unwrap_or_default();
                                let line = msg.get_line_number(tc).unwrap_or(0);
                                let source_line = msg
                                    .get_source_line(tc)
                                    .map(|v| v.to_rust_string_lossy(tc))
                                    .unwrap_or_default();
                                return Err(anyhow::anyhow!(
                                    "Module evaluation rejected for {specifier}: {exception}\n  at {resource}:{line}\n  > {source_line}"
                                ));
                            }
                            v8::PromiseState::Pending => {
                                // Pump microtasks to settle
                                tc.perform_microtask_checkpoint();
                                if promise.state() == v8::PromiseState::Rejected {
                                    let exception = promise.result(tc).to_rust_string_lossy(tc);
                                    return Err(anyhow::anyhow!(
                                        "Module evaluation rejected for {specifier}: {exception}"
                                    ));
                                }
                            }
                            v8::PromiseState::Fulfilled => {}
                        }
                    }
                }
                None => {
                    let exception = tc
                        .exception()
                        .map(|e| e.to_rust_string_lossy(tc))
                        .unwrap_or_else(|| "Unknown evaluation error".into());
                    return Err(anyhow::anyhow!(
                        "Failed to evaluate module {specifier}: {exception}"
                    ));
                }
            }
        }

        debug!(specifier, "Module evaluated");
        Ok(())
    }

    /// Remove a module from the registry (for HMR invalidation).
    ///
    /// After removal, the module can be recompiled with updated source.
    /// Note: V8 doesn't support re-instantiating modules. After invalidation,
    /// a fresh entry module must be compiled that imports the updated module.
    pub fn remove_module(&mut self, specifier: &str) {
        MODULE_MAP.with(|map| {
            map.borrow_mut().remove(specifier);
        });
        self.hashes.remove(specifier);
    }

    /// Check if a module's content has changed (by comparing hashes).
    pub fn has_changed(&self, specifier: &str, new_source: &str) -> bool {
        match self.hashes.get(specifier) {
            Some(old_hash) => *old_hash != simple_hash(new_source),
            None => true,
        }
    }

    /// Clear all modules from the registry.
    pub fn clear(&mut self) {
        MODULE_MAP.with(|map| {
            map.borrow_mut().clear();
        });
        SYNTHETIC_KEYS.with(|keys| {
            keys.borrow_mut().clear();
        });
        MODULE_PATHS.with(|paths| {
            paths.borrow_mut().clear();
        });
        self.hashes.clear();
    }

    /// Check if a module exists in the registry.
    pub fn contains(&self, specifier: &str) -> bool {
        MODULE_MAP.with(|map| map.borrow().contains_key(specifier))
    }

    /// Register an alias so that `alias_specifier` resolves to the same
    /// compiled module as `target_specifier`. This shares a single V8 module
    /// instance between two specifiers — no wrapper, no re-execution.
    pub fn alias_module(&self, alias_specifier: &str, target_specifier: &str) -> bool {
        MODULE_MAP.with(|map| {
            let map = map.borrow();
            if let Some(module) = map.get(target_specifier) {
                let cloned = module.clone();
                drop(map);
                MODULE_MAP.with(|m| {
                    m.borrow_mut().insert(alias_specifier.to_string(), cloned);
                });
                true
            } else {
                false
            }
        })
    }
}

/// Normalize a path by removing `.` and `..` components without touching the filesystem.
/// Unlike `canonicalize()`, this works for virtual paths (e.g., `/_rex_deps/./chunk.js`).
fn normalize_path(path: &std::path::Path) -> std::path::PathBuf {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {} // skip "."
            std::path::Component::ParentDir => {
                components.pop();
            }
            other => components.push(other),
        }
    }
    components.iter().collect()
}

/// V8 resolve callback: look up modules from the thread-local registry.
///
/// Called by V8 during `instantiate_module()` for each `import` statement.
/// Uses `CallbackScope` to get a proper scope for converting Global → Local.
///
/// Resolution order:
/// 1. Direct lookup by specifier (handles absolute paths and synthetic modules)
/// 2. For relative specifiers, resolve against the referrer's path with extension probing
fn resolve_callback<'s>(
    context: v8::Local<'s, v8::Context>,
    specifier: v8::Local<'s, v8::String>,
    _import_attributes: v8::Local<'s, v8::FixedArray>,
    referrer: v8::Local<'s, v8::Module>,
) -> Option<v8::Local<'s, v8::Module>> {
    // SAFETY: This callback is called by V8 during instantiate_module(),
    // which is called from within a valid scope. CallbackScope creates a
    // HandleScope on the current isolate's stack for the duration of this call.
    v8::callback_scope!(unsafe scope, context);

    let spec_str = specifier.to_rust_string_lossy(scope);

    // 1. Direct lookup (handles absolute paths, synthetic modules, and bare specifiers)
    let direct = MODULE_MAP.with(|map| {
        map.borrow()
            .get(&spec_str)
            .map(|global| v8::Local::new(scope, global))
    });
    if direct.is_some() {
        return direct;
    }

    tracing::debug!(specifier = %spec_str, "ESM resolve: not found in direct lookup");

    // 2. Relative import resolution using referrer's path
    if spec_str.starts_with('.') {
        let referrer_hash = referrer.get_identity_hash().get();
        let referrer_path = MODULE_PATHS.with(|paths| paths.borrow().get(&referrer_hash).cloned());

        if let Some(ref_path) = referrer_path {
            let ref_dir = std::path::Path::new(&ref_path).parent()?;
            let candidate = normalize_path(&ref_dir.join(&spec_str));

            // Try exact path, then with extensions
            let extensions = ["", ".tsx", ".ts", ".jsx", ".js"];
            for ext in &extensions {
                let try_path = if ext.is_empty() {
                    candidate.clone()
                } else {
                    let fname = candidate.file_name()?.to_str()?;
                    candidate.with_file_name(format!("{fname}{ext}"))
                };
                let try_str = try_path.to_string_lossy().to_string();
                let found = MODULE_MAP.with(|map| {
                    map.borrow()
                        .get(&try_str)
                        .map(|global| v8::Local::new(scope, global))
                });
                if found.is_some() {
                    return found;
                }
            }

            // Try index files in directory
            if candidate.is_dir() {
                for ext in &[".tsx", ".ts", ".jsx", ".js"] {
                    let index = candidate.join(format!("index{ext}"));
                    let try_str = index.to_string_lossy().to_string();
                    let found = MODULE_MAP.with(|map| {
                        map.borrow()
                            .get(&try_str)
                            .map(|global| v8::Local::new(scope, global))
                    });
                    if found.is_some() {
                        return found;
                    }
                }
            }
        }
    }

    tracing::warn!(specifier = %spec_str, "ESM resolve: module not found");
    None
}

/// Evaluation callback for synthetic modules.
///
/// Reads exports from the globalThis property stored by `create_synthetic_module()`
/// and sets them on the module via `set_synthetic_module_export`.
fn synthetic_module_eval_callback<'s>(
    context: v8::Local<'s, v8::Context>,
    module: v8::Local<'s, v8::Module>,
) -> Option<v8::Local<'s, v8::Value>> {
    // SAFETY: Called by V8 during module evaluation within a valid scope.
    v8::callback_scope!(unsafe scope, context);

    let identity_hash = module.get_identity_hash().get();

    // Look up the globalThis key for this synthetic module
    let store_key = SYNTHETIC_KEYS.with(|keys| keys.borrow().get(&identity_hash).cloned())?;

    // Get the namespace object from globalThis[store_key]
    let global = context.global(scope);
    let key = v8::String::new(scope, &store_key)?;
    let namespace = global.get(scope, key.into())?;

    // The namespace should be an object — set each property as a module export
    if let Ok(ns_obj) = v8::Local::<v8::Object>::try_from(namespace) {
        let prop_names =
            ns_obj.get_own_property_names(scope, v8::GetPropertyNamesArgs::default())?;
        let len = prop_names.length();
        for i in 0..len {
            let key = prop_names.get_index(scope, i)?;
            if let Ok(key_str) = v8::Local::<v8::String>::try_from(key) {
                if let Some(value) = ns_obj.get(scope, key) {
                    let _ = module.set_synthetic_module_export(scope, key_str, value);
                }
            }
        }
    } else {
        // If it's not an object, treat it as a default export
        let default_key = v8::String::new(scope, "default")?;
        let _ = module.set_synthetic_module_export(scope, default_key, namespace);
    }

    // Return undefined to indicate success
    Some(v8::undefined(scope).into())
}

/// Simple string hash for content change detection.
fn simple_hash(s: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Dynamic `import()` callback for V8.
///
/// Called when JavaScript code uses `import('specifier')`. Looks up the module
/// in the thread-local registry, instantiates it if needed, and returns a
/// promise that resolves with the module's namespace object.
pub fn dynamic_import_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _host_defined_options: v8::Local<'s, v8::Data>,
    resource_name: v8::Local<'s, v8::Value>,
    specifier: v8::Local<'s, v8::String>,
    _import_attributes: v8::Local<'s, v8::FixedArray>,
) -> Option<v8::Local<'s, v8::Promise>> {
    let spec_str = specifier.to_rust_string_lossy(scope);
    let referrer_str = resource_name.to_rust_string_lossy(scope);

    // Resolve specifier (same logic as the static resolve callback)
    let resolved = if spec_str.starts_with('.') {
        // Relative import — resolve against referrer
        let ref_dir = std::path::Path::new(&referrer_str).parent()?;
        let candidate = normalize_path(&ref_dir.join(&spec_str));
        candidate.to_string_lossy().to_string()
    } else {
        spec_str.clone()
    };

    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);

    // Look up in module registry
    let module = MODULE_MAP.with(|map| {
        map.borrow()
            .get(&resolved)
            .map(|g| v8::Local::new(scope, g))
    });

    let module = match module {
        Some(m) => m,
        None => {
            let msg = v8::String::new(
                scope,
                &format!("Cannot find module '{spec_str}' (resolved: {resolved})"),
            )?;
            let err = v8::Exception::error(scope, msg);
            resolver.reject(scope, err);
            return Some(promise);
        }
    };

    // Instantiate if needed
    if module.get_status() == v8::ModuleStatus::Uninstantiated {
        let ok = module.instantiate_module(scope, resolve_callback);
        if ok != Some(true) {
            let msg = v8::String::new(
                scope,
                &format!("Failed to instantiate dynamically imported module: {resolved}"),
            )?;
            let err = v8::Exception::error(scope, msg);
            resolver.reject(scope, err);
            return Some(promise);
        }
    }

    // Evaluate if needed
    if module.get_status() == v8::ModuleStatus::Instantiated {
        let result = module.evaluate(scope);
        if let Some(val) = result {
            if let Ok(p) = v8::Local::<v8::Promise>::try_from(val) {
                if p.state() == v8::PromiseState::Rejected {
                    let reason = p.result(scope);
                    resolver.reject(scope, reason);
                    return Some(promise);
                }
                scope.perform_microtask_checkpoint();
            }
        }
    }

    // Get namespace and resolve
    let namespace = module.get_module_namespace();
    resolver.resolve(scope, namespace);
    Some(promise)
}
