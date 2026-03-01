use rand::RngCore;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// Generate a random CSRF state token (32 bytes, hex-encoded = 64 chars).
pub fn generate_state() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Validate that a CSRF state matches the expected value.
///
/// Uses SHA-256 hashing followed by constant-time comparison via `subtle::ConstantTimeEq`
/// to prevent timing side-channel attacks.
pub fn validate_state(received: &str, expected: &str) -> bool {
    let received_hash = Sha256::digest(received.as_bytes());
    let expected_hash = Sha256::digest(expected.as_bytes());
    received_hash.ct_eq(&expected_hash).into()
}

/// Build a `Set-Cookie` header value for the CSRF state cookie.
pub fn csrf_state_cookie(state: &str, is_dev: bool) -> String {
    let mut cookie =
        format!("__rex_auth_state={state}; HttpOnly; SameSite=Lax; Path=/; Max-Age=600");
    if !is_dev {
        cookie.push_str("; Secure");
    }
    cookie
}

/// Build a `Set-Cookie` header value for the callback URL cookie.
///
/// The value is stored as-is (a validated relative path like `/dashboard`).
/// No URL-encoding — the cookie is HttpOnly and the value was already validated
/// by `is_safe_callback_url` (no CRLF, no protocol-relative, etc.).
pub fn callback_url_cookie(url: &str, is_dev: bool) -> String {
    let mut cookie =
        format!("__rex_callback_url={url}; HttpOnly; SameSite=Lax; Path=/; Max-Age=600");
    if !is_dev {
        cookie.push_str("; Secure");
    }
    cookie
}

/// Build a `Set-Cookie` header value that clears the callback URL cookie.
pub fn clear_callback_url_cookie(is_dev: bool) -> String {
    let mut cookie = "__rex_callback_url=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0".to_string();
    if !is_dev {
        cookie.push_str("; Secure");
    }
    cookie
}

/// Build a `Set-Cookie` header value that clears the CSRF state cookie.
pub fn clear_csrf_cookie(is_dev: bool) -> String {
    let mut cookie = "__rex_auth_state=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0".to_string();
    if !is_dev {
        cookie.push_str("; Secure");
    }
    cookie
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_state_length() {
        let state = generate_state();
        assert_eq!(state.len(), 64);
    }

    #[test]
    fn test_generate_state_uniqueness() {
        let s1 = generate_state();
        let s2 = generate_state();
        assert_ne!(s1, s2);
    }

    #[test]
    fn test_validate_state_matching() {
        let state = generate_state();
        assert!(validate_state(&state, &state));
    }

    #[test]
    fn test_validate_state_mismatch() {
        let s1 = generate_state();
        let s2 = generate_state();
        assert!(!validate_state(&s1, &s2));
    }

    #[test]
    fn test_csrf_cookie_dev() {
        let cookie = csrf_state_cookie("abc123", true);
        assert!(cookie.contains("__rex_auth_state=abc123"));
        assert!(cookie.contains("Max-Age=600"));
        assert!(!cookie.contains("Secure"));
    }

    #[test]
    fn test_csrf_cookie_prod() {
        let cookie = csrf_state_cookie("abc123", false);
        assert!(cookie.contains("Secure"));
    }

    #[test]
    fn test_callback_url_cookie() {
        let cookie = callback_url_cookie("/dashboard", true);
        assert!(cookie.contains("__rex_callback_url="));
        assert!(cookie.contains("Max-Age=600"));
    }

    #[test]
    fn test_clear_callback_url_cookie() {
        let cookie = clear_callback_url_cookie(true);
        assert!(cookie.contains("__rex_callback_url="));
        assert!(cookie.contains("Max-Age=0"));
    }
}
