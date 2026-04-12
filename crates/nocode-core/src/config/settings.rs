use crate::config::runtime::{HookConfig, McpServerConfig, SandboxConfig};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Which tier a settings file belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTier {
    User,
    Project,
    Local,
}

impl SettingsTier {
    /// Directory name and file name for this tier.
    pub fn path_for(&self, cwd: &str) -> std::path::PathBuf {
        let home = std::env::var("HOME").unwrap_or_default();
        match self {
            Self::User => Path::new(&home).join(".nocode/settings.json"),
            Self::Project => Path::new(cwd).join(".nocode/settings.json"),
            Self::Local => Path::new(cwd).join(".nocode/settings.local.json"),
        }
    }
}

/// Runtime configuration merged from 3 tiers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub model_provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub max_turns: Option<u32>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub custom_base_url: Option<String>,
    #[serde(default)]
    pub custom_api_format: Option<String>,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    /// MCP servers — merged key-by-key across tiers.
    #[serde(default)]
    pub mcp_servers: HashMap<String, McpServerConfig>,
    /// Hooks — replaced wholesale by later tier.
    #[serde(default)]
    pub hooks: Option<HookConfig>,
    /// Sandbox — replaced wholesale by later tier.
    #[serde(default)]
    pub sandbox: Option<SandboxConfig>,
    /// Telemetry opt-in/out (default: not set → use feature flag).
    #[serde(default)]
    pub telemetry_enabled: Option<bool>,
}

