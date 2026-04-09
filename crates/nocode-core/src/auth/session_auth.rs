//! Session ingress auth — token-based authentication for bridge sessions.
//!
//! Validates bearer tokens, binds sessions to authenticated identities,
//! and enforces token expiration.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// An authenticated session identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionIdentity {
    pub token_hash: String,
    pub user_id: Option<String>,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub scopes: Vec<String>,
}

/// Token validation result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthResult {
    /// Token is valid.
    Authenticated(String), // session_id
    /// Token is expired.
    Expired,
    /// Token is invalid/unknown.
    Invalid,
    /// No token provided.
    Missing,
}

/// Session auth store — maps tokens to session identities.
pub struct SessionAuthStore {
    /// token_hash → identity
    identities: HashMap<String, SessionIdentity>,
    /// session_id → token_hash
    sessions: HashMap<String, String>,
}

impl SessionAuthStore {
    pub fn new() -> Self {
        Self {
            identities: HashMap::new(),
            sessions: HashMap::new(),
        }
    }
// APPEND_REST

    /// Register a token for a session.
    pub fn register(
        &mut self,
        token: &str,
        session_id: &str,
        user_id: Option<&str>,
        ttl_secs: Option<i64>,
    ) {
        let hash = hash_token(token);
        let now = chrono::Utc::now().timestamp();
        let identity = SessionIdentity {
            token_hash: hash.clone(),
            user_id: user_id.map(String::from),
            created_at: now,
            expires_at: ttl_secs.map(|ttl| now + ttl),
            scopes: vec!["*".to_string()],
        };
        self.identities.insert(hash.clone(), identity);
        self.sessions.insert(session_id.to_string(), hash);
    }

    /// Validate a bearer token. Returns the bound session ID if valid.
    pub fn validate(&self, token: &str) -> AuthResult {
        if token.is_empty() {
            return AuthResult::Missing;
        }
        let hash = hash_token(token);
        let Some(identity) = self.identities.get(&hash) else {
            return AuthResult::Invalid;
        };
        // Check expiration
        if let Some(expires_at) = identity.expires_at {
            let now = chrono::Utc::now().timestamp();
            if now > expires_at {
                return AuthResult::Expired;
            }
        }
        // Find session bound to this token
        for (session_id, token_hash) in &self.sessions {
            if token_hash == &hash {
                return AuthResult::Authenticated(session_id.clone());
            }
        }
        AuthResult::Invalid
    }

    /// Revoke a token.
    pub fn revoke(&mut self, token: &str) -> bool {
        let hash = hash_token(token);
        let removed = self.identities.remove(&hash).is_some();
        self.sessions.retain(|_, v| v != &hash);
        removed
    }

    /// Revoke all tokens for a session.
    pub fn revoke_session(&mut self, session_id: &str) -> bool {
        if let Some(hash) = self.sessions.remove(session_id) {
            self.identities.remove(&hash);
            true
        } else {
            false
        }
    }

    /// List all active sessions.
    pub fn active_sessions(&self) -> Vec<(&str, &SessionIdentity)> {
        let now = chrono::Utc::now().timestamp();
        self.sessions
            .iter()
            .filter_map(|(sid, hash)| {
                let identity = self.identities.get(hash)?;
                if let Some(exp) = identity.expires_at
                    && now > exp
                {
                    return None;
                }
                Some((sid.as_str(), identity))
            })
            .collect()
    }

    /// Clean up expired tokens.
    pub fn cleanup_expired(&mut self) -> usize {
        let now = chrono::Utc::now().timestamp();
        let expired: Vec<String> = self
            .identities
            .iter()
            .filter(|(_, id)| id.expires_at.is_some_and(|exp| now > exp))
            .map(|(hash, _)| hash.clone())
            .collect();
        let count = expired.len();
        for hash in &expired {
            self.identities.remove(hash);
            self.sessions.retain(|_, v| v != hash);
        }
        count
    }
}

impl Default for SessionAuthStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Simple FNV-1a hash of a token string (for storage, not crypto).
fn hash_token(token: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in token.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_validate() {
        let mut store = SessionAuthStore::new();
        store.register("token-abc", "sess-1", Some("user-1"), None);
        assert_eq!(
            store.validate("token-abc"),
            AuthResult::Authenticated("sess-1".to_string())
        );
    }

    #[test]
    fn invalid_token() {
        let store = SessionAuthStore::new();
        assert_eq!(store.validate("bogus"), AuthResult::Invalid);
    }

    #[test]
    fn missing_token() {
        let store = SessionAuthStore::new();
        assert_eq!(store.validate(""), AuthResult::Missing);
    }

    #[test]
    fn expired_token() {
        let mut store = SessionAuthStore::new();
        store.register("token-exp", "sess-2", None, Some(-1)); // already expired
        assert_eq!(store.validate("token-exp"), AuthResult::Expired);
    }

    #[test]
    fn revoke_token() {
        let mut store = SessionAuthStore::new();
        store.register("token-rev", "sess-3", None, None);
        assert!(store.revoke("token-rev"));
        assert_eq!(store.validate("token-rev"), AuthResult::Invalid);
        assert!(!store.revoke("token-rev")); // already revoked
    }

    #[test]
    fn revoke_session() {
        let mut store = SessionAuthStore::new();
        store.register("token-s", "sess-4", None, None);
        assert!(store.revoke_session("sess-4"));
        assert_eq!(store.validate("token-s"), AuthResult::Invalid);
    }

    #[test]
    fn active_sessions_excludes_expired() {
        let mut store = SessionAuthStore::new();
        store.register("t1", "s1", None, Some(3600));
        store.register("t2", "s2", None, Some(-1)); // expired
        let active = store.active_sessions();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].0, "s1");
    }

    #[test]
    fn cleanup_expired() {
        let mut store = SessionAuthStore::new();
        store.register("t1", "s1", None, Some(-1));
        store.register("t2", "s2", None, Some(-1));
        store.register("t3", "s3", None, Some(3600));
        let cleaned = store.cleanup_expired();
        assert_eq!(cleaned, 2);
        assert_eq!(store.validate("t3"), AuthResult::Authenticated("s3".to_string()));
    }

    #[test]
    fn hash_token_deterministic() {
        assert_eq!(hash_token("test"), hash_token("test"));
        assert_ne!(hash_token("a"), hash_token("b"));
    }

    #[test]
    fn identity_serde_roundtrip() {
        let id = SessionIdentity {
            token_hash: "abc".to_string(),
            user_id: Some("user-1".to_string()),
            created_at: 1000,
            expires_at: Some(2000),
            scopes: vec!["read".to_string()],
        };
        let json = serde_json::to_string(&id).unwrap();
        let parsed: SessionIdentity = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.user_id.as_deref(), Some("user-1"));
        assert_eq!(parsed.scopes, vec!["read"]);
    }
}
