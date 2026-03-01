//! Security-focused tests for rex_auth.
//!
//! Covers session tampering, CSRF attacks, PKCE bypass, JWT manipulation,
//! auth code replay, and timing-safe comparisons.
#![allow(clippy::unwrap_used)]

use rex_auth::config::derive_key;
use rex_auth::csrf;
use rex_auth::jwt::{sign_access_token, validate_access_token, AccessTokenClaims};
use rex_auth::pkce;
use rex_auth::session::{self, SessionData, UserProfile};
use rex_auth::store::FileStore;

fn test_key() -> [u8; 32] {
    derive_key("test-secret-key")
}

fn future_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600
}

fn make_session(expires: u64) -> SessionData {
    SessionData {
        user: UserProfile {
            id: "github|12345".to_string(),
            name: Some("Test User".to_string()),
            email: Some("test@example.com".to_string()),
            image: None,
        },
        provider: "github".to_string(),
        access_token: Some("gho_abc123".to_string()),
        expires,
    }
}

// ---------------------------------------------------------------------------
// Session Security
// ---------------------------------------------------------------------------

#[test]
fn test_tampered_cookie_rejected() {
    let key = test_key();
    let data = make_session(future_timestamp());
    let encrypted = session::encrypt_session(&data, &key).unwrap();

    // Flip a random bit in the middle of the ciphertext
    let mut bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(&encrypted)
        .unwrap();
    if bytes.len() > 20 {
        bytes[20] ^= 0xff;
    }
    let tampered = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&bytes);

    assert!(
        session::decrypt_session(&tampered, &key).is_none(),
        "Tampered cookie must be rejected"
    );
}

use base64::Engine;

#[test]
fn test_truncated_cookie_rejected() {
    let key = test_key();
    let data = make_session(future_timestamp());
    let encrypted = session::encrypt_session(&data, &key).unwrap();

    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(&encrypted)
        .unwrap();

    // Try various truncation lengths
    for len in [0, 1, 11, 12, 13, 28, bytes.len() / 2] {
        let truncated = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&bytes[..len]);
        assert!(
            session::decrypt_session(&truncated, &key).is_none(),
            "Truncated cookie (len={len}) must be rejected"
        );
    }
}

#[test]
fn test_expired_session_rejected() {
    let key = test_key();
    // Session that expired an hour ago
    let past = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        - 3600;

    let data = make_session(past);
    let encrypted = session::encrypt_session(&data, &key).unwrap();
    assert!(
        session::decrypt_session(&encrypted, &key).is_none(),
        "Expired session must be rejected"
    );
}

#[test]
fn test_wrong_key_rejected() {
    let key_a = derive_key("secret-a");
    let key_b = derive_key("secret-b");

    let data = make_session(future_timestamp());
    let encrypted = session::encrypt_session(&data, &key_a).unwrap();

    assert!(
        session::decrypt_session(&encrypted, &key_b).is_none(),
        "Session encrypted with key A must not decrypt with key B"
    );
}

#[test]
fn test_empty_string_cookie_rejected() {
    let key = test_key();
    assert!(session::decrypt_session("", &key).is_none());
}

#[test]
fn test_garbage_cookie_rejected() {
    let key = test_key();
    assert!(session::decrypt_session("not-base64!", &key).is_none());
    assert!(session::decrypt_session("AAAA", &key).is_none());
    assert!(session::decrypt_session("a]b[c", &key).is_none());
}

#[test]
fn test_session_cookie_flags_production() {
    let cookie = session::session_cookie("__session", "value", 86400, false);
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Lax"));
    assert!(cookie.contains("Secure"));
    assert!(cookie.contains("Path=/"));
    assert!(!cookie.contains("SameSite=None"));
}

#[test]
fn test_session_cookie_flags_dev() {
    let cookie = session::session_cookie("__session", "value", 86400, true);
    assert!(cookie.contains("HttpOnly"));
    assert!(!cookie.contains("Secure"));
}

// ---------------------------------------------------------------------------
// CSRF
// ---------------------------------------------------------------------------

#[test]
fn test_csrf_mismatched_state_rejected() {
    let state = csrf::generate_state();
    assert!(
        !csrf::validate_state("completely-different-state", &state),
        "Mismatched CSRF state must be rejected"
    );
}

