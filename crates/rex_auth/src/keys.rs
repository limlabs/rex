// RSA 2048-bit key management for JWT signing.
//
// Key storage format: JWK in `.rex/auth/keys/active.json`
// Key ID (kid): SHA-256 of public key modulus, truncated to 8 hex chars
//
// Uses `rsa` crate for key generation and `jsonwebtoken` for JWT operations.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info};

use crate::AuthError;

/// A serializable RSA key pair with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredKeyPair {
    pub kid: String,
    /// PEM-encoded PKCS#8 private key
    pub private_key_pem: String,
    /// PEM-encoded public key
    pub public_key_pem: String,
    /// RSA modulus (n) base64url-encoded for JWK
    pub n: String,
    /// RSA exponent (e) base64url-encoded for JWK
    pub e: String,
    pub created_at: u64,
}

/// A public JWK for the JWKS endpoint (RFC 7517).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwkPublic {
    pub kty: String,
    pub alg: String,
    #[serde(rename = "use")]
    pub use_: String,
    pub kid: String,
    pub n: String,
    pub e: String,
}

/// Manages active and previous RSA key pairs for JWT signing.
///
/// Supports key rotation: the active key signs new tokens, while the
/// previous key is still accepted for validation (graceful rollover).
pub struct KeyManager {
    pub active: StoredKeyPair,
    pub previous: Option<StoredKeyPair>,
    keys_dir: PathBuf,
}

impl KeyManager {
    /// Load existing keys from disk, or generate a new key pair if none exist.
    pub fn load_or_generate(keys_dir: &Path) -> Result<Self, AuthError> {
        std::fs::create_dir_all(keys_dir)
            .map_err(|e| AuthError::Key(format!("failed to create keys dir: {e}")))?;

        let active_path = keys_dir.join("active.json");
        let previous_path = keys_dir.join("previous.json");

        let active = if active_path.exists() {
            debug!("loading active key from {}", active_path.display());
            let data = std::fs::read_to_string(&active_path)
                .map_err(|e| AuthError::Key(format!("failed to read active key: {e}")))?;
            serde_json::from_str(&data)
                .map_err(|e| AuthError::Key(format!("failed to parse active key: {e}")))?
        } else {
            info!("no active key found, generating new RSA 2048-bit key pair");
            let key = generate_rsa_keypair()?;
            let data = serde_json::to_string_pretty(&key)
                .map_err(|e| AuthError::Key(format!("failed to serialize key: {e}")))?;
            std::fs::write(&active_path, &data)
                .map_err(|e| AuthError::Key(format!("failed to write active key: {e}")))?;
            set_restrictive_permissions(&active_path)?;
            key
        };

        let previous = if previous_path.exists() {
            debug!("loading previous key from {}", previous_path.display());
            let data = std::fs::read_to_string(&previous_path)
                .map_err(|e| AuthError::Key(format!("failed to read previous key: {e}")))?;
            Some(
                serde_json::from_str(&data)
                    .map_err(|e| AuthError::Key(format!("failed to parse previous key: {e}")))?,
            )
        } else {
            None
        };

        Ok(Self {
            active,
            previous,
            keys_dir: keys_dir.to_path_buf(),
        })
    }

    /// Return the public JWK for the active key.
    pub fn active_jwk(&self) -> JwkPublic {
        to_jwk_public(&self.active)
    }

    /// Return all public JWKs (active + previous if present) for the JWKS endpoint.
    pub fn all_jwks(&self) -> Vec<JwkPublic> {
        let mut jwks = vec![to_jwk_public(&self.active)];
        if let Some(prev) = &self.previous {
            jwks.push(to_jwk_public(prev));
        }
        jwks
    }

    /// Get the `jsonwebtoken::EncodingKey` for the active key.
    pub fn encoding_key(&self) -> Result<jsonwebtoken::EncodingKey, AuthError> {
        jsonwebtoken::EncodingKey::from_rsa_pem(self.active.private_key_pem.as_bytes())
            .map_err(|e| AuthError::Key(format!("failed to create encoding key: {e}")))
    }

    /// Get all valid decoding keys (active + previous) paired with their kid.
    pub fn decoding_keys(&self) -> Result<Vec<(String, jsonwebtoken::DecodingKey)>, AuthError> {
        let mut keys = Vec::new();

        let active_dk =
            jsonwebtoken::DecodingKey::from_rsa_pem(self.active.public_key_pem.as_bytes())
                .map_err(|e| AuthError::Key(format!("failed to create decoding key: {e}")))?;
        keys.push((self.active.kid.clone(), active_dk));

        if let Some(prev) = &self.previous {
            let prev_dk = jsonwebtoken::DecodingKey::from_rsa_pem(prev.public_key_pem.as_bytes())
                .map_err(|e| {
                AuthError::Key(format!("failed to create previous decoding key: {e}"))
            })?;
            keys.push((prev.kid.clone(), prev_dk));
        }

        Ok(keys)
    }

    /// Rotate keys: current active becomes previous, a new key pair is generated.
    pub fn rotate(&mut self) -> Result<(), AuthError> {
        info!("rotating RSA key pair");

        // Move active to previous
        let prev_path = self.keys_dir.join("previous.json");
        let active_data = serde_json::to_string_pretty(&self.active)
            .map_err(|e| AuthError::Key(format!("failed to serialize key: {e}")))?;
        std::fs::write(&prev_path, &active_data)
            .map_err(|e| AuthError::Key(format!("failed to write previous key: {e}")))?;
        set_restrictive_permissions(&prev_path)?;
        self.previous = Some(self.active.clone());

        // Generate new active
        let new_key = generate_rsa_keypair()?;
        let new_data = serde_json::to_string_pretty(&new_key)
            .map_err(|e| AuthError::Key(format!("failed to serialize key: {e}")))?;
        let active_path = self.keys_dir.join("active.json");
        std::fs::write(&active_path, new_data)
            .map_err(|e| AuthError::Key(format!("failed to write active key: {e}")))?;
        set_restrictive_permissions(&active_path)?;
        self.active = new_key;

        Ok(())
    }

