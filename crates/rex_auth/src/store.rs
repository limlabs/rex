use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::debug;

use crate::AuthError;

/// A registered OAuth2 client (public client, no secret).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisteredClient {
    pub client_id: String,
    pub client_name: String,
    pub redirect_uris: Vec<String>,
    pub created_at: u64,
}

/// An in-memory authorization code (not persisted, acceptable to lose on restart).
#[derive(Debug, Clone)]
pub struct AuthCode {
    pub code: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub subject: String,
    pub scope: String,
    pub code_challenge: String,
    pub created_at: u64,
}

/// A persisted refresh token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredRefreshToken {
    pub token: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub subject: String,
    pub scope: String,
    pub created_at: u64,
    pub expires_at: u64,
}

/// A persisted consent decision (user granted scopes to a client).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsentDecision {
    pub subject: String,
    pub client_id: String,
    pub scopes: Vec<String>,
    pub granted_at: u64,
}

/// JSON-file-backed store for OAuth2 server state.
///
/// Auth codes are in-memory only (HashMap with TTL).
/// Clients, refresh tokens, and consent decisions persist to JSON files.
///
/// **Note:** The `RwLock` only protects against concurrent access within a single
/// process. If multiple Rex instances share the same `.rex/auth/` directory (e.g.,
/// behind a load balancer), they may race on the JSON files. For multi-process
/// deployments, use a shared database backend or ensure each instance has its own
/// store directory.
pub struct FileStore {
    dir: PathBuf,
    clients: RwLock<HashMap<String, RegisteredClient>>,
    auth_codes: RwLock<HashMap<String, AuthCode>>,
    refresh_tokens: RwLock<HashMap<String, StoredRefreshToken>>,
    consents: RwLock<HashMap<String, ConsentDecision>>,
}

/// Maximum age for auth codes: 10 minutes.
const AUTH_CODE_TTL_SECS: u64 = 600;

impl FileStore {
    /// Create a new FileStore, loading existing data from disk.
    pub fn new(dir: &Path) -> Result<Self, AuthError> {
        std::fs::create_dir_all(dir)
            .map_err(|e| AuthError::Store(format!("failed to create store dir: {e}")))?;

        let clients = load_json_map(&dir.join("clients.json"))?;
        let refresh_tokens = load_json_map(&dir.join("refresh_tokens.json"))?;
        let consents = load_json_map(&dir.join("consents.json"))?;

        Ok(Self {
            dir: dir.to_path_buf(),
            clients: RwLock::new(clients),
            auth_codes: RwLock::new(HashMap::new()),
            refresh_tokens: RwLock::new(refresh_tokens),
            consents: RwLock::new(consents),
        })
    }

    // ── Client CRUD ──────────────────────────────────────────────────

    /// Register a new client. Returns the generated client_id.
    pub fn register_client(
        &self,
        name: String,
        redirect_uris: Vec<String>,
    ) -> Result<RegisteredClient, AuthError> {
        let client_id = generate_client_id();
        let now = now_secs();

        let client = RegisteredClient {
            client_id: client_id.clone(),
            client_name: name,
            redirect_uris,
            created_at: now,
        };

        {
            let mut clients = self.clients.write().map_err(lock_error)?;
            clients.insert(client_id, client.clone());
        }

        self.flush_clients()?;
        debug!("registered client: {}", client.client_id);
        Ok(client)
    }

    /// Get a client by ID.
    pub fn get_client(&self, client_id: &str) -> Result<RegisteredClient, AuthError> {
        let clients = self.clients.read().map_err(lock_error)?;
        clients
            .get(client_id)
            .cloned()
            .ok_or_else(|| AuthError::ClientNotFound(client_id.to_string()))
    }

    /// List all registered clients.
    pub fn list_clients(&self) -> Result<Vec<RegisteredClient>, AuthError> {
        let clients = self.clients.read().map_err(lock_error)?;
        Ok(clients.values().cloned().collect())
    }

    // ── Auth Codes (in-memory only) ──────────────────────────────────