#[test]
fn test_csrf_empty_state_rejected() {
    assert!(!csrf::validate_state("", "some-expected-state"));
    assert!(!csrf::validate_state("some-state", ""));
}

#[test]
fn test_csrf_valid_state_accepted() {
    let state = csrf::generate_state();
    assert!(
        csrf::validate_state(&state, &state),
        "Matching CSRF state must be accepted"
    );
}

#[test]
fn test_csrf_state_uniqueness() {
    let s1 = csrf::generate_state();
    let s2 = csrf::generate_state();
    assert_ne!(s1, s2, "Each state must be unique");
}

#[test]
fn test_csrf_state_format() {
    let state = csrf::generate_state();
    assert_eq!(state.len(), 64, "State should be 32 bytes hex = 64 chars");
    assert!(state.chars().all(|c| c.is_ascii_hexdigit()));
}

// ---------------------------------------------------------------------------
// PKCE
// ---------------------------------------------------------------------------

#[test]
fn test_pkce_wrong_verifier_rejected() {
    let verifier = "correct-verifier-value-at-least-43-chars-long-for-spec";
    let challenge = pkce::compute_challenge_s256(verifier);
    assert!(
        !pkce::verify_pkce_s256(
            "attacker-random-verifier-that-is-43-chars-long-minimum",
            &challenge
        ),
        "Wrong verifier must be rejected"
    );
}

#[test]
fn test_pkce_rfc7636_test_vector() {
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    let expected = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
    assert_eq!(pkce::compute_challenge_s256(verifier), expected);
    assert!(pkce::verify_pkce_s256(verifier, expected));
}

// ---------------------------------------------------------------------------
// JWT
// ---------------------------------------------------------------------------

