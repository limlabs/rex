//! Client reference manifest for RSC.
//!
//! Maps client reference IDs to their chunk URLs and export names.
//! Used by the flight protocol to resolve `"use client"` references.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A single client reference entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientRefEntry {
    /// URL path to the client chunk (e.g., `/blog-a1b2c3.js`)
    pub chunk_url: String,
    /// The export name from the chunk (e.g., `"default"`, `"Counter"`)
    pub export_name: String,
}

/// Maps client reference IDs to their chunk location.
///
/// Reference IDs are stable hashes of `(file_path, export_name, build_id)`.
/// The flight protocol embeds these IDs in the wire format, and the client
/// runtime uses this manifest to resolve them to actual module chunks.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClientReferenceManifest {
    pub entries: HashMap<String, ClientRefEntry>,
}

impl ClientReferenceManifest {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, ref_id: &str, chunk_url: String, export_name: String) {
        self.entries.insert(
            ref_id.to_string(),
            ClientRefEntry {
                chunk_url,
                export_name,
            },
        );
    }

    /// Produce the server-side webpack bundler config for `renderToReadableStream`.
    ///
    /// React's server looks up `config[$$id]` → `{ id, name, chunks }`.
    /// We use the reference ID as both the `$$id` and the module `id`.
    pub fn to_server_webpack_config(&self) -> serde_json::Value {
        let mut config = serde_json::Map::new();
        for (ref_id, entry) in &self.entries {
            config.insert(
                ref_id.clone(),
                serde_json::json!({
                    "id": ref_id,
                    "name": &entry.export_name,
                    "chunks": []
                }),
            );
        }
        serde_json::Value::Object(config)
    }

    /// Produce the SSR-side webpack module map for `createFromReadableStream`.
    ///
    /// React's client looks up `config[moduleId][exportName]` → `{ id, name, chunks }`.
    /// The `moduleId` and `exportName` come from the flight data emitted by the server.
    pub fn to_ssr_webpack_manifest(&self) -> serde_json::Value {
        let mut manifest = serde_json::Map::new();
        for (ref_id, entry) in &self.entries {
            let mut export_map = serde_json::Map::new();
            export_map.insert(
                entry.export_name.clone(),
                serde_json::json!({
                    "id": ref_id,
                    "chunks": [],
                    "name": &entry.export_name
                }),
            );
            // Wildcard fallback
            export_map.insert(
                "*".to_string(),
                serde_json::json!({
                    "id": ref_id,
                    "chunks": [],
                    "name": ""
                }),
            );
            manifest.insert(ref_id.clone(), serde_json::Value::Object(export_map));
        }
        serde_json::Value::Object(manifest)
    }
}

/// Generate a stable reference ID for a client component export.
///
/// Uses truncated SHA-256 for cross-version stability.
/// `DefaultHasher` is not guaranteed stable across Rust releases;
/// a cryptographic hash ensures IDs are consistent across builds.
pub fn client_reference_id(rel_path: &str, export_name: &str, build_id: &str) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
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

    #[test]
    fn reference_id_is_deterministic() {
        let id1 = client_reference_id("components/Counter.tsx", "default", "abc123");
        let id2 = client_reference_id("components/Counter.tsx", "default", "abc123");
        assert_eq!(id1, id2);
    }

    #[test]
    fn reference_id_differs_by_export() {
        let id1 = client_reference_id("Counter.tsx", "default", "abc");
        let id2 = client_reference_id("Counter.tsx", "Counter", "abc");
        assert_ne!(id1, id2);
    }

    #[test]
    fn reference_id_differs_by_build() {
        let id1 = client_reference_id("Counter.tsx", "default", "build1");
        let id2 = client_reference_id("Counter.tsx", "default", "build2");
        assert_ne!(id1, id2);
    }

    #[test]
    fn manifest_add_and_lookup() {
        let mut manifest = ClientReferenceManifest::new();
        manifest.add(
            "abc123",
            "/Counter-xyz.js".to_string(),
            "default".to_string(),
        );
        assert_eq!(manifest.entries.len(), 1);
        let entry = manifest.entries.get("abc123").unwrap();
        assert_eq!(entry.chunk_url, "/Counter-xyz.js");
        assert_eq!(entry.export_name, "default");
    }

    #[test]
    fn server_webpack_config_format() {
        let mut manifest = ClientReferenceManifest::new();
        manifest.add("ref1", "/Counter.js".to_string(), "default".to_string());
        manifest.add("ref2", "/Input.js".to_string(), "Input".to_string());

        let config = manifest.to_server_webpack_config();
        let obj = config.as_object().unwrap();

        let entry1 = obj.get("ref1").unwrap();
        assert_eq!(entry1["id"], "ref1");
        assert_eq!(entry1["name"], "default");
        assert!(entry1["chunks"].as_array().unwrap().is_empty());

        let entry2 = obj.get("ref2").unwrap();
        assert_eq!(entry2["id"], "ref2");
        assert_eq!(entry2["name"], "Input");
    }

    #[test]
    fn ssr_webpack_manifest_format() {
        let mut manifest = ClientReferenceManifest::new();
        manifest.add("ref1", "/Counter.js".to_string(), "default".to_string());

        let ssr = manifest.to_ssr_webpack_manifest();
        let obj = ssr.as_object().unwrap();

        // Has entry keyed by ref_id
        let ref1 = obj.get("ref1").unwrap().as_object().unwrap();

        // Has export name entry
        let default_entry = ref1.get("default").unwrap();
        assert_eq!(default_entry["id"], "ref1");
        assert_eq!(default_entry["name"], "default");

        // Has wildcard fallback
        let wildcard = ref1.get("*").unwrap();
        assert_eq!(wildcard["id"], "ref1");
        assert_eq!(wildcard["name"], "");
    }
}