    /// Store an authorization code. Returns the generated code string.
    pub fn store_auth_code(
        &self,
        client_id: String,
        redirect_uri: String,
        subject: String,
        scope: String,
        code_challenge: String,
    ) -> Result<String, AuthError> {
        // Clean expired codes first
        self.clean_expired_codes()?;

        let code = generate_auth_code();
        let now = now_secs();

        let auth_code = AuthCode {
            code: code.clone(),
            client_id,
            redirect_uri,
            subject,
            scope,
            code_challenge,
            created_at: now,
        };

        {
            let mut codes = self.auth_codes.write().map_err(lock_error)?;
            codes.insert(code.clone(), auth_code);
        }

        debug!("stored auth code (expires in {}s)", AUTH_CODE_TTL_SECS);
        Ok(code)
    }

    /// Consume an authorization code (get and delete atomically).
    ///
    /// Returns `None` if the code doesn't exist or has expired.
    pub fn consume_auth_code(&self, code: &str) -> Result<Option<AuthCode>, AuthError> {
        let mut codes = self.auth_codes.write().map_err(lock_error)?;
        let auth_code = codes.remove(code);

        match auth_code {
            Some(ac) => {
                let now = now_secs();
                if now - ac.created_at > AUTH_CODE_TTL_SECS {
                    debug!("auth code expired, discarding");
                    Ok(None)
                } else {
                    Ok(Some(ac))
                }
            }
            None => Ok(None),
        }
    }

    /// Remove expired auth codes from memory.
    fn clean_expired_codes(&self) -> Result<(), AuthError> {
        let now = now_secs();
        let mut codes = self.auth_codes.write().map_err(lock_error)?;
        let before = codes.len();
        codes.retain(|_, ac| now - ac.created_at <= AUTH_CODE_TTL_SECS);
        let removed = before - codes.len();
        if removed > 0 {
            debug!("cleaned {removed} expired auth codes");
        }
        Ok(())
    }

    // ── Refresh Tokens ───────────────────────────────────────────────

    /// Store a refresh token. Returns the generated token string.
    pub fn store_refresh_token(
        &self,
        client_id: String,
        redirect_uri: String,
        subject: String,
        scope: String,
        ttl_secs: u64,
    ) -> Result<String, AuthError> {
        let token = generate_refresh_token();
        let now = now_secs();

        let stored = StoredRefreshToken {
            token: token.clone(),
            client_id,
            redirect_uri,
            subject,
            scope,
            created_at: now,
            expires_at: now + ttl_secs,
        };

        {
            let mut tokens = self.refresh_tokens.write().map_err(lock_error)?;
            tokens.insert(token.clone(), stored);
        }

        self.flush_refresh_tokens()?;
        Ok(token)
    }

    /// Get a refresh token if it exists and has not expired.
    pub fn get_refresh_token(&self, token: &str) -> Result<Option<StoredRefreshToken>, AuthError> {
        let tokens = self.refresh_tokens.read().map_err(lock_error)?;
        match tokens.get(token) {
            Some(rt) => {
                let now = now_secs();
                if now >= rt.expires_at {
                    Ok(None) // expired
                } else {
                    Ok(Some(rt.clone()))
                }
            }
            None => Ok(None),
        }
    }

    /// Revoke (delete) a refresh token.
    pub fn revoke_refresh_token(&self, token: &str) -> Result<bool, AuthError> {
        let removed = {
            let mut tokens = self.refresh_tokens.write().map_err(lock_error)?;
            tokens.remove(token).is_some()
        };

        if removed {
            self.flush_refresh_tokens()?;
        }

        Ok(removed)
    }

    // ── Consent Decisions ────────────────────────────────────────────

    /// Record that a user granted consent for specific scopes to a client.
    pub fn store_consent(
        &self,
        subject: String,
        client_id: String,
        scopes: Vec<String>,
    ) -> Result<(), AuthError> {
        let now = now_secs();
        let key = consent_key(&subject, &client_id);

        let decision = ConsentDecision {
            subject,
            client_id,
            scopes,
            granted_at: now,
        };

        {
            let mut consents = self.consents.write().map_err(lock_error)?;
            consents.insert(key, decision);
        }

        self.flush_consents()?;
        Ok(())
    }

    /// Check if a user has previously consented to the requested scopes for a client.
    pub fn check_consent(
        &self,
        subject: &str,
        client_id: &str,
        requested_scopes: &[String],
    ) -> Result<bool, AuthError> {
        let key = consent_key(subject, client_id);
        let consents = self.consents.read().map_err(lock_error)?;

        match consents.get(&key) {
            Some(decision) => {
                // All requested scopes must be in the previously granted set
                let all_granted = requested_scopes
                    .iter()
                    .all(|s| decision.scopes.iter().any(|g| g == s));
                Ok(all_granted)
            }
            None => Ok(false),
        }
    }

