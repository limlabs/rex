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
    /// The `id` is the reference ID (stable hash) used as the module identifier
    /// in both SSR (`__webpack_require__`) and the client (pre-loaded into cache).
    ///
    /// The `chunks` array is empty because:
    /// - SSR: modules are already bundled inline in the SSR IIFE
    /// - Client: the hydration entry pre-loads modules from `__REX_RSC_MODULE_MAP__`
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

    /// Serialize the client-facing module map as JSON.
    ///
    /// This is embedded in the HTML as `window.__REX_RSC_MODULE_MAP__` and used
    /// by the hydration runtime to preload client component chunks. Entries with
    /// empty `chunk_url` (placeholders from the server build phase that were never
    /// resolved to a real chunk) are excluded — importing an empty specifier
    /// crashes the browser with `TypeError: Failed to resolve module specifier ''`.
    pub fn to_client_module_map_json(&self) -> String {
        let filtered: HashMap<&String, &ClientRefEntry> = self
            .entries
            .iter()
            .filter(|(_, entry)| !entry.chunk_url.is_empty())
            .collect();

        #[derive(Serialize)]
        struct Wrapper<'a> {
            entries: HashMap<&'a String, &'a ClientRefEntry>,
        }

        serde_json::to_string(&Wrapper { entries: filtered }).unwrap_or_else(|_| "{}".to_string())
    }

    /// Produce the SSR-side webpack module map for `createFromReadableStream`.
    ///
    /// React's client looks up `config[moduleId][exportName]` → `{ id, name, chunks }`.
    /// The `moduleId` comes from the flight I row (which is the chunk URL from
    /// the server config). The SSR bundle resolves these via `__webpack_require__`
    /// backed by `__rex_ssr_modules__`.
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

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

    #[test]
    fn client_module_map_excludes_empty_chunk_urls() {
        let mut manifest = ClientReferenceManifest::new();
        // Real entry with a chunk URL
        manifest.add("ref1", "/Counter.js".to_string(), "default".to_string());
        // Placeholder entry with empty chunk URL (from server build phase)
        manifest.add("ref2", String::new(), "Widget".to_string());

        let json = manifest.to_client_module_map_json();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let entries = parsed["entries"].as_object().unwrap();

        // ref1 should be present (has chunk URL)
        assert!(entries.contains_key("ref1"), "ref1 should be included");
        assert_eq!(entries["ref1"]["chunk_url"], "/Counter.js");

        // ref2 should be excluded (empty chunk URL)
        assert!(!entries.contains_key("ref2"), "ref2 should be excluded");
    }
}
