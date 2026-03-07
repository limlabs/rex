//! Server action manifest for RSC.
//!
//! Maps server action IDs to their module paths and export names.
//! Used to dispatch `"use server"` function calls from the client.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A single server action entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerActionEntry {
    /// Relative path to the module containing this action.
    pub module_path: String,
    /// The export name (e.g., `"incrementCounter"`, `"default"`).
    pub export_name: String,
}

/// Maps server action IDs to their module location.
///
/// Action IDs are stable hashes of `(file_path, export_name, build_id)`.
/// The client calls `/_rex/action/{build_id}/{action_id}` and the server
/// uses this manifest to dispatch to the correct function.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerActionManifest {
    pub actions: HashMap<String, ServerActionEntry>,
}

impl ServerActionManifest {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, action_id: &str, module_path: String, export_name: String) {
        self.actions.insert(
            action_id.to_string(),
            ServerActionEntry {
                module_path,
                export_name,
            },
        );
    }

    /// Produce a config object for `registerServerReference` in the flight bundle.
    ///
    /// Format: `{ [actionId]: { id: actionId, name: exportName, chunks: [] } }`
    pub fn to_server_reference_config(&self) -> serde_json::Value {
        let mut config = serde_json::Map::new();
        for (action_id, entry) in &self.actions {
            config.insert(
                action_id.clone(),
                serde_json::json!({
                    "id": action_id,
                    "name": &entry.export_name,
                    "chunks": []
                }),
            );
        }
        serde_json::Value::Object(config)
    }
}

/// Generate a stable action ID for a server action export.
///
/// Uses SHA-256 with a `"server_action\0"` prefix to avoid collision
/// with client reference IDs (which use a bare hash).
pub fn server_action_id(rel_path: &str, export_name: &str, build_id: &str) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(b"server_action\0");
    hasher.update(rel_path.as_bytes());
    hasher.update(b"\0");
    hasher.update(export_name.as_bytes());
    hasher.update(b"\0");
    hasher.update(build_id.as_bytes());
    let hash = hasher.finalize();
    // Truncate to 7 bytes (14 hex chars) — enough for uniqueness, short for URLs
    hex::encode(&hash[..7])
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::client_manifest::client_reference_id;

    #[test]
    fn action_id_is_deterministic() {
        let id1 = server_action_id("app/actions.ts", "increment", "abc123");
        let id2 = server_action_id("app/actions.ts", "increment", "abc123");
        assert_eq!(id1, id2);
    }

    #[test]
    fn action_id_differs_by_export() {
        let id1 = server_action_id("actions.ts", "increment", "abc");
        let id2 = server_action_id("actions.ts", "decrement", "abc");
        assert_ne!(id1, id2);
    }

    #[test]
    fn action_id_differs_by_build() {
        let id1 = server_action_id("actions.ts", "increment", "build1");
        let id2 = server_action_id("actions.ts", "increment", "build2");
        assert_ne!(id1, id2);
    }

    #[test]
    fn action_id_differs_from_client_reference_id() {
        let action = server_action_id("app/actions.ts", "default", "abc");
        let client = client_reference_id("app/actions.ts", "default", "abc");
        assert_ne!(action, client);
    }

    #[test]
    fn manifest_add_and_lookup() {
        let mut manifest = ServerActionManifest::new();
        manifest.add(
            "sa_abc",
            "app/actions.ts".to_string(),
            "increment".to_string(),
        );
        assert_eq!(manifest.actions.len(), 1);
        let entry = manifest.actions.get("sa_abc").unwrap();
        assert_eq!(entry.module_path, "app/actions.ts");
        assert_eq!(entry.export_name, "increment");
    }

    #[test]
    fn server_reference_config_format() {
        let mut manifest = ServerActionManifest::new();
        manifest.add("sa1", "actions.ts".to_string(), "increment".to_string());
        manifest.add("sa2", "actions.ts".to_string(), "decrement".to_string());

        let config = manifest.to_server_reference_config();
        let obj = config.as_object().unwrap();

        let entry1 = obj.get("sa1").unwrap();
        assert_eq!(entry1["id"], "sa1");
        assert_eq!(entry1["name"], "increment");
        assert!(entry1["chunks"].as_array().unwrap().is_empty());

        let entry2 = obj.get("sa2").unwrap();
        assert_eq!(entry2["id"], "sa2");
        assert_eq!(entry2["name"], "decrement");
    }
}
