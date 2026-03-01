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
            std::fs::write(&active_path, data)
                .map_err(|e| AuthError::Key(format!("failed to write active key: {e}")))?;
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
        self.previous = Some(self.active.clone());

        // Generate new active
        let new_key = generate_rsa_keypair()?;
        let new_data = serde_json::to_string_pretty(&new_key)
            .map_err(|e| AuthError::Key(format!("failed to serialize key: {e}")))?;
        let active_path = self.keys_dir.join("active.json");
        std::fs::write(&active_path, new_data)
            .map_err(|e| AuthError::Key(format!("failed to write active key: {e}")))?;
        self.active = new_key;

        Ok(())
    }

    /// Return the kid of the active key.
    pub fn active_kid(&self) -> &str {
        &self.active.kid
    }
}

/// Generate a new RSA 2048-bit key pair.
///
/// Uses the `openssl` CLI to generate a PKCS#8 private key and extract
/// the public key. This avoids pulling in the heavy `rsa` crate while
/// still producing standards-compliant keys.
///
/// If `openssl` is not available, returns an error with instructions.
fn generate_rsa_keypair() -> Result<StoredKeyPair, AuthError> {
    // Generate private key in PKCS#8 PEM format
    let output = std::process::Command::new("openssl")
        .args([
            "genpkey",
            "-algorithm",
            "RSA",
            "-pkeyopt",
            "rsa_keygen_bits:2048",
        ])
        .output()
        .map_err(|e| {
            AuthError::Key(format!(
                "failed to run openssl for key generation: {e}. \
                 Ensure openssl is installed and available in PATH."
            ))
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AuthError::Key(format!(
            "openssl key generation failed: {stderr}"
        )));
    }

    let private_key_pem = String::from_utf8(output.stdout)
        .map_err(|e| AuthError::Key(format!("invalid UTF-8 in private key: {e}")))?;

    // Extract public key from private key
    let output = std::process::Command::new("openssl")
        .args(["pkey", "-pubout"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .as_mut()
                .unwrap()
                .write_all(private_key_pem.as_bytes())?;
            child.wait_with_output()
        })
        .map_err(|e| AuthError::Key(format!("failed to extract public key: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AuthError::Key(format!(
            "openssl public key extraction failed: {stderr}"
        )));
    }

    let public_key_pem = String::from_utf8(output.stdout)
        .map_err(|e| AuthError::Key(format!("invalid UTF-8 in public key: {e}")))?;

    // Extract RSA modulus (n) and exponent (e) for JWK
    let (n, e) = extract_rsa_components(&private_key_pem)?;

    // Compute kid: first 8 hex chars of SHA-256 of the modulus bytes
    let n_bytes = URL_SAFE_NO_PAD
        .decode(&n)
        .map_err(|e| AuthError::Key(format!("failed to decode modulus: {e}")))?;
    let kid_hash = Sha256::digest(&n_bytes);
    let kid = hex::encode(&kid_hash[..4]); // 4 bytes = 8 hex chars

    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
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

/// Extract RSA modulus (n) and exponent (e) from a PEM private key using openssl.
fn extract_rsa_components(private_key_pem: &str) -> Result<(String, String), AuthError> {
    // Use openssl to output the RSA key components in text form
    let output = std::process::Command::new("openssl")
        .args(["pkey", "-text", "-noout"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .as_mut()
                .unwrap()
                .write_all(private_key_pem.as_bytes())?;
            child.wait_with_output()
        })
        .map_err(|e| AuthError::Key(format!("failed to extract RSA components: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AuthError::Key(format!(
            "openssl RSA component extraction failed: {stderr}"
        )));
    }

    let text = String::from_utf8(output.stdout)
        .map_err(|e| AuthError::Key(format!("invalid UTF-8 in key text: {e}")))?;

    let n = parse_openssl_bignum(&text, "modulus:")
        .ok_or_else(|| AuthError::Key("failed to parse modulus from openssl output".into()))?;
    let e = parse_openssl_exponent(&text)
        .ok_or_else(|| AuthError::Key("failed to parse exponent from openssl output".into()))?;

    Ok((n, e))
}

/// Parse a big number field (like modulus) from openssl text output.
///
/// The openssl output format is:
/// ```text
/// modulus:
///     00:ab:cd:ef:...
///     12:34:...
/// ```
fn parse_openssl_bignum(text: &str, field: &str) -> Option<String> {
    let mut lines = text.lines();
    // Find the field line
    loop {
        let line = lines.next()?;
        if line.trim().starts_with(field) {
            break;
        }
    }

    // Collect hex bytes from subsequent indented lines
    let mut hex_bytes = Vec::new();
    for line in lines {
        let trimmed = line.trim();
        // Stop when we hit a non-hex line (next field)
        if trimmed.is_empty() || (!trimmed.contains(':') && !trimmed.ends_with(':')) {
            // Check if it's a continuation of hex
            if trimmed.chars().all(|c| c.is_ascii_hexdigit() || c == ':') && !trimmed.is_empty() {
                for part in trimmed.split(':') {
                    let part = part.trim();
                    if !part.is_empty() {
                        if let Ok(byte) = u8::from_str_radix(part, 16) {
                            hex_bytes.push(byte);
                        }
                    }
                }
                continue;
            }
            break;
        }
        for part in trimmed.split(':') {
            let part = part.trim();
            if !part.is_empty() {
                if let Ok(byte) = u8::from_str_radix(part, 16) {
                    hex_bytes.push(byte);
                }
            }
        }
    }

    // Strip leading zero byte (ASN.1 sign byte)
    if hex_bytes.first() == Some(&0) && hex_bytes.len() > 1 {
        hex_bytes.remove(0);
    }

    if hex_bytes.is_empty() {
        return None;
    }

    Some(URL_SAFE_NO_PAD.encode(&hex_bytes))
}

/// Parse the public exponent from openssl text output.
///
/// Format: `publicExponent: 65537 (0x10001)`
fn parse_openssl_exponent(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("publicExponent:") || trimmed.starts_with("Exponent:") {
            // Extract the decimal number
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() >= 2 {
                if let Ok(exp) = parts[1].parse::<u64>() {
                    // Encode as big-endian bytes, stripping leading zeros
                    let bytes = exp.to_be_bytes();
                    let start = bytes.iter().position(|&b| b != 0).unwrap_or(7);
                    return Some(URL_SAFE_NO_PAD.encode(&bytes[start..]));
                }
            }
        }
    }
    None
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
    fn test_parse_openssl_exponent() {
        let text = "publicExponent: 65537 (0x10001)\n";
        let e = parse_openssl_exponent(text).unwrap();
        // 65537 = 0x010001, base64url of [1, 0, 1] = "AQAB"
        assert_eq!(e, "AQAB");
    }

    #[test]
    fn test_parse_openssl_bignum() {
        let text = "modulus:\n    00:ab:cd:ef:01:23\n";
        let n = parse_openssl_bignum(text, "modulus:").unwrap();
        // After stripping leading 00: bytes are [0xab, 0xcd, 0xef, 0x01, 0x23]
        let decoded = URL_SAFE_NO_PAD.decode(&n).unwrap();
        assert_eq!(decoded, vec![0xab, 0xcd, 0xef, 0x01, 0x23]);
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