    // ── Flush to disk ────────────────────────────────────────────────

    fn flush_clients(&self) -> Result<(), AuthError> {
        let clients = self.clients.read().map_err(lock_error)?;
        write_json_map(&self.dir.join("clients.json"), &*clients)
    }

    fn flush_refresh_tokens(&self) -> Result<(), AuthError> {
        let tokens = self.refresh_tokens.read().map_err(lock_error)?;
        write_json_map(&self.dir.join("refresh_tokens.json"), &*tokens)
    }

    fn flush_consents(&self) -> Result<(), AuthError> {
        let consents = self.consents.read().map_err(lock_error)?;
        write_json_map(&self.dir.join("consents.json"), &*consents)
    }
}

// ── ID Generation ────────────────────────────────────────────────────

/// Generate a client ID: `rex_` + 12 random alphanumeric characters.
fn generate_client_id() -> String {
    let mut bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut bytes);
    let alphanum: String = bytes
        .iter()
        .map(|b| {
            let idx = (*b as usize) % 36;
            if idx < 10 {
                (b'0' + idx as u8) as char
            } else {
                (b'a' + (idx - 10) as u8) as char
            }
        })
        .collect();
    format!("rex_{alphanum}")
}

/// Generate an authorization code: 32 random bytes, hex-encoded (64 chars).
fn generate_auth_code() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Generate a refresh token: 48 random bytes, hex-encoded (96 chars).
fn generate_refresh_token() -> String {
    let mut bytes = [0u8; 48];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

// ── Helpers ──────────────────────────────────────────────────────────

fn consent_key(subject: &str, client_id: &str) -> String {
    format!("{subject}:{client_id}")
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before UNIX epoch")
        .as_secs()
}

fn lock_error<T>(_: T) -> AuthError {
    AuthError::Store("lock poisoned".to_string())
}

/// Load a JSON file into a HashMap, returning empty map if file doesn't exist.
fn load_json_map<V: serde::de::DeserializeOwned>(
    path: &Path,
) -> Result<HashMap<String, V>, AuthError> {
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let data = std::fs::read_to_string(path)
        .map_err(|e| AuthError::Store(format!("failed to read {}: {e}", path.display())))?;

    if data.trim().is_empty() {
        return Ok(HashMap::new());
    }

    serde_json::from_str(&data)
        .map_err(|e| AuthError::Store(format!("failed to parse {}: {e}", path.display())))
}

/// Write a HashMap to a JSON file.
fn write_json_map<V: Serialize>(path: &Path, map: &HashMap<String, V>) -> Result<(), AuthError> {
    let data = serde_json::to_string_pretty(map)
        .map_err(|e| AuthError::Store(format!("failed to serialize: {e}")))?;

    std::fs::write(path, data)
        .map_err(|e| AuthError::Store(format!("failed to write {}: {e}", path.display())))?;

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn temp_store() -> (FileStore, PathBuf) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("rex_store_test_{}_{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = FileStore::new(&dir).expect("failed to create store");
        (store, dir)
    }

    #[test]
    fn test_generate_client_id_format() {
        let id = generate_client_id();
        assert!(id.starts_with("rex_"));
        assert_eq!(id.len(), 16); // "rex_" (4) + 12 chars
        assert!(id[4..].chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn test_generate_client_id_uniqueness() {
        let id1 = generate_client_id();
        let id2 = generate_client_id();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_generate_auth_code_format() {
        let code = generate_auth_code();
        assert_eq!(code.len(), 64); // 32 bytes hex
        assert!(code.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_generate_refresh_token_format() {
        let token = generate_refresh_token();
        assert_eq!(token.len(), 96); // 48 bytes hex
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_client_crud() {
        let (store, dir) = temp_store();

        let client = store
            .register_client(
                "Test App".to_string(),
                vec!["http://localhost:3000/callback".to_string()],
            )
            .unwrap();

        assert!(client.client_id.starts_with("rex_"));
        assert_eq!(client.client_name, "Test App");

        // Get by ID
        let fetched = store.get_client(&client.client_id).unwrap();
        assert_eq!(fetched.client_name, "Test App");

        // List
        let all = store.list_clients().unwrap();
        assert_eq!(all.len(), 1);

        // Not found
        let err = store.get_client("rex_nonexistent").unwrap_err();
        assert!(matches!(err, AuthError::ClientNotFound(_)));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_client_persistence() {
        let dir = std::env::temp_dir().join(format!("rex_store_persist_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        // Create and register
        let client_id = {
            let store = FileStore::new(&dir).unwrap();
            let client = store
                .register_client("Persistent App".to_string(), vec![])
                .unwrap();
            client.client_id
        };

        // Reload and verify
        let store2 = FileStore::new(&dir).unwrap();
        let client = store2.get_client(&client_id).unwrap();
        assert_eq!(client.client_name, "Persistent App");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_auth_code_store_and_consume() {
        let (store, dir) = temp_store();

        let code = store
            .store_auth_code(
                "rex_client1".to_string(),
                "http://localhost/cb".to_string(),
                "user-1".to_string(),
                "tools:read".to_string(),
                "challenge-abc".to_string(),
            )
            .unwrap();

        // Consume returns the code
        let ac = store.consume_auth_code(&code).unwrap();
        assert!(ac.is_some());
        let ac = ac.unwrap();
        assert_eq!(ac.client_id, "rex_client1");
        assert_eq!(ac.subject, "user-1");

        // Second consume returns None (already consumed)
        let ac2 = store.consume_auth_code(&code).unwrap();
        assert!(ac2.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_auth_code_nonexistent() {
        let (store, dir) = temp_store();

        let result = store.consume_auth_code("nonexistent").unwrap();
        assert!(result.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_refresh_token_lifecycle() {
        let (store, dir) = temp_store();

        let token = store
            .store_refresh_token(
                "rex_client1".to_string(),
                "http://localhost/cb".to_string(),
                "user-1".to_string(),
                "tools:read".to_string(),
                3600, // 1 hour TTL
            )
            .unwrap();

        // Get valid token
        let rt = store.get_refresh_token(&token).unwrap();
        assert!(rt.is_some());
        assert_eq!(rt.unwrap().subject, "user-1");

        // Revoke
        let revoked = store.revoke_refresh_token(&token).unwrap();
        assert!(revoked);

        // Get after revoke returns None
        let rt2 = store.get_refresh_token(&token).unwrap();
        assert!(rt2.is_none());

        // Revoking again returns false
        let revoked2 = store.revoke_refresh_token(&token).unwrap();
        assert!(!revoked2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_consent_store_and_check() {
        let (store, dir) = temp_store();

        // No prior consent
        let has = store
            .check_consent("user-1", "rex_client1", &["tools:read".to_string()])
            .unwrap();
        assert!(!has);

        // Grant consent
        store
            .store_consent(
                "user-1".to_string(),
                "rex_client1".to_string(),
                vec!["tools:read".to_string(), "tools:execute".to_string()],
            )
            .unwrap();

        // Check subset — should pass
        let has = store
            .check_consent("user-1", "rex_client1", &["tools:read".to_string()])
            .unwrap();
        assert!(has);

        // Check full set — should pass
        let has = store
            .check_consent(
                "user-1",
                "rex_client1",
                &["tools:read".to_string(), "tools:execute".to_string()],
            )
            .unwrap();
        assert!(has);

        // Check with extra scope — should fail
        let has = store
            .check_consent(
                "user-1",
                "rex_client1",
                &["tools:read".to_string(), "admin".to_string()],
            )
            .unwrap();
        assert!(!has);

        // Different client — should fail
        let has = store
            .check_consent("user-1", "rex_other", &["tools:read".to_string()])
            .unwrap();
        assert!(!has);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_consent_persistence() {
        let dir = std::env::temp_dir().join(format!("rex_store_consent_p_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        {
            let store = FileStore::new(&dir).unwrap();
            store
                .store_consent(
                    "user-1".to_string(),
                    "rex_c1".to_string(),
                    vec!["scope1".to_string()],
                )
                .unwrap();
        }

        let store2 = FileStore::new(&dir).unwrap();
        let has = store2
            .check_consent("user-1", "rex_c1", &["scope1".to_string()])
            .unwrap();
        assert!(has);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_empty_store_operations() {
        let (store, dir) = temp_store();

        assert!(store.list_clients().unwrap().is_empty());
        assert!(store.get_refresh_token("nope").unwrap().is_none());
        assert!(store.consume_auth_code("nope").unwrap().is_none());
        assert!(!store.revoke_refresh_token("nope").unwrap());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
