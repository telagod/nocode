//! LSP registry — global singleton for LSP server management.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// LSP server state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LspState {
    Disconnected,
    Connecting,
    Connected,
    Failed,
}

/// A registered LSP server entry.
#[derive(Debug, Clone)]
pub struct LspEntry {
    pub name: String,
    pub language: String,
    pub state: LspState,
    pub root_uri: Option<String>,
    pub capabilities: Vec<String>,
}

impl LspEntry {
    pub fn new(name: &str, language: &str) -> Self {
        Self {
            name: name.to_string(),
            language: language.to_string(),
            state: LspState::Disconnected,
            root_uri: None,
            capabilities: Vec::new(),
        }
    }
}

/// Registry tracking LSP servers.
pub struct LspRegistry {
    servers: HashMap<String, LspEntry>,
}

impl LspRegistry {
    pub fn new() -> Self {
        Self {
            servers: HashMap::new(),
        }
    }

    pub fn register(&mut self, name: &str, language: &str) {
        self.servers
            .insert(name.to_string(), LspEntry::new(name, language));
    }

    pub fn set_state(&mut self, name: &str, state: LspState) {
        if let Some(entry) = self.servers.get_mut(name) {
            entry.state = state;
        }
    }

    pub fn set_connected(&mut self, name: &str, root_uri: &str, capabilities: Vec<String>) {
        if let Some(entry) = self.servers.get_mut(name) {
            entry.state = LspState::Connected;
            entry.root_uri = Some(root_uri.to_string());
            entry.capabilities = capabilities;
        }
    }

    pub fn get(&self, name: &str) -> Option<&LspEntry> {
        self.servers.get(name)
    }

    pub fn list(&self) -> Vec<&LspEntry> {
        self.servers.values().collect()
    }

    pub fn connected(&self) -> Vec<&LspEntry> {
        self.servers
            .values()
            .filter(|e| e.state == LspState::Connected)
            .collect()
    }

    pub fn remove(&mut self, name: &str) -> Option<LspEntry> {
        self.servers.remove(name)
    }
}

impl Default for LspRegistry {
    fn default() -> Self {
        Self::new()
    }
}

static GLOBAL_LSP_REGISTRY: OnceLock<Arc<Mutex<LspRegistry>>> = OnceLock::new();

pub fn global_lsp_registry() -> &'static Arc<Mutex<LspRegistry>> {
    GLOBAL_LSP_REGISTRY.get_or_init(|| Arc::new(Mutex::new(LspRegistry::new())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_connect() {
        let mut reg = LspRegistry::new();
        reg.register("rust-analyzer", "rust");
        assert_eq!(
            reg.get("rust-analyzer").unwrap().state,
            LspState::Disconnected
        );

        reg.set_connected(
            "rust-analyzer",
            "file:///project",
            vec!["completion".to_string(), "hover".to_string()],
        );
        let entry = reg.get("rust-analyzer").unwrap();
        assert_eq!(entry.state, LspState::Connected);
        assert_eq!(entry.capabilities.len(), 2);
    }

    #[test]
    fn connected_filter() {
        let mut reg = LspRegistry::new();
        reg.register("ra", "rust");
        reg.register("tsserver", "typescript");
        reg.set_state("ra", LspState::Connected);
        assert_eq!(reg.connected().len(), 1);
    }

    #[test]
    fn remove_server() {
        let mut reg = LspRegistry::new();
        reg.register("test", "go");
        assert!(reg.get("test").is_some());
        reg.remove("test");
        assert!(reg.get("test").is_none());
    }
}
