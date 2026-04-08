//! Runtime configuration — full config with mcp_servers, hooks, sandbox, env overrides.

use crate::config::settings::Settings;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;

/// MCP server configuration entry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// Hook configuration — shell commands triggered by events.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HookConfig {
    #[serde(default)]
    pub pre_tool_use: Vec<HookEntry>,
    #[serde(default)]
    pub post_tool_use: Vec<HookEntry>,
    #[serde(default)]
    pub on_submit: Vec<HookEntry>,
}

/// A single hook entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookEntry {
    pub command: String,
    #[serde(default)]
    pub tool_filter: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// Sandbox configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SandboxConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub allowed_paths: Vec<String>,
    #[serde(default)]
    pub network_enabled: bool,
}

/// Full runtime configuration — merged from settings + env overrides.
#[derive(Debug, Clone, Default)]
pub struct RuntimeConfig {
    pub model: String,
    pub permission_mode: String,
    pub max_turns: u32,
    pub max_tokens: u32,
    pub system_prompt: Option<String>,
    pub custom_base_url: Option<String>,
    pub custom_api_format: Option<String>,
    pub reasoning_effort: Option<String>,
    pub mcp_servers: HashMap<String, McpServerConfig>,
    pub hooks: HookConfig,
    pub sandbox: SandboxConfig,
}

impl RuntimeConfig {
    /// Build RuntimeConfig from Settings + environment variable overrides.
    pub fn from_settings(settings: &Settings, _cwd: &str) -> Self {
        let mut config = Self {
            model: settings
                .model
                .clone()
                .unwrap_or_else(|| String::from("claude-sonnet-4-20250514")),
            permission_mode: settings
                .permission_mode
                .clone()
                .unwrap_or_else(|| String::from("ask")),
            max_turns: settings.max_turns.unwrap_or(10),
            max_tokens: settings.max_tokens.unwrap_or(16384),
            system_prompt: settings.system_prompt.clone(),
            custom_base_url: settings.custom_base_url.clone(),
            custom_api_format: settings.custom_api_format.clone(),
            reasoning_effort: settings.reasoning_effort.clone(),
            mcp_servers: settings.mcp_servers.clone(),
            hooks: settings.hooks.clone().unwrap_or_default(),
            sandbox: settings.sandbox.clone().unwrap_or_default(),
        };

        // Environment variable overrides
        if let Ok(m) = env::var("NOCODE_MODEL") {
            config.model = m;
        }
        if let Ok(u) = env::var("NOCODE_CUSTOM_BASE_URL") {
            config.custom_base_url = Some(u);
        }
        if let Ok(f) = env::var("NOCODE_CUSTOM_API_FORMAT") {
            config.custom_api_format = Some(f);
        }
        if let Ok(p) = env::var("NOCODE_SYSTEM_PROMPT") {
            config.system_prompt = Some(p);
        }
        if let Ok(r) = env::var("NOCODE_MODEL_REASONING_EFFORT") {
            config.reasoning_effort = Some(r);
        }

        config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let settings = Settings::default();
        let config = RuntimeConfig::from_settings(&settings, "/tmp");
        assert_eq!(config.model, "claude-sonnet-4-20250514");
        assert_eq!(config.permission_mode, "ask");
        assert_eq!(config.max_turns, 10);
        assert_eq!(config.max_tokens, 16384);
        assert!(config.mcp_servers.is_empty());
    }

    #[test]
    fn settings_override() {
        let settings = Settings {
            model: Some("gpt-4o".to_string()),
            max_turns: Some(20),
            max_tokens: Some(8192),
            ..Default::default()
        };
        let config = RuntimeConfig::from_settings(&settings, "/tmp");
        assert_eq!(config.model, "gpt-4o");
        assert_eq!(config.max_turns, 20);
        assert_eq!(config.max_tokens, 8192);
    }

    #[test]
    fn mcp_server_config_deserialize() {
        let json = serde_json::json!({
            "command": "npx",
            "args": ["-y", "@modelcontextprotocol/server-github"],
            "env": {"GITHUB_TOKEN": "xxx"}
        });
        let srv: McpServerConfig = serde_json::from_value(json).unwrap();
        assert_eq!(srv.command, "npx");
        assert_eq!(srv.args.len(), 2);
        assert_eq!(srv.env.get("GITHUB_TOKEN").unwrap(), "xxx");
    }

    #[test]
    fn hook_config_deserialize() {
        let json = serde_json::json!({
            "pre_tool_use": [{"command": "echo pre", "tool_filter": "Bash"}],
            "post_tool_use": [],
            "on_submit": []
        });
        let hooks: HookConfig = serde_json::from_value(json).unwrap();
        assert_eq!(hooks.pre_tool_use.len(), 1);
        assert_eq!(hooks.pre_tool_use[0].tool_filter.as_deref(), Some("Bash"));
    }

    #[test]
    fn settings_mcp_servers_flow_to_runtime() {
        let mut settings = Settings::default();
        settings.mcp_servers.insert(
            "github".to_string(),
            McpServerConfig {
                command: "npx".to_string(),
                args: vec!["-y".to_string(), "mcp-github".to_string()],
                ..Default::default()
            },
        );
        settings.mcp_servers.insert(
            "slack".to_string(),
            McpServerConfig {
                command: "npx".to_string(),
                args: vec!["-y".to_string(), "mcp-slack".to_string()],
                ..Default::default()
            },
        );
        let config = RuntimeConfig::from_settings(&settings, "/tmp");
        assert_eq!(config.mcp_servers.len(), 2);
        assert!(config.mcp_servers.contains_key("github"));
        assert!(config.mcp_servers.contains_key("slack"));
    }

    #[test]
    fn sandbox_config_deserialize() {
        let json = serde_json::json!({
            "enabled": true,
            "allowed_paths": ["/tmp", "/home/user/project"],
            "network_enabled": false
        });
        let sandbox: SandboxConfig = serde_json::from_value(json).unwrap();
        assert!(sandbox.enabled);
        assert_eq!(sandbox.allowed_paths.len(), 2);
        assert!(!sandbox.network_enabled);
    }
}
