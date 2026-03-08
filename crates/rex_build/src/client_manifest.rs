// Data types live in rex_core for decoupling. Re-exported here for
// backward compatibility.
pub use rex_core::client_manifest::{ClientRefEntry, ClientReferenceManifest};

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
}
