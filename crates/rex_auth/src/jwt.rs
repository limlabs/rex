use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::AuthError;

/// Claims for an OAuth2 access token (JWT).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessTokenClaims {
    /// Issuer — the Rex server URL.
    pub iss: String,
    /// Subject — the authenticated user identifier.
    pub sub: String,
    /// Audience — typically the client_id or resource server.
    pub aud: String,
    /// Expiration time (seconds since epoch).
    pub exp: u64,
    /// Issued at (seconds since epoch).
    pub iat: u64,
    /// JWT ID — unique identifier for this token.
    pub jti: String,
    /// Space-delimited scope string.
    pub scope: String,
    /// The OAuth2 client that requested this token.
    pub client_id: String,
}

impl AccessTokenClaims {
    /// Check that the token's scope includes the required scope.
    ///
    /// Scope is space-delimited per RFC 6749.
    pub fn require_scope(&self, required: &str) -> Result<(), AuthError> {
        let scopes: Vec<&str> = self.scope.split_whitespace().collect();
        if scopes.contains(&required) {
            Ok(())
        } else {
            Err(AuthError::InsufficientScope {
                required: required.to_string(),
                have: self.scope.clone(),
            })
        }
    }

    /// Check whether the token has expired.
    pub fn is_expired(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.exp <= now
    }
}

/// Sign an access token JWT using RS256.
///
/// The `kid` is included in the JWT header so clients can look up the correct
/// verification key from the JWKS endpoint.
pub fn sign_access_token(
    claims: &AccessTokenClaims,
    encoding_key: &EncodingKey,
    kid: &str,
) -> Result<String, AuthError> {
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(kid.to_string());

    jsonwebtoken::encode(&header, claims, encoding_key)
        .map_err(|e| AuthError::Key(format!("failed to sign access token: {e}")))
}

