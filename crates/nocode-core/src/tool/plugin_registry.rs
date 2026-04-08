//! Plugin registry — lifecycle management for tool plugins.
//!
//! Plugins follow an explicit state machine:
//! Unconfigured → Validated → Healthy/Degraded/Failed

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// Plugin lifecycle states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginState {
    Unconfigured,
    Validated,
    Healthy,
    Degraded,
    Failed,
}

impl PluginState {
    pub fn is_active(self) -> bool {
        matches!(self, Self::Healthy | Self::Degraded)
    }
}

/// A registered plugin entry.
#[derive(Debug, Clone)]
pub struct PluginEntry {
    pub name: String,
    pub version: String,
    pub state: PluginState,
    pub error: Option<String>,
    pub tool_count: usize,
}

impl PluginEntry {
    pub fn new(name: &str, version: &str) -> Self {
        Self {
            name: name.to_string(),
            version: version.to_string(),
            state: PluginState::Unconfigured,
            error: None,
            tool_count: 0,
        }
    }
}

/// Registry managing all plugins and their lifecycles.
pub struct PluginRegistry {
    plugins: HashMap<String, PluginEntry>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
        }
    }

    pub fn register(&mut self, name: &str, version: &str) {
        self.plugins
            .insert(name.to_string(), PluginEntry::new(name, version));
    }

    pub fn validate(&mut self, name: &str) -> Result<(), String> {
        let entry = self
            .plugins
            .get_mut(name)
            .ok_or_else(|| format!("Plugin '{name}' not found"))?;
        if entry.state != PluginState::Unconfigured {
            return Err(format!(
                "Plugin '{name}' cannot validate from {:?}",
                entry.state
            ));
        }
        entry.state = PluginState::Validated;
        Ok(())
    }

    pub fn activate(&mut self, name: &str, tool_count: usize) -> Result<(), String> {
        let entry = self
            .plugins
            .get_mut(name)
            .ok_or_else(|| format!("Plugin '{name}' not found"))?;
        if entry.state != PluginState::Validated {
            return Err(format!(
                "Plugin '{name}' cannot activate from {:?}",
                entry.state
            ));
        }
        entry.state = PluginState::Healthy;
        entry.tool_count = tool_count;
        Ok(())
    }

    pub fn degrade(&mut self, name: &str, reason: &str) -> Result<(), String> {
        let entry = self
            .plugins
            .get_mut(name)
            .ok_or_else(|| format!("Plugin '{name}' not found"))?;
        if entry.state != PluginState::Healthy {
            return Err(format!(
                "Plugin '{name}' cannot degrade from {:?}",
                entry.state
            ));
        }
        entry.state = PluginState::Degraded;
        entry.error = Some(reason.to_string());
        Ok(())
    }

    pub fn fail(&mut self, name: &str, reason: &str) {
        if let Some(entry) = self.plugins.get_mut(name) {
            entry.state = PluginState::Failed;
            entry.error = Some(reason.to_string());
        }
    }

    pub fn get(&self, name: &str) -> Option<&PluginEntry> {
        self.plugins.get(name)
    }

    pub fn list(&self) -> Vec<&PluginEntry> {
        self.plugins.values().collect()
    }

    pub fn active_plugins(&self) -> Vec<&PluginEntry> {
        self.plugins
            .values()
            .filter(|p| p.state.is_active())
            .collect()
    }

    pub fn remove(&mut self, name: &str) -> Option<PluginEntry> {
        self.plugins.remove(name)
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

static GLOBAL_PLUGIN_REGISTRY: OnceLock<Arc<Mutex<PluginRegistry>>> = OnceLock::new();

pub fn global_plugin_registry() -> &'static Arc<Mutex<PluginRegistry>> {
    GLOBAL_PLUGIN_REGISTRY.get_or_init(|| Arc::new(Mutex::new(PluginRegistry::new())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_lifecycle() {
        let mut reg = PluginRegistry::new();
        reg.register("my-plugin", "1.0.0");
        assert_eq!(
            reg.get("my-plugin").unwrap().state,
            PluginState::Unconfigured
        );

        reg.validate("my-plugin").unwrap();
        assert_eq!(reg.get("my-plugin").unwrap().state, PluginState::Validated);

        reg.activate("my-plugin", 3).unwrap();
        let p = reg.get("my-plugin").unwrap();
        assert_eq!(p.state, PluginState::Healthy);
        assert_eq!(p.tool_count, 3);

        reg.degrade("my-plugin", "timeout").unwrap();
        assert_eq!(reg.get("my-plugin").unwrap().state, PluginState::Degraded);

        reg.fail("my-plugin", "crash");
        assert_eq!(reg.get("my-plugin").unwrap().state, PluginState::Failed);
    }

    #[test]
    fn invalid_transitions_rejected() {
        let mut reg = PluginRegistry::new();
        reg.register("p", "1.0");
        assert!(reg.activate("p", 1).is_err());
        reg.validate("p").unwrap();
        assert!(reg.degrade("p", "x").is_err());
    }

    #[test]
    fn active_plugins_filter() {
        let mut reg = PluginRegistry::new();
        reg.register("a", "1.0");
        reg.register("b", "1.0");
        reg.register("c", "1.0");
        reg.validate("a").unwrap();
        reg.activate("a", 2).unwrap();
        reg.validate("b").unwrap();
        reg.activate("b", 1).unwrap();
        reg.degrade("b", "slow").unwrap();
        let active = reg.active_plugins();
        assert_eq!(active.len(), 2);
    }

    #[test]
    fn remove_plugin() {
        let mut reg = PluginRegistry::new();
        reg.register("temp", "0.1");
        assert!(reg.get("temp").is_some());
        reg.remove("temp");
        assert!(reg.get("temp").is_none());
    }
}
