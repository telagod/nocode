//! Feature flags — runtime toggles for experimental/optional features.
//!
//! Flags are stored in a JSON file (`~/.nocode/feature_flags.json`).
//! Environment variable `NOCODE_FF_<FLAG_NAME>` overrides file-based values.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

/// Known feature flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureFlag {
    /// Enable telemetry event logging.
    Telemetry,
    /// Enable auto-update version checks.
    AutoUpdate,
    /// Enable auto-dream memory consolidation.
    AutoDream,
    /// Enable MCP elicitation support.
    McpElicitation,
    /// Enable experimental TUI features.
    ExperimentalTui,
    /// Enable plugin system.
    Plugins,
    /// Enable team memory sync.
    TeamMemorySync,
    /// Enable structured output (JSON schema responses).
    StructuredOutput,
}

impl FeatureFlag {
    /// All known flags.
    pub const ALL: &[Self] = &[
        Self::Telemetry,
        Self::AutoUpdate,
        Self::AutoDream,
        Self::McpElicitation,
        Self::ExperimentalTui,
        Self::Plugins,
        Self::TeamMemorySync,
        Self::StructuredOutput,
    ];
    // APPEND_REST

    /// Flag name as used in JSON and env vars.
    pub fn name(self) -> &'static str {
        match self {
            Self::Telemetry => "telemetry",
            Self::AutoUpdate => "auto_update",
            Self::AutoDream => "auto_dream",
            Self::McpElicitation => "mcp_elicitation",
            Self::ExperimentalTui => "experimental_tui",
            Self::Plugins => "plugins",
            Self::TeamMemorySync => "team_memory_sync",
            Self::StructuredOutput => "structured_output",
        }
    }

    /// Parse from string name.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "telemetry" => Some(Self::Telemetry),
            "auto_update" => Some(Self::AutoUpdate),
            "auto_dream" => Some(Self::AutoDream),
            "mcp_elicitation" => Some(Self::McpElicitation),
            "experimental_tui" => Some(Self::ExperimentalTui),
            "plugins" => Some(Self::Plugins),
            "team_memory_sync" => Some(Self::TeamMemorySync),
            "structured_output" => Some(Self::StructuredOutput),
            _ => None,
        }
    }

    /// Default enabled state.
    pub fn default_enabled(self) -> bool {
        matches!(self, Self::AutoUpdate | Self::StructuredOutput)
    }

    /// Environment variable name for this flag.
    pub fn env_var(self) -> String {
        format!("NOCODE_FF_{}", self.name().to_uppercase())
    }
}
// APPEND_STORE

/// Feature flag store — file-backed with env var overrides.
pub struct FeatureFlagStore {
    path: PathBuf,
    flags: HashMap<FeatureFlag, bool>,
}

impl FeatureFlagStore {
    /// Create a new store, loading from file if it exists.
    pub fn new(path: &str) -> Self {
        let path = PathBuf::from(path);
        let flags = Self::load_from_file(&path);
        Self { path, flags }
    }

    /// Check if a flag is enabled (env var > file > default).
    pub fn is_enabled(&self, flag: FeatureFlag) -> bool {
        if let Ok(val) = std::env::var(flag.env_var()) {
            return matches!(val.as_str(), "1" | "true" | "yes");
        }
        if let Some(&val) = self.flags.get(&flag) {
            return val;
        }
        flag.default_enabled()
    }

    /// Set a flag value and persist.
    pub fn set(&mut self, flag: FeatureFlag, enabled: bool) -> Result<(), String> {
        self.flags.insert(flag, enabled);
        self.save()
    }

    /// Reset a flag to its default value.
    pub fn reset(&mut self, flag: FeatureFlag) -> Result<(), String> {
        self.flags.remove(&flag);
        self.save()
    }

    /// List all flags with their current effective values.
    pub fn list(&self) -> Vec<(FeatureFlag, bool, &'static str)> {
        FeatureFlag::ALL
            .iter()
            .map(|&f| {
                let enabled = self.is_enabled(f);
                let source = if std::env::var(f.env_var()).is_ok() {
                    "env"
                } else if self.flags.contains_key(&f) {
                    "file"
                } else {
                    "default"
                };
                (f, enabled, source)
            })
            .collect()
    }

    /// Reset all flags to defaults.
    pub fn reset_all(&mut self) -> Result<(), String> {
        self.flags.clear();
        self.save()
    }
    // APPEND_PRIVATE

    fn save(&self) -> Result<(), String> {
        let map: HashMap<String, bool> = self
            .flags
            .iter()
            .map(|(k, v)| (k.name().to_string(), *v))
            .collect();
        let json =
            serde_json::to_string_pretty(&map).map_err(|e| format!("serialize error: {e}"))?;
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        fs::write(&self.path, json).map_err(|e| format!("write error: {e}"))
    }

