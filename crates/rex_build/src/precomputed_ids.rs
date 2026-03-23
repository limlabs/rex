//! Pre-computed RSC reference IDs from the ESM module graph walk.
//!
//! In dev mode, the ESM path walks the import graph first and computes all
//! client reference IDs and server action IDs. These are then passed to the
//! IIFE build path (client bundles, SSR bundle) so both systems use identical
//! IDs, avoiding mismatches caused by path canonicalization differences.

use std::collections::HashMap;

/// Pre-computed IDs from the ESM module graph walk.
///
/// When present, the IIFE builder looks up IDs from this map instead of
/// computing its own. Lookups that miss fall back to direct computation,
/// ensuring production builds (no ESM) work unchanged.
#[derive(Debug, Clone, Default)]
pub struct PrecomputedIds {
    /// Maps canonical_rel_path → {export_name → client_reference_id}.
    client_ref_ids: HashMap<String, HashMap<String, String>>,
    /// Maps canonical_rel_path → {export_name → server_action_id}.
    server_action_ids: HashMap<String, HashMap<String, String>>,
}

impl PrecomputedIds {
    /// Build from ESM-collected module data.
    ///
    /// Client boundary ref_ids are taken directly from the ESM walk (already computed).
    /// Server action IDs are computed here using ESM's canonical rel_paths.
    pub fn from_collected(
        collected: &crate::esm_transform::CollectedModules,
        build_id: &str,
    ) -> Self {
        let mut ids = Self::default();

        // Client boundaries: ESM already computed ref_ids during the graph walk
        for boundary in &collected.client_boundaries {
            let mut exports_map = HashMap::new();
            for (export, ref_id) in boundary.exports.iter().zip(boundary.ref_ids.iter()) {
                exports_map.insert(export.clone(), ref_id.clone());
            }
            ids.client_ref_ids
                .insert(boundary.rel_path.clone(), exports_map);
        }

        // Inline extracted server actions: ESM already computed action_ids
        for action in &collected.extracted_actions {
            ids.server_action_ids
                .entry(action.rel_path.clone())
                .or_default()
                .insert(action.action_name.clone(), action.action_id.clone());
        }

        // Module-level "use server" modules: compute IDs using ESM's rel_paths
        for module in &collected.server_action_modules {
            let entry = ids
                .server_action_ids
                .entry(module.rel_path.clone())
                .or_default();
            for export in &module.exports {
                let action_id = crate::server_action_manifest::server_action_id(
                    &module.rel_path,
                    export,
                    build_id,
                );
                entry.insert(export.clone(), action_id);
            }
        }

        ids
    }

    /// Look up a client reference ID by rel_path and export name.
    /// Falls back to computing if not pre-computed.
    pub fn client_ref_id(&self, rel_path: &str, export: &str, build_id: &str) -> String {
        self.client_ref_ids
            .get(rel_path)
            .and_then(|m| m.get(export))
            .cloned()
            .unwrap_or_else(|| {
                crate::client_manifest::client_reference_id(rel_path, export, build_id)
            })
    }

    /// Look up a server action ID by rel_path and export name.
    /// Falls back to computing if not pre-computed.
    pub fn server_action_id(&self, rel_path: &str, export: &str, build_id: &str) -> String {
        self.server_action_ids
            .get(rel_path)
            .and_then(|m| m.get(export))
            .cloned()
            .unwrap_or_else(|| {
                crate::server_action_manifest::server_action_id(rel_path, export, build_id)
            })
    }
}
