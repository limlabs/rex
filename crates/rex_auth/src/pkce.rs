use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// Verify a PKCE S256 code challenge against a code verifier.
///
/// Computes `BASE64URL(SHA256(code_verifier))` and compares to `code_challenge`
/// using constant-time comparison to prevent timing side-channel attacks.
pub fn verify_pkce_s256(code_verifier: &str, code_challenge: &str) -> bool {
    let hash = Sha256::digest(code_verifier.as_bytes());
    let computed = URL_SAFE_NO_PAD.encode(hash);
    computed.as_bytes().ct_eq(code_challenge.as_bytes()).into()
}

/// Generate a PKCE S256 code challenge from a verifier.
pub fn compute_challenge_s256(code_verifier: &str) -> String {
    let hash = Sha256::digest(code_verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_pkce_pair() {
        // RFC 7636 Appendix B test vector
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = compute_challenge_s256(verifier);
        assert!(verify_pkce_s256(verifier, &challenge));
    }

    #[test]
    fn test_invalid_verifier() {
        let verifier = "correct-verifier-value";
        let challenge = compute_challenge_s256(verifier);
        assert!(!verify_pkce_s256("wrong-verifier-value", &challenge));
    }

    #[test]
    fn test_empty_verifier() {
        let challenge = compute_challenge_s256("");
        assert!(verify_pkce_s256("", &challenge));
    }

    #[test]
    fn test_empty_challenge_mismatch() {
        assert!(!verify_pkce_s256("some-verifier", ""));
    }

    #[test]
    fn test_challenge_is_base64url_no_padding() {
        let challenge = compute_challenge_s256("test-verifier-123");
        // Base64url should not contain +, /, or = characters
        assert!(!challenge.contains('+'));
        assert!(!challenge.contains('/'));
        assert!(!challenge.contains('='));
    }

    #[test]
    fn test_challenge_deterministic() {
        let verifier = "same-verifier";
        let c1 = compute_challenge_s256(verifier);
        let c2 = compute_challenge_s256(verifier);
        assert_eq!(c1, c2);
    }

    #[test]
    fn test_rfc7636_appendix_b() {
        // From RFC 7636 Appendix B:
        // verifier = dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk
        // expected challenge = E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let expected = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert_eq!(compute_challenge_s256(verifier), expected);
        assert!(verify_pkce_s256(verifier, expected));
    }
}
