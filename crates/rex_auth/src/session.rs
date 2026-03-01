use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};

/// Session payload stored in the encrypted cookie.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionData {
    pub user: UserProfile,
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    /// Expiry timestamp (Unix seconds).
    pub expires: u64,
}

/// Normalized user profile from OAuth provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfile {
    /// Provider-specific unique ID (e.g., "github|12345").
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}

/// Encrypt a session payload using AES-256-GCM.
///
/// Output format: `base64url(nonce_12b || ciphertext || tag_16b)`
pub fn encrypt_session(data: &SessionData, key: &[u8; 32]) -> Result<String, crate::AuthError> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| crate::AuthError::Session(format!("cipher init: {e}")))?;

    let plaintext = serde_json::to_vec(data)
        .map_err(|e| crate::AuthError::Session(format!("serialize: {e}")))?;

    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_ref())
        .map_err(|e| crate::AuthError::Session(format!("encrypt: {e}")))?;

    // nonce || ciphertext (includes tag)
    let mut combined = Vec::with_capacity(12 + ciphertext.len());
    combined.extend_from_slice(&nonce_bytes);
    combined.extend_from_slice(&ciphertext);

    Ok(URL_SAFE_NO_PAD.encode(&combined))
}

/// Decrypt a session cookie value using AES-256-GCM.
///
/// Returns `None` if decryption fails (tampered, wrong key, etc.).
pub fn decrypt_session(encoded: &str, key: &[u8; 32]) -> Option<SessionData> {
    let combined = URL_SAFE_NO_PAD.decode(encoded).ok()?;

    // Need at least nonce (12) + tag (16) + 1 byte ciphertext
    if combined.len() < 29 {
        return None;
    }

    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);

    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    let plaintext = cipher.decrypt(nonce, ciphertext).ok()?;

    let data: SessionData = serde_json::from_slice(&plaintext).ok()?;

    // Check expiry
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    if data.expires <= now {
        return None;
    }

    Some(data)
}

/// Build a `Set-Cookie` header value for the session cookie.
pub fn session_cookie(cookie_name: &str, value: &str, max_age: u64, is_dev: bool) -> String {
    let mut cookie =
        format!("{cookie_name}={value}; HttpOnly; SameSite=Lax; Path=/; Max-Age={max_age}");
    if !is_dev {
        cookie.push_str("; Secure");
    }
    cookie
}

/// Build a `Set-Cookie` header value that clears the session cookie.
pub fn clear_session_cookie(cookie_name: &str, is_dev: bool) -> String {
    let mut cookie = format!("{cookie_name}=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0");
    if !is_dev {
        cookie.push_str("; Secure");
    }
    cookie
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn test_key() -> [u8; 32] {
        crate::config::derive_key("test-secret-key")
    }

    fn make_session() -> SessionData {
        SessionData {
            user: UserProfile {
                id: "github|12345".to_string(),
                name: Some("Test User".to_string()),
                email: Some("test@example.com".to_string()),
                image: None,
            },
            provider: "github".to_string(),
            access_token: Some("gho_abc123".to_string()),
            expires: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 86400,
        }
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let key = test_key();
        let session = make_session();

        let encrypted = encrypt_session(&session, &key).unwrap();
        let decrypted = decrypt_session(&encrypted, &key).unwrap();

        assert_eq!(decrypted.user.id, "github|12345");
        assert_eq!(decrypted.provider, "github");
        assert_eq!(decrypted.access_token.as_deref(), Some("gho_abc123"));
    }

    #[test]
    fn test_decrypt_tampered_cookie() {
        let key = test_key();
        let session = make_session();
        let encrypted = encrypt_session(&session, &key).unwrap();

        // Tamper with the encrypted data
        let mut bytes = URL_SAFE_NO_PAD.decode(&encrypted).unwrap();
        if let Some(b) = bytes.get_mut(20) {
            *b ^= 0xFF;
        }
        let tampered = URL_SAFE_NO_PAD.encode(&bytes);

        assert!(decrypt_session(&tampered, &key).is_none());
    }

    #[test]
    fn test_decrypt_wrong_key() {
        let key1 = crate::config::derive_key("key-one");
        let key2 = crate::config::derive_key("key-two");
        let session = make_session();

        let encrypted = encrypt_session(&session, &key1).unwrap();
        assert!(decrypt_session(&encrypted, &key2).is_none());
    }

    #[test]
    fn test_decrypt_truncated() {
        let key = test_key();
        assert!(decrypt_session("abc", &key).is_none());
        assert!(decrypt_session("", &key).is_none());
    }

    #[test]
    fn test_decrypt_expired_session() {
        let key = test_key();
        let session = SessionData {
            expires: 1000, // Far in the past
            ..make_session()
        };

        let encrypted = encrypt_session(&session, &key).unwrap();
        assert!(decrypt_session(&encrypted, &key).is_none());
    }

    #[test]
    fn test_session_cookie_dev() {
        let cookie = session_cookie("__rex_session", "value", 86400, true);
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Lax"));
        assert!(!cookie.contains("Secure"));
    }

    #[test]
    fn test_session_cookie_prod() {
        let cookie = session_cookie("__rex_session", "value", 86400, false);
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("Secure"));
    }

    #[test]
    fn test_clear_session_cookie() {
        let cookie = clear_session_cookie("__rex_session", false);
        assert!(cookie.contains("Max-Age=0"));
        assert!(cookie.contains("Secure"));
    }

    #[test]
    fn test_decrypt_random_bytes() {
        let key = test_key();
        // Various random/malformed inputs should never panic
        for input in &[
            "",
            "x",
            "aGVsbG8",
            "////",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        ] {
            let _ = decrypt_session(input, &key);
        }
    }
}