/// Validate an access token JWT using RS256.
///
/// Tries each provided decoding key (to support key rotation). Verifies:
/// - Algorithm is RS256 (rejects none, HS256, etc.)
/// - Token has not expired
/// - Issuer matches expected value
///
/// Returns the validated claims on success.
pub fn validate_access_token(
    token: &str,
    decoding_keys: &[(String, DecodingKey)],
    issuer: &str,
) -> Result<AccessTokenClaims, AuthError> {
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&[issuer]);
    // We validate audience manually since it varies per request
    validation.validate_aud = false;
    // exp is validated automatically by jsonwebtoken

    let mut last_error = None;

    for (_kid, key) in decoding_keys {
        match jsonwebtoken::decode::<AccessTokenClaims>(token, key, &validation) {
            Ok(token_data) => return Ok(token_data.claims),
            Err(e) => {
                last_error = Some(e);
                continue;
            }
        }
    }

    match last_error {
        Some(e) => match e.kind() {
            jsonwebtoken::errors::ErrorKind::ExpiredSignature => Err(AuthError::TokenExpired),
            jsonwebtoken::errors::ErrorKind::InvalidIssuer => Err(AuthError::InvalidToken),
            _ => Err(AuthError::InvalidToken),
        },
        None => Err(AuthError::InvalidToken),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn test_claims(exp: u64) -> AccessTokenClaims {
        AccessTokenClaims {
            iss: "https://example.com".to_string(),
            sub: "user-123".to_string(),
            aud: "client-abc".to_string(),
            exp,
            iat: 1000,
            jti: "token-id-1".to_string(),
            scope: "tools:read tools:execute".to_string(),
            client_id: "client-abc".to_string(),
        }
    }

    #[test]
    fn test_require_scope_present() {
        let claims = test_claims(u64::MAX);
        assert!(claims.require_scope("tools:read").is_ok());
        assert!(claims.require_scope("tools:execute").is_ok());
    }

    #[test]
    fn test_require_scope_missing() {
        let claims = test_claims(u64::MAX);
        let err = claims.require_scope("admin").unwrap_err();
        match err {
            AuthError::InsufficientScope { required, have } => {
                assert_eq!(required, "admin");
                assert_eq!(have, "tools:read tools:execute");
            }
            other => panic!("expected InsufficientScope, got: {other:?}"),
        }
    }

    #[test]
    fn test_is_expired() {
        let expired_claims = test_claims(0);
        assert!(expired_claims.is_expired());

        let valid_claims = test_claims(u64::MAX);
        assert!(!valid_claims.is_expired());
    }

    #[test]
    fn test_claims_serialization() {
        let claims = test_claims(9999999999);
        let json = serde_json::to_string(&claims).unwrap();
        let deserialized: AccessTokenClaims = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.sub, "user-123");
        assert_eq!(deserialized.scope, "tools:read tools:execute");
        assert_eq!(deserialized.client_id, "client-abc");
    }

    #[test]
    fn test_sign_and_validate_roundtrip() {
        // Generate a test RSA key pair using openssl
        let output = std::process::Command::new("openssl")
            .args([
                "genpkey",
                "-algorithm",
                "RSA",
                "-pkeyopt",
                "rsa_keygen_bits:2048",
            ])
            .output()
            .expect("openssl must be available for tests");

        let private_pem = String::from_utf8(output.stdout).unwrap();

        let pub_output = std::process::Command::new("openssl")
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
                    .write_all(private_pem.as_bytes())
                    .unwrap();
                child.wait_with_output()
            })
            .unwrap();

        let public_pem = String::from_utf8(pub_output.stdout).unwrap();

        let encoding_key = EncodingKey::from_rsa_pem(private_pem.as_bytes()).unwrap();
        let decoding_key = DecodingKey::from_rsa_pem(public_pem.as_bytes()).unwrap();

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let claims = AccessTokenClaims {
            iss: "https://rex.test".to_string(),
            sub: "user-42".to_string(),
            aud: "client-xyz".to_string(),
            exp: now + 3600,
            iat: now,
            jti: "test-jti-001".to_string(),
            scope: "tools:read".to_string(),
            client_id: "client-xyz".to_string(),
        };

        let token = sign_access_token(&claims, &encoding_key, "test-kid").unwrap();

        // Validate with correct issuer
        let keys = vec![("test-kid".to_string(), decoding_key)];
        let validated = validate_access_token(&token, &keys, "https://rex.test").unwrap();
        assert_eq!(validated.sub, "user-42");
        assert_eq!(validated.scope, "tools:read");
        assert_eq!(validated.client_id, "client-xyz");
    }

    #[test]
    fn test_validate_wrong_issuer() {
        let output = std::process::Command::new("openssl")
            .args([
                "genpkey",
                "-algorithm",
                "RSA",
                "-pkeyopt",
                "rsa_keygen_bits:2048",
            ])
            .output()
            .expect("openssl must be available for tests");

        let private_pem = String::from_utf8(output.stdout).unwrap();

        let pub_output = std::process::Command::new("openssl")
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
                    .write_all(private_pem.as_bytes())
                    .unwrap();
                child.wait_with_output()
            })
            .unwrap();

        let public_pem = String::from_utf8(pub_output.stdout).unwrap();

        let encoding_key = EncodingKey::from_rsa_pem(private_pem.as_bytes()).unwrap();
        let decoding_key = DecodingKey::from_rsa_pem(public_pem.as_bytes()).unwrap();

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let claims = test_claims(now + 3600);

        let token = sign_access_token(&claims, &encoding_key, "kid").unwrap();

        let keys = vec![("kid".to_string(), decoding_key)];
        let result = validate_access_token(&token, &keys, "https://wrong-issuer.com");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_expired_token() {
        let output = std::process::Command::new("openssl")
            .args([
                "genpkey",
                "-algorithm",
                "RSA",
                "-pkeyopt",
                "rsa_keygen_bits:2048",
            ])
            .output()
            .expect("openssl must be available for tests");

        let private_pem = String::from_utf8(output.stdout).unwrap();

        let pub_output = std::process::Command::new("openssl")
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
                    .write_all(private_pem.as_bytes())
                    .unwrap();
                child.wait_with_output()
            })
            .unwrap();

        let public_pem = String::from_utf8(pub_output.stdout).unwrap();

        let encoding_key = EncodingKey::from_rsa_pem(private_pem.as_bytes()).unwrap();
        let decoding_key = DecodingKey::from_rsa_pem(public_pem.as_bytes()).unwrap();

        // Token expired 1 hour ago
        let claims = test_claims(1);

        let token = sign_access_token(&claims, &encoding_key, "kid").unwrap();

        let keys = vec![("kid".to_string(), decoding_key)];
        let result = validate_access_token(&token, &keys, "https://example.com");
        assert!(matches!(result, Err(AuthError::TokenExpired)));
    }

    #[test]
    fn test_validate_no_keys() {
        let result = validate_access_token("some.jwt.token", &[], "https://example.com");
        assert!(matches!(result, Err(AuthError::InvalidToken)));
    }
}
