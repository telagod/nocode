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

// ---------------------------------------------------------------------------
// Plugin manifest and execution runtime
// ---------------------------------------------------------------------------

/// Plugin manifest loaded from `manifest.json`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    /// Command to execute the plugin (e.g. "node index.js", "python plugin.py").
    #[serde(default)]
    pub command: String,
    /// Arguments passed to the command.
    #[serde(default)]
    pub args: Vec<String>,
    /// Tool names this plugin provides.
    #[serde(default)]
    pub tools: Vec<String>,
}

/// Plugin execution runtime — discovers, loads, and runs plugins.
pub struct PluginRuntime {
    /// Base directory for plugin discovery.
    pub plugins_dir: String,
}

impl PluginRuntime {
    pub fn new(plugins_dir: &str) -> Self {
        Self {
            plugins_dir: plugins_dir.to_string(),
        }
    }

    /// Default plugins directory: `.nocode/plugins/` in the given project root.
    pub fn default_dir(project_root: &str) -> String {
        format!("{project_root}/.nocode/plugins")
    }

    /// Discover all plugin manifests in the plugins directory.
    pub fn discover(&self) -> Vec<(String, PluginManifest)> {
        let dir = std::path::Path::new(&self.plugins_dir);
        if !dir.exists() {
            return Vec::new();
        }
        let mut manifests = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let manifest_path = path.join("manifest.json");
                if !manifest_path.exists() {
                    continue;
                }
                if let Ok(raw) = std::fs::read_to_string(&manifest_path)
                    && let Ok(manifest) = serde_json::from_str::<PluginManifest>(&raw)
                {
                    let plugin_dir = path.to_string_lossy().to_string();
                    manifests.push((plugin_dir, manifest));
                }
            }
        }
        manifests
    }

    /// Load and register all discovered plugins.
    pub fn load_all(&self, registry: &mut PluginRegistry) -> Vec<String> {
        let manifests = self.discover();
        let mut loaded = Vec::new();
        for (dir, manifest) in &manifests {
            registry.register(&manifest.name, &manifest.version);
            if let Err(e) = registry.validate(&manifest.name) {
                registry.fail(&manifest.name, &e);
                continue;
            }
            let tool_count = manifest.tools.len();
            if let Err(e) = registry.activate(&manifest.name, tool_count) {
                registry.fail(&manifest.name, &e);
                continue;
            }
            loaded.push(format!("{} v{} ({})", manifest.name, manifest.version, dir));
        }
        loaded
    }

    /// Execute a plugin by name. Returns stdout output or error.
    pub fn execute(&self, name: &str, input: &str) -> Result<String, String> {
        let manifests = self.discover();
        let (dir, manifest) = manifests
            .iter()
            .find(|(_, m)| m.name == name)
            .ok_or_else(|| format!("Plugin '{name}' not found"))?;

        if manifest.command.is_empty() {
            return Err(format!("Plugin '{name}' has no command defined"));
        }

        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(&manifest.command)
            .args(&manifest.args)
            .current_dir(dir)
            .env("PLUGIN_INPUT", input)
            .output()
            .map_err(|e| format!("Failed to execute plugin '{name}': {e}"))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if output.status.success() {
            Ok(stdout)
        } else {
            Err(format!(
                "Plugin '{name}' failed (exit {}): {stderr}",
                output.status.code().unwrap_or(-1)
            ))
        }
    }
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

    // --- PluginManifest ---
    #[test]
    fn manifest_deserialize() {
        let json = r#"{"name":"test-plugin","version":"1.0","description":"A test","command":"echo hi","tools":["tool1"]}"#;
        let m: PluginManifest = serde_json::from_str(json).unwrap();
        assert_eq!(m.name, "test-plugin");
        assert_eq!(m.version, "1.0");
        assert_eq!(m.tools, vec!["tool1"]);
    }

    #[test]
    fn manifest_defaults() {
        let json = r#"{"name":"minimal","version":"0.1"}"#;
        let m: PluginManifest = serde_json::from_str(json).unwrap();
        assert!(m.command.is_empty());
        assert!(m.tools.is_empty());
        assert!(m.description.is_empty());
    }

    // --- PluginRuntime ---
    #[test]
    fn discover_empty_dir() {
        let tmp = std::env::temp_dir().join(format!("nocode_plug1_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let rt = PluginRuntime::new(tmp.to_str().unwrap());
        assert!(rt.discover().is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn discover_finds_manifest() {
        let tmp = std::env::temp_dir().join(format!("nocode_plug2_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let plugin_dir = tmp.join("my-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("manifest.json"),
            r#"{"name":"my-plugin","version":"1.0","command":"echo hello"}"#,
        )
        .unwrap();
        let rt = PluginRuntime::new(tmp.to_str().unwrap());
        let found = rt.discover();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].1.name, "my-plugin");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn load_all_registers_plugins() {
        let tmp = std::env::temp_dir().join(format!("nocode_plug3_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let plugin_dir = tmp.join("test-plug");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("manifest.json"),
            r#"{"name":"test-plug","version":"2.0","tools":["t1","t2"]}"#,
        )
        .unwrap();
        let rt = PluginRuntime::new(tmp.to_str().unwrap());
        let mut reg = PluginRegistry::new();
        let loaded = rt.load_all(&mut reg);
        assert_eq!(loaded.len(), 1);
        let entry = reg.get("test-plug").unwrap();
        assert_eq!(entry.state, PluginState::Healthy);
        assert_eq!(entry.tool_count, 2);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn execute_plugin() {
        let tmp = std::env::temp_dir().join(format!("nocode_plug4_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let plugin_dir = tmp.join("echo-plug");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("manifest.json"),
            r#"{"name":"echo-plug","version":"1.0","command":"echo plugin_output"}"#,
        )
        .unwrap();
        let rt = PluginRuntime::new(tmp.to_str().unwrap());
        let result = rt.execute("echo-plug", "test").unwrap();
        assert!(result.contains("plugin_output"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn execute_nonexistent_plugin() {
        let tmp = std::env::temp_dir().join(format!("nocode_plug5_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let rt = PluginRuntime::new(tmp.to_str().unwrap());
        assert!(rt.execute("ghost", "").is_err());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