impl Settings {
    /// Load settings from a JSON file, returning default if not found.
    pub fn load_from(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Save settings to a JSON file, creating parent directories as needed.
    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create dir: {e}"))?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize settings: {e}"))?;
        fs::write(path, json).map_err(|e| format!("Failed to write settings: {e}"))?;
        Ok(())
    }

    /// Save to a specific tier at the given cwd.
    pub fn save_tier(&self, tier: SettingsTier, cwd: &str) -> Result<(), String> {
        let path = tier.path_for(cwd);
        self.save_to(&path)
    }

    /// Merge another settings on top (later wins for non-None scalars,
    /// maps merged key-by-key, vecs replaced wholesale).
    pub fn merge(mut self, other: Self) -> Self {
        // Scalars: last wins
        self.model_provider = other.model_provider.or(self.model_provider);
        self.model = other.model.or(self.model);
        self.permission_mode = other.permission_mode.or(self.permission_mode);
        self.max_turns = other.max_turns.or(self.max_turns);
        self.max_tokens = other.max_tokens.or(self.max_tokens);
        self.custom_base_url = other.custom_base_url.or(self.custom_base_url);
        self.custom_api_format = other.custom_api_format.or(self.custom_api_format);
        self.system_prompt = other.system_prompt.or(self.system_prompt);
        self.reasoning_effort = other.reasoning_effort.or(self.reasoning_effort);

        // Maps: merged key-by-key
        for (k, v) in other.mcp_servers {
            self.mcp_servers.insert(k, v);
        }

        // Structs: replaced wholesale if present
        if other.hooks.is_some() {
            self.hooks = other.hooks;
        }
        if other.sandbox.is_some() {
            self.sandbox = other.sandbox;
        }

        // Scalars: last wins
        self.telemetry_enabled = other.telemetry_enabled.or(self.telemetry_enabled);

        self
    }

    /// Load 3-tier settings: user → project → local.
    pub fn load_merged(cwd: &str) -> Self {
        let home = std::env::var("HOME").unwrap_or_default();
        let user = Self::load_from(&Path::new(&home).join(".nocode/settings.json"));
        let project = Self::load_from(&Path::new(cwd).join(".nocode/settings.json"));
        let local = Self::load_from(&Path::new(cwd).join(".nocode/settings.local.json"));
        user.merge(project).merge(local)
    }

    /// Set a single key-value pair and persist to the given tier.
    /// Supports dotted key paths like "mcp_servers.github.command".
    pub fn set_and_persist(
        &mut self,
        key: &str,
        value: &str,
        tier: SettingsTier,
        cwd: &str,
    ) -> Result<(), String> {
        self.set_key(key, value)?;
        self.save_tier(tier, cwd)
    }

    /// Set a single key-value pair in memory (does not persist).
    /// Supports dotted key paths like "mcp_servers.github.command".
    pub fn set_key(&mut self, key: &str, value: &str) -> Result<(), String> {
        match key {
            "model_provider" => self.model_provider = Some(value.to_string()),
            "model" => self.model = Some(value.to_string()),
            "permission_mode" => self.permission_mode = Some(value.to_string()),
            "max_turns" => {
                self.max_turns = Some(value.parse().map_err(|_| "max_turns must be a number")?)
            }
            "max_tokens" => {
                self.max_tokens = Some(value.parse().map_err(|_| "max_tokens must be a number")?)
            }
            "custom_base_url" => self.custom_base_url = Some(value.to_string()),
            "custom_api_format" => self.custom_api_format = Some(value.to_string()),
            "system_prompt" => self.system_prompt = Some(value.to_string()),
            "reasoning_effort" => self.reasoning_effort = Some(value.to_string()),
            "telemetry_enabled" => {
                self.telemetry_enabled = Some(
                    value
                        .parse()
                        .map_err(|_| "telemetry_enabled must be true or false")?,
                )
            }
            _ => {
                return Err(format!(
                    "Unknown setting key: '{key}'. Supported: model_provider, model, permission_mode, max_turns, max_tokens, custom_base_url, custom_api_format, system_prompt, reasoning_effort, telemetry_enabled"
                ));
            }
        }
        Ok(())
    }

    /// Get a single key's current effective value.
    pub fn get_key(&self, key: &str) -> Option<String> {
        match key {
            "model_provider" => self.model_provider.clone(),
            "model" => self.model.clone(),
            "permission_mode" => self.permission_mode.clone(),
            "max_turns" => self.max_turns.map(|v| v.to_string()),
            "max_tokens" => self.max_tokens.map(|v| v.to_string()),
            "custom_base_url" => self.custom_base_url.clone(),
            "custom_api_format" => self.custom_api_format.clone(),
            "system_prompt" => self.system_prompt.clone(),
            "reasoning_effort" => self.reasoning_effort.clone(),
            "telemetry_enabled" => self.telemetry_enabled.map(|v| v.to_string()),
            _ => None,
        }
    }

    /// List all set keys and their values.
    pub fn list_set_keys(&self) -> Vec<(String, String)> {
        let mut result = Vec::new();
        if let Some(v) = &self.model_provider {
            result.push(("model_provider".into(), v.clone()));
        }
        if let Some(v) = &self.model {
            result.push(("model".into(), v.clone()));
        }
        if let Some(v) = &self.permission_mode {
            result.push(("permission_mode".into(), v.clone()));
        }
        if let Some(v) = self.max_turns {
            result.push(("max_turns".into(), v.to_string()));
        }
        if let Some(v) = self.max_tokens {
            result.push(("max_tokens".into(), v.to_string()));
        }
        if let Some(v) = &self.custom_base_url {
            result.push(("custom_base_url".into(), v.clone()));
        }
        if let Some(v) = &self.custom_api_format {
            result.push(("custom_api_format".into(), v.clone()));
        }
        if let Some(v) = &self.system_prompt {
            result.push(("system_prompt".into(), v.clone()));
        }
        if let Some(v) = &self.reasoning_effort {
            result.push(("reasoning_effort".into(), v.clone()));
        }
        if let Some(v) = self.telemetry_enabled {
            result.push(("telemetry_enabled".into(), v.to_string()));
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_scalars_last_wins() {
        let a = Settings {
            model: Some("gpt-4".to_string()),
            max_turns: Some(5),
            ..Default::default()
        };
        let b = Settings {
            model: Some("claude".to_string()),
            max_tokens: Some(8192),
            ..Default::default()
        };
        let merged = a.merge(b);
        assert_eq!(merged.model.as_deref(), Some("claude"));
        assert_eq!(merged.max_turns, Some(5));
        assert_eq!(merged.max_tokens, Some(8192));
    }

    #[test]
    fn merge_mcp_servers_key_by_key() {
        let mut a = Settings::default();
        a.mcp_servers.insert(
            "github".to_string(),
            McpServerConfig {
                command: "npx".to_string(),
                args: vec!["-y".to_string(), "mcp-github".to_string()],
                ..Default::default()
            },
        );
        let mut b = Settings::default();
        b.mcp_servers.insert(
            "slack".to_string(),
            McpServerConfig {
                command: "npx".to_string(),
                args: vec!["-y".to_string(), "mcp-slack".to_string()],
                ..Default::default()
            },
        );
        let merged = a.merge(b);
        assert_eq!(merged.mcp_servers.len(), 2);
        assert!(merged.mcp_servers.contains_key("github"));
        assert!(merged.mcp_servers.contains_key("slack"));
    }

    #[test]
    fn merge_hooks_replaced_wholesale() {
        let a = Settings {
            hooks: Some(HookConfig {
                pre_tool_use: vec![],
                post_tool_use: vec![],
                on_submit: vec![],
            }),
            ..Default::default()
        };
        let b = Settings::default(); // hooks = None
        let merged = a.merge(b);
        assert!(merged.hooks.is_some()); // a's hooks survive

        let c = Settings {
            hooks: Some(HookConfig::default()),
            ..Default::default()
        };
        let merged2 = merged.merge(c);
        assert!(merged2.hooks.is_some()); // c's hooks replace
    }

    #[test]
    fn merge_sandbox_replaced_wholesale() {
        let a = Settings {
            sandbox: Some(SandboxConfig {
                enabled: true,
                allowed_paths: vec!["/tmp".to_string()],
                network_enabled: false,
            }),
            ..Default::default()
        };
        let b = Settings::default();
        let merged = a.merge(b);
        assert!(merged.sandbox.as_ref().unwrap().enabled);
    }
}
