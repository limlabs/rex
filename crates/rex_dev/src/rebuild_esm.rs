//! ESM HMR fast path for server-side file changes.
//!
//! Instead of a full rolldown rebuild (~3500ms), re-transforms the single changed
//! file with OXC (~1ms) and invalidates the V8 ESM module registry (~10-50ms).

use anyhow::Result;
use rex_server::state::AppState;
use std::path::Path;
use std::sync::Arc;
use tracing::debug;

/// Attempt the ESM fast path for a file change.
///
/// Returns `true` if the fast path succeeded, `false` if it should fall back
/// to a full rebuild (e.g., ESM state not available, transform failed).
pub async fn try_esm_fast_path(state: &Arc<AppState>, changed_path: &Path) -> Result<bool> {
    let esm_lock = match &state.esm {
        Some(lock) => lock,
        None => return Ok(false),
    };

    let changed_specifier = changed_path.to_string_lossy().to_string();

    // Read and transform the changed file, rewriting relative imports to absolute paths
    let source = match std::fs::read_to_string(changed_path) {
        Ok(s) => s,
        Err(_) => return Ok(false),
    };

    let known_specifiers = rex_build::esm_transform::dep_specifiers(
        state
            .hot
            .read()
            .ok()
            .is_some_and(|h| !h.manifest.app_routes.is_empty()),
    );

    let transformed = match rex_build::esm_transform::transform_and_rewrite_imports(
        &source,
        changed_path,
        &state.project_root,
        &known_specifiers,
    ) {
        Ok(t) => t,
        Err(e) => {
            debug!("ESM transform failed for {}: {e:#}", changed_path.display());
            return Ok(false);
        }
    };

    // Update the source module in ESM state
    let (dep_modules, source_modules, entry_specifier, entry_source) = {
        let mut esm = esm_lock
            .write()
            .map_err(|e| anyhow::anyhow!("ESM state lock poisoned: {e}"))?;

        // Find and update the changed module
        let mut found = false;
        for module in &mut esm.source_modules {
            if module.specifier == changed_specifier {
                module.source = transformed.clone();
                found = true;
                break;
            }
        }

        if !found {
            debug!("File not in ESM module list, falling back to full rebuild");
            return Ok(false);
        }

        (
            esm.dep_modules.clone(),
            esm.source_modules.clone(),
            esm.entry_specifier.clone(),
            esm.entry_source.clone(),
        )
    };

    // Invalidate ESM modules in all isolates
    let source_modules_arc = Arc::new(source_modules);
    let entry_spec_arc = Arc::new(entry_specifier);
    let entry_src_arc = Arc::new(entry_source);

    state
        .isolate_pool
        .invalidate_esm_module_all(
            dep_modules,
            source_modules_arc,
            entry_spec_arc,
            entry_src_arc,
        )
        .await?;

    debug!(
        path = %changed_path.display(),
        "ESM fast path: module invalidated"
    );

    Ok(true)
}
