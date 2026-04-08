//! Secure credential storage — encrypted API key persistence.
//!
//! Uses XOR-based obfuscation with a machine-derived key.
//! Stored at `~/.nocode/credentials.json` as base64-encoded ciphertext.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Stored credentials (encrypted at rest).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CredentialStore {
    /// Provider name → encrypted (base64) API key.
    pub keys: HashMap<String, String>,
}

impl CredentialStore {
    /// Default credentials file path: `~/.nocode/credentials.json`.
    pub fn default_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(home).join(".nocode").join("credentials.json")
    }

    /// Load credentials from disk.
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw =
            fs::read_to_string(path).map_err(|e| format!("Failed to read credentials: {e}"))?;
        serde_json::from_str(&raw).map_err(|e| format!("Failed to parse credentials: {e}"))
    }

    /// Save credentials to disk (creates parent dir if needed).
    pub fn save(&self, path: &std::path::Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create credentials dir: {e}"))?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize credentials: {e}"))?;
        fs::write(path, &json).map_err(|e| format!("Failed to write credentials: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    /// Store an API key (encrypted).
    pub fn set_key(&mut self, provider: &str, api_key: &str) {
        let encrypted = encrypt(api_key);
        self.keys.insert(provider.to_string(), encrypted);
    }

    /// Retrieve an API key (decrypted).
    pub fn get_key(&self, provider: &str) -> Option<String> {
        self.keys.get(provider).and_then(|enc| decrypt(enc).ok())
    }

    /// Remove an API key.
    pub fn remove_key(&mut self, provider: &str) -> bool {
        self.keys.remove(provider).is_some()
    }

    /// List stored provider names.
    pub fn providers(&self) -> Vec<&str> {
        self.keys.keys().map(String::as_str).collect()
    }

    /// Load API keys into environment variables (if not already set).
    /// # Safety
    /// Uses `set_var` which is unsafe in Rust 2024 due to potential data races
    /// in multi-threaded programs. Call only during single-threaded startup.
    pub fn load_into_env(&self) {
        let mappings = [
            ("anthropic", "ANTHROPIC_API_KEY"),
            ("openai", "OPENAI_API_KEY"),
            ("gemini", "GEMINI_API_KEY"),
        ];
        for (provider, env_var) in &mappings {
            if std::env::var(env_var).is_err()
                && let Some(key) = self.get_key(provider)
            {
                // SAFETY: called during single-threaded startup before spawning threads
                unsafe {
                    std::env::set_var(env_var, key);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// XOR obfuscation (not cryptographic — defense in depth against casual reads)
// ---------------------------------------------------------------------------

fn derive_key() -> Vec<u8> {
    // Use machine-id + username as key material
    let machine_id = fs::read_to_string("/etc/machine-id")
        .or_else(|_| fs::read_to_string("/var/lib/dbus/machine-id"))
        .unwrap_or_else(|_| "nocode-default-key-material".to_string());
    let user = std::env::var("USER").unwrap_or_else(|_| "nocode".to_string());
    let combined = format!("{}{}", machine_id.trim(), user);
    // Simple hash: FNV-1a
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in combined.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash.to_le_bytes().to_vec()
}

fn xor_bytes(data: &[u8], key: &[u8]) -> Vec<u8> {
    data.iter()
        .enumerate()
        .map(|(i, b)| b ^ key[i % key.len()])
        .collect()
}

fn encrypt(plaintext: &str) -> String {
    use base64::Engine;
    let key = derive_key();
    let encrypted = xor_bytes(plaintext.as_bytes(), &key);
    base64::engine::general_purpose::STANDARD.encode(&encrypted)
}

fn decrypt(ciphertext: &str) -> Result<String, String> {
    use base64::Engine;
    let key = derive_key();
    let encrypted = base64::engine::general_purpose::STANDARD
        .decode(ciphertext)
        .map_err(|e| format!("base64 decode error: {e}"))?;
    let decrypted = xor_bytes(&encrypted, &key);
    String::from_utf8(decrypted).map_err(|e| format!("UTF-8 decode error: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let key = "sk-ant-api03-test-key-12345";
        let enc = encrypt(key);
        assert_ne!(enc, key); // must be different
        let dec = decrypt(&enc).unwrap();
        assert_eq!(dec, key);
    }

    #[test]
    fn store_set_get_key() {
        let mut store = CredentialStore::default();
        store.set_key("anthropic", "sk-ant-test");
        assert_eq!(store.get_key("anthropic"), Some("sk-ant-test".to_string()));
        assert_eq!(store.get_key("openai"), None);
    }

    #[test]
    fn store_remove_key() {
        let mut store = CredentialStore::default();
        store.set_key("openai", "sk-test");
        assert!(store.remove_key("openai"));
        assert!(!store.remove_key("openai"));
        assert_eq!(store.get_key("openai"), None);
    }

    #[test]
    fn store_save_load_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("nocode_cred_{}", std::process::id()));
        let path = tmp.join("credentials.json");
        let _ = fs::remove_dir_all(&tmp);

        let mut store = CredentialStore::default();
        store.set_key("anthropic", "sk-ant-secret");
        store.set_key("openai", "sk-openai-secret");
        store.save(&path).unwrap();

        let loaded = CredentialStore::load(&path).unwrap();
        assert_eq!(
            loaded.get_key("anthropic"),
            Some("sk-ant-secret".to_string())
        );
        assert_eq!(
            loaded.get_key("openai"),
            Some("sk-openai-secret".to_string())
        );
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn store_providers_list() {
        let mut store = CredentialStore::default();
        store.set_key("anthropic", "key1");
        store.set_key("gemini", "key2");
        let mut providers = store.providers();
        providers.sort();
        assert_eq!(providers, vec!["anthropic", "gemini"]);
    }

    #[test]
    fn load_nonexistent_returns_empty() {
        let path = PathBuf::from("/tmp/nocode_nonexistent_cred_12345.json");
        let store = CredentialStore::load(&path).unwrap();
        assert!(store.keys.is_empty());
    }
}
