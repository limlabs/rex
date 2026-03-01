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
}

/// Generate a stable reference ID for a client component export.
///
/// Uses a simple hash of the relative path + export name + build ID.
pub fn client_reference_id(rel_path: &str, export_name: &str, build_id: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    rel_path.hash(&mut hasher);
    export_name.hash(&mut hasher);
    build_id.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

#[cfg(test)]
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
}