    fn load_from_file(path: &PathBuf) -> HashMap<FeatureFlag, bool> {
        let Ok(raw) = fs::read_to_string(path) else {
            return HashMap::new();
        };
        let Ok(map): Result<HashMap<String, bool>, _> = serde_json::from_str(&raw) else {
            return HashMap::new();
        };
        map.into_iter()
            .filter_map(|(k, v)| FeatureFlag::parse(&k).map(|f| (f, v)))
            .collect()
    }
}

/// Global singleton feature flag store.
static GLOBAL_FEATURE_FLAGS: OnceLock<Arc<Mutex<FeatureFlagStore>>> = OnceLock::new();

pub fn global_feature_flags() -> &'static Arc<Mutex<FeatureFlagStore>> {
    GLOBAL_FEATURE_FLAGS.get_or_init(|| {
        let home = std::env::var("HOME").unwrap_or_default();
        let path = format!("{home}/.nocode/feature_flags.json");
        Arc::new(Mutex::new(FeatureFlagStore::new(&path)))
    })
}

pub fn init_global_feature_flags(path: &str) {
    let global = global_feature_flags();
    let mut guard = global.lock().unwrap_or_else(|e| e.into_inner());
    *guard = FeatureFlagStore::new(path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_name_roundtrip() {
        for &flag in FeatureFlag::ALL {
            assert_eq!(FeatureFlag::parse(flag.name()), Some(flag));
        }
        assert_eq!(FeatureFlag::parse("bogus"), None);
    }

    #[test]
    fn default_values() {
        assert!(FeatureFlag::AutoUpdate.default_enabled());
        assert!(FeatureFlag::StructuredOutput.default_enabled());
        assert!(!FeatureFlag::Telemetry.default_enabled());
        assert!(!FeatureFlag::AutoDream.default_enabled());
    }

    #[test]
    fn store_defaults_without_file() {
        let tmp = format!("/tmp/nocode_ff_test_{}.json", std::process::id());
        let _ = fs::remove_file(&tmp);
        let store = FeatureFlagStore::new(&tmp);
        assert!(store.is_enabled(FeatureFlag::AutoUpdate));
        assert!(!store.is_enabled(FeatureFlag::Telemetry));
    }

    #[test]
    fn store_set_and_persist() {
        let tmp = format!("/tmp/nocode_ff_test2_{}.json", std::process::id());
        let _ = fs::remove_file(&tmp);
        {
            let mut store = FeatureFlagStore::new(&tmp);
            store.set(FeatureFlag::Telemetry, true).unwrap();
            assert!(store.is_enabled(FeatureFlag::Telemetry));
        }
        // Reload from file
        let store2 = FeatureFlagStore::new(&tmp);
        assert!(store2.is_enabled(FeatureFlag::Telemetry));
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn store_reset_flag() {
        let tmp = format!("/tmp/nocode_ff_test3_{}.json", std::process::id());
        let _ = fs::remove_file(&tmp);
        let mut store = FeatureFlagStore::new(&tmp);
        store.set(FeatureFlag::Telemetry, true).unwrap();
        assert!(store.is_enabled(FeatureFlag::Telemetry));
        store.reset(FeatureFlag::Telemetry).unwrap();
        assert!(!store.is_enabled(FeatureFlag::Telemetry)); // back to default
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn store_list_all() {
        let tmp = format!("/tmp/nocode_ff_test4_{}.json", std::process::id());
        let _ = fs::remove_file(&tmp);
        let store = FeatureFlagStore::new(&tmp);
        let list = store.list();
        assert_eq!(list.len(), FeatureFlag::ALL.len());
        // All should have source "default" since no file
        for (_, _, source) in &list {
            assert_eq!(*source, "default");
        }
    }

    #[test]
    fn store_reset_all() {
        let tmp = format!("/tmp/nocode_ff_test5_{}.json", std::process::id());
        let _ = fs::remove_file(&tmp);
        let mut store = FeatureFlagStore::new(&tmp);
        store.set(FeatureFlag::Telemetry, true).unwrap();
        store.set(FeatureFlag::Plugins, true).unwrap();
        store.reset_all().unwrap();
        assert!(!store.is_enabled(FeatureFlag::Telemetry));
        assert!(!store.is_enabled(FeatureFlag::Plugins));
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn env_var_name() {
        assert_eq!(FeatureFlag::Telemetry.env_var(), "NOCODE_FF_TELEMETRY");
        assert_eq!(FeatureFlag::AutoUpdate.env_var(), "NOCODE_FF_AUTO_UPDATE");
    }

    #[test]
    fn all_flags_count() {
        assert_eq!(FeatureFlag::ALL.len(), 8);
    }
}