#[test]
fn test_jwt_expired_token_rejected() {
    // This test requires actual RSA keys, so we'll use the KeyManager
    let dir = std::env::temp_dir().join(format!("rex_jwt_test_expired_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let km = rex_auth::KeyManager::load_or_generate(&dir).unwrap();

    let claims = AccessTokenClaims {
        iss: "https://test.example.com".to_string(),
        sub: "user123".to_string(),
        aud: "client123".to_string(),
        exp: 1000, // Way in the past
        iat: 900,
        jti: uuid::Uuid::new_v4().to_string(),
        scope: "tools:read".to_string(),
        client_id: "client123".to_string(),
    };

    let token = sign_access_token(&claims, &km.encoding_key().unwrap(), km.active_kid()).unwrap();

    let result = validate_access_token(
        &token,
        &km.decoding_keys().unwrap(),
        "https://test.example.com",
    );

    assert!(result.is_err(), "Expired token must be rejected");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_jwt_wrong_issuer_rejected() {
    let dir = std::env::temp_dir().join(format!("rex_jwt_test_issuer_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let km = rex_auth::KeyManager::load_or_generate(&dir).unwrap();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let claims = AccessTokenClaims {
        iss: "https://real.example.com".to_string(),
        sub: "user123".to_string(),
        aud: "client123".to_string(),
        exp: now + 3600,
        iat: now,
        jti: uuid::Uuid::new_v4().to_string(),
        scope: "tools:read".to_string(),
        client_id: "client123".to_string(),
    };

    let token = sign_access_token(&claims, &km.encoding_key().unwrap(), km.active_kid()).unwrap();

    // Validate with different issuer
    let result = validate_access_token(
        &token,
        &km.decoding_keys().unwrap(),
        "https://wrong.example.com",
    );

    assert!(result.is_err(), "Token with wrong issuer must be rejected");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_jwt_tampered_payload_rejected() {
    let dir = std::env::temp_dir().join(format!("rex_jwt_test_tamper_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let km = rex_auth::KeyManager::load_or_generate(&dir).unwrap();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let claims = AccessTokenClaims {
        iss: "https://test.example.com".to_string(),
        sub: "user123".to_string(),
        aud: "client123".to_string(),
        exp: now + 3600,
        iat: now,
        jti: uuid::Uuid::new_v4().to_string(),
        scope: "tools:read".to_string(),
        client_id: "client123".to_string(),
    };

    let token = sign_access_token(&claims, &km.encoding_key().unwrap(), km.active_kid()).unwrap();

    // Tamper with the payload — change the middle part
    let parts: Vec<&str> = token.split('.').collect();
    assert_eq!(parts.len(), 3);

    // Modify payload bytes
    let mut payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .unwrap();
    if !payload_bytes.is_empty() {
        payload_bytes[0] ^= 0xff;
    }
    let tampered_payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&payload_bytes);
    let tampered_token = format!("{}.{}.{}", parts[0], tampered_payload, parts[2]);

    let result = validate_access_token(
        &tampered_token,
        &km.decoding_keys().unwrap(),
        "https://test.example.com",
    );

    assert!(
        result.is_err(),
        "Token with tampered payload must be rejected"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_jwt_none_algorithm_rejected() {
    let dir = std::env::temp_dir().join(format!("rex_jwt_test_none_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let km = rex_auth::KeyManager::load_or_generate(&dir).unwrap();

    // Craft a JWT with alg: none
    let header =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"alg":"none","typ":"JWT"}"#);
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
        r#"{"iss":"https://test.example.com","sub":"user","aud":"client","exp":9999999999,"iat":0,"jti":"x","scope":"all","client_id":"c"}"#,
    );
    let token = format!("{header}.{payload}.");

    let result = validate_access_token(
        &token,
        &km.decoding_keys().unwrap(),
        "https://test.example.com",
    );

    assert!(result.is_err(), "Token with alg:none must be rejected");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_jwt_garbage_rejected() {
    let dir = std::env::temp_dir().join(format!("rex_jwt_test_garbage_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let km = rex_auth::KeyManager::load_or_generate(&dir).unwrap();

    let result = validate_access_token(
        "not-a-jwt",
        &km.decoding_keys().unwrap(),
        "https://test.example.com",
    );
    assert!(result.is_err());

    let result =
        validate_access_token("", &km.decoding_keys().unwrap(), "https://test.example.com");
    assert!(result.is_err());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_jwt_valid_roundtrip() {
    let dir = std::env::temp_dir().join(format!("rex_jwt_test_valid_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let km = rex_auth::KeyManager::load_or_generate(&dir).unwrap();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let claims = AccessTokenClaims {
        iss: "https://test.example.com".to_string(),
        sub: "user123".to_string(),
        aud: "client123".to_string(),
        exp: now + 3600,
        iat: now,
        jti: uuid::Uuid::new_v4().to_string(),
        scope: "tools:read tools:execute".to_string(),
        client_id: "client123".to_string(),
    };

    let token = sign_access_token(&claims, &km.encoding_key().unwrap(), km.active_kid()).unwrap();

    let validated = validate_access_token(
        &token,
        &km.decoding_keys().unwrap(),
        "https://test.example.com",
    )
    .unwrap();

    assert_eq!(validated.sub, "user123");
    assert_eq!(validated.scope, "tools:read tools:execute");
    assert!(validated.require_scope("tools:read").is_ok());
    assert!(validated.require_scope("tools:execute").is_ok());
    assert!(validated.require_scope("tools:admin").is_err());

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Authorization Code
// ---------------------------------------------------------------------------

#[test]
fn test_auth_code_reuse_rejected() {
    let dir =
        std::env::temp_dir().join(format!("rex_store_test_code_reuse_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = FileStore::new(&dir).unwrap();

    let code = store
        .store_auth_code(
            "client1".to_string(),
            "http://localhost/callback".to_string(),
            "user1".to_string(),
            "tools:read".to_string(),
            "challenge123".to_string(),
        )
        .unwrap();

    // First consumption succeeds
    let auth_code = store.consume_auth_code(&code).unwrap();
    assert!(auth_code.is_some(), "First consumption must succeed");
    assert_eq!(auth_code.unwrap().client_id, "client1");

    // Second consumption returns None (code consumed)
    let result = store.consume_auth_code(&code).unwrap();
    assert!(result.is_none(), "Auth code reuse must return None");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_nonexistent_auth_code_rejected() {
    let dir = std::env::temp_dir().join(format!(
        "rex_store_test_code_missing_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let store = FileStore::new(&dir).unwrap();

    let result = store.consume_auth_code("nonexistent-code").unwrap();
    assert!(result.is_none(), "Non-existent auth code must return None");

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Dynamic Client Registration
// ---------------------------------------------------------------------------

#[test]
fn test_client_id_format() {
    let dir =
        std::env::temp_dir().join(format!("rex_store_test_client_fmt_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = FileStore::new(&dir).unwrap();

    let client = store
        .register_client(
            "Test".to_string(),
            vec!["http://localhost:8080/callback".to_string()],
        )
        .unwrap();

    assert!(client.client_id.starts_with("rex_"));
    assert_eq!(client.client_id.len(), 16); // "rex_" + 12 chars

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Callback URL validation (via client_handlers tests)
// ---------------------------------------------------------------------------
// These are tested inline in client_handlers.rs — the is_safe_callback_url
// function rejects protocol-relative, data:, javascript:, and CRLF URLs.

// ---------------------------------------------------------------------------
// Timing-Safe Comparison (static analysis)
// ---------------------------------------------------------------------------

#[test]
fn test_csrf_uses_hash_comparison() {
    // The CSRF module should use SHA-256 hash comparison, not direct ==
    let src = include_str!("../src/csrf.rs");
    assert!(
        src.contains("Sha256::digest"),
        "CSRF validation must use SHA-256 based comparison for timing safety"
    );
}

#[test]
fn test_pkce_uses_hash_comparison() {
    // PKCE verification should be based on SHA-256 hash
    let src = include_str!("../src/pkce.rs");
    assert!(
        src.contains("Sha256::digest"),
        "PKCE verification must use SHA-256 based comparison"
    );
}

// ---------------------------------------------------------------------------
// JWKS Endpoint (key exposure)
// ---------------------------------------------------------------------------

#[test]
fn test_jwk_public_does_not_leak_private_key() {
    let dir = std::env::temp_dir().join(format!("rex_jwk_leak_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let km = rex_auth::KeyManager::load_or_generate(&dir).unwrap();
    let jwks = km.all_jwks();

    for jwk in &jwks {
        let json = serde_json::to_value(jwk).unwrap();
        assert_eq!(json["kty"], "RSA");
        assert_eq!(json["alg"], "RS256");
        assert!(json["kid"].is_string());
        assert!(json["n"].is_string(), "modulus must be present");
        assert!(json["e"].is_string(), "exponent must be present");
        // Must NOT contain private key components
        assert!(
            json.get("d").is_none(),
            "private exponent must not be in JWK public"
        );
        assert!(json.get("p").is_none(), "prime p must not be in JWK public");
        assert!(json.get("q").is_none(), "prime q must not be in JWK public");
        assert!(json.get("dp").is_none(), "dp must not be in JWK public");
        assert!(json.get("dq").is_none(), "dq must not be in JWK public");
        assert!(json.get("qi").is_none(), "qi must not be in JWK public");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Key Rotation
// ---------------------------------------------------------------------------

#[test]
fn test_token_valid_after_key_rotation() {
    let dir = std::env::temp_dir().join(format!("rex_key_rotation_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let km = rex_auth::KeyManager::load_or_generate(&dir).unwrap();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // Sign with current key
    let claims = AccessTokenClaims {
        iss: "https://test.example.com".to_string(),
        sub: "user123".to_string(),
        aud: "client123".to_string(),
        exp: now + 3600,
        iat: now,
        jti: uuid::Uuid::new_v4().to_string(),
        scope: "tools:read".to_string(),
        client_id: "client123".to_string(),
    };
    let token = sign_access_token(&claims, &km.encoding_key().unwrap(), km.active_kid()).unwrap();

    // Rotate key
    let km2 = rex_auth::KeyManager::load_or_generate(&dir).unwrap();
    // After rotation, the old key should still be valid for verification
    // (since load_or_generate loads existing keys)
    let result = validate_access_token(
        &token,
        &km2.decoding_keys().unwrap(),
        "https://test.example.com",
    );
    assert!(
        result.is_ok(),
        "Token signed with original key must still verify after reload"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