    /// Return the kid of the active key.
    pub fn active_kid(&self) -> &str {
        &self.active.kid
    }
}

/// Set restrictive file permissions (0o600) on sensitive files.
///
/// On non-Unix platforms this is a no-op.
fn set_restrictive_permissions(path: &Path) -> Result<(), AuthError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| {
            AuthError::Key(format!(
                "failed to set permissions on {}: {e}",
                path.display()
            ))
        })?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

/// Generate a new RSA 2048-bit key pair using the `rsa` crate (pure Rust).
fn generate_rsa_keypair() -> Result<StoredKeyPair, AuthError> {
    use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
    use rsa::traits::PublicKeyParts;
    use rsa::RsaPrivateKey;

    let mut rng = rand::thread_rng();
    let private_key = RsaPrivateKey::new(&mut rng, 2048)
        .map_err(|e| AuthError::Key(format!("RSA key generation failed: {e}")))?;
    let public_key = private_key.to_public_key();

    let private_key_pem = private_key
        .to_pkcs8_pem(LineEnding::LF)
        .map_err(|e| AuthError::Key(format!("failed to encode private key PEM: {e}")))?
        .to_string();

    let public_key_pem = public_key
        .to_public_key_pem(LineEnding::LF)
        .map_err(|e| AuthError::Key(format!("failed to encode public key PEM: {e}")))?;

    // Extract modulus (n) and exponent (e) for JWK
    let n_bytes = public_key.n().to_bytes_be();
    let e_bytes = public_key.e().to_bytes_be();
    let n = URL_SAFE_NO_PAD.encode(&n_bytes);
    let e = URL_SAFE_NO_PAD.encode(&e_bytes);

    // Compute kid: first 8 hex chars of SHA-256 of the modulus bytes
    let kid_hash = Sha256::digest(&n_bytes);
    let kid = hex::encode(&kid_hash[..4]); // 4 bytes = 8 hex chars

    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before UNIX epoch")
        .as_secs();

    Ok(StoredKeyPair {
        kid,
        private_key_pem,
        public_key_pem,
        n,
        e,
        created_at,
    })
}

/// Convert a `StoredKeyPair` into a public JWK.
fn to_jwk_public(key: &StoredKeyPair) -> JwkPublic {
    JwkPublic {
        kty: "RSA".to_string(),
        alg: "RS256".to_string(),
        use_: "sig".to_string(),
        kid: key.kid.clone(),
        n: key.n.clone(),
        e: key.e.clone(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_to_jwk_public() {
        let key = StoredKeyPair {
            kid: "abcd1234".to_string(),
            private_key_pem: String::new(),
            public_key_pem: String::new(),
            n: "test-modulus".to_string(),
            e: "AQAB".to_string(),
            created_at: 1000,
        };

        let jwk = to_jwk_public(&key);
        assert_eq!(jwk.kty, "RSA");
        assert_eq!(jwk.alg, "RS256");
        assert_eq!(jwk.use_, "sig");
        assert_eq!(jwk.kid, "abcd1234");
        assert_eq!(jwk.n, "test-modulus");
        assert_eq!(jwk.e, "AQAB");
    }

    #[test]
    fn test_generate_rsa_keypair() {
        let key = generate_rsa_keypair().expect("key generation failed");
        assert!(!key.private_key_pem.is_empty());
        assert!(!key.public_key_pem.is_empty());
        assert!(!key.n.is_empty());
        assert!(!key.e.is_empty());
        assert_eq!(key.kid.len(), 8);

        // Verify the PEM keys work with jsonwebtoken
        jsonwebtoken::EncodingKey::from_rsa_pem(key.private_key_pem.as_bytes())
            .expect("encoding key from generated PEM");
        jsonwebtoken::DecodingKey::from_rsa_pem(key.public_key_pem.as_bytes())
            .expect("decoding key from generated PEM");
    }

    #[test]
    fn test_generate_and_load_keypair() {
        let dir = std::env::temp_dir().join(format!("rex_keys_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        // First call generates
        let km = KeyManager::load_or_generate(&dir).expect("key generation failed");
        assert!(!km.active.kid.is_empty());
        assert_eq!(km.active.kid.len(), 8);
        assert!(km.previous.is_none());

        // Second call loads from disk
        let km2 = KeyManager::load_or_generate(&dir).expect("key load failed");
        assert_eq!(km2.active.kid, km.active.kid);

        // Encoding/decoding keys work
        let _ek = km2.encoding_key().expect("encoding key");
        let dks = km2.decoding_keys().expect("decoding keys");
        assert_eq!(dks.len(), 1);

        // JWKS
        let jwks = km2.all_jwks();
        assert_eq!(jwks.len(), 1);
        assert_eq!(jwks[0].kid, km.active.kid);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_key_rotation() {
        let dir = std::env::temp_dir().join(format!("rex_keys_rotate_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let mut km = KeyManager::load_or_generate(&dir).expect("key generation failed");
        let original_kid = km.active.kid.clone();

        km.rotate().expect("rotation failed");

        assert_ne!(km.active.kid, original_kid);
        assert!(km.previous.is_some());
        assert_eq!(km.previous.as_ref().unwrap().kid, original_kid);

        // Both keys available for decoding
        let dks = km.decoding_keys().expect("decoding keys");
        assert_eq!(dks.len(), 2);

        // JWKS includes both
        let jwks = km.all_jwks();
        assert_eq!(jwks.len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
