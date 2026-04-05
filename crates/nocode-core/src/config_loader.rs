use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSource {
    User,
    Project,
    Local,
}

#[derive(Debug, Clone)]
pub struct ConfigEntry {
    pub source: ConfigSource,
    pub path: String,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub model: Option<String>,
    pub permission_mode: Option<String>,
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub mcp_servers: BTreeMap<String, McpServerConfig>,
    #[serde(default)]
    pub hooks: HooksConfig,
    #[serde(default)]
    pub sandbox: SandboxConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HooksConfig {
    #[serde(default)]
    pub pre_tool_use: Vec<String>,
    #[serde(default)]
    pub post_tool_use: Vec<String>,
    #[serde(default)]
    pub post_tool_use_failure: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SandboxConfig {
    pub enabled: Option<bool>,
    pub network_isolation: Option<bool>,
    pub filesystem_mode: Option<String>,
}

/// Read and parse a JSON config file from disk.
pub fn load_config_file(path: &str) -> Result<serde_json::Value, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read {path}: {e}"))?;
    serde_json::from_str(&content)
        .map_err(|e| format!("failed to parse {path}: {e}"))
}

/// Merge multiple config entries into a single RuntimeConfig.
/// Later entries override earlier ones. Scalar fields use last-wins,
/// maps are merged key-by-key, and vec fields are replaced wholesale.
pub fn merge_configs(entries: &[ConfigEntry]) -> RuntimeConfig {
    let mut rc = RuntimeConfig::default();
    for entry in entries {
        let obj = match entry.data.as_object() {
            Some(o) => o,
            None => continue,
        };
        if let Some(v) = obj.get("model").and_then(|v| v.as_str()) {
            rc.model = Some(v.to_string());
        }
        if let Some(v) = obj.get("permission_mode").and_then(|v| v.as_str()) {
            rc.permission_mode = Some(v.to_string());
        }
        if let Some(v) = obj.get("system_prompt").and_then(|v| v.as_str()) {
            rc.system_prompt = Some(v.to_string());
        }
        if let Some(servers) = obj.get("mcp_servers").and_then(|v| v.as_object()) {
            for (k, v) in servers {
                if let Ok(cfg) = serde_json::from_value::<McpServerConfig>(v.clone()) {
                    rc.mcp_servers.insert(k.clone(), cfg);
                }
            }
        }
        if let Some(hooks) = obj.get("hooks")
            && let Ok(h) = serde_json::from_value::<HooksConfig>(hooks.clone())
        {
            rc.hooks = h;
        }
        if let Some(sandbox) = obj.get("sandbox").and_then(|v| v.as_object()) {
            if let Some(v) = sandbox.get("enabled").and_then(|v| v.as_bool()) {
                rc.sandbox.enabled = Some(v);
            }
            if let Some(v) = sandbox.get("network_isolation").and_then(|v| v.as_bool()) {
                rc.sandbox.network_isolation = Some(v);
            }
            if let Some(v) = sandbox.get("filesystem_mode").and_then(|v| v.as_str()) {
                rc.sandbox.filesystem_mode = Some(v.to_string());
            }
        }
    }
    rc
}

/// Discover config files following the 3-tier hierarchy:
///
///   1. `~/.nocode/settings.json`        (User)
///   2. `{cwd}/.nocode/settings.json`    (Project)
///   3. `{cwd}/.nocode/settings.local.json` (Local)
///
/// Only files that exist and parse successfully are returned.
pub fn discover_config_files(cwd: &str) -> Vec<ConfigEntry> {
    let mut entries = Vec::new();

    let home = std::env::var("HOME").unwrap_or_default();
    let candidates: Vec<(ConfigSource, String)> = vec![
        (ConfigSource::User, format!("{home}/.nocode/settings.json")),
        (ConfigSource::Project, format!("{cwd}/.nocode/settings.json")),
        (ConfigSource::Local, format!("{cwd}/.nocode/settings.local.json")),
    ];

    for (source, path) in candidates {
        if let Ok(data) = load_config_file(&path) {
            entries.push(ConfigEntry { source, path, data });
        }
    }
    entries
}

/// Convenience: discover all config files for `cwd` and merge them.
pub fn load_runtime_config(cwd: &str) -> RuntimeConfig {
    let entries = discover_config_files(cwd);
    merge_configs(&entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_config_file_valid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cfg.json");
        std::fs::write(&path, r#"{"model":"opus"}"#).unwrap();
        let val = load_config_file(path.to_str().unwrap()).unwrap();
        assert_eq!(val["model"], "opus");
    }

    #[test]
    fn test_load_config_file_missing() {
        let res = load_config_file("/nonexistent/path.json");
        assert!(res.is_err());
    }

    #[test]
    fn test_load_config_file_invalid_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "not json").unwrap();
        let res = load_config_file(path.to_str().unwrap());
        assert!(res.is_err());
    }

    #[test]
    fn test_merge_configs_last_wins() {
        let entries = vec![
            ConfigEntry {
                source: ConfigSource::User,
                path: "a".into(),
                data: serde_json::json!({"model": "haiku"}),
            },
            ConfigEntry {
                source: ConfigSource::Project,
                path: "b".into(),
                data: serde_json::json!({"model": "opus"}),
            },
        ];
        let rc = merge_configs(&entries);
        assert_eq!(rc.model.as_deref(), Some("opus"));
    }

    #[test]
    fn test_merge_configs_mcp_servers_merged() {
        let entries = vec![
            ConfigEntry {
                source: ConfigSource::User,
                path: "a".into(),
                data: serde_json::json!({
                    "mcp_servers": {
                        "srv1": {"command": "cmd1", "args": [], "env": {}}
                    }
                }),
            },
            ConfigEntry {
                source: ConfigSource::Project,
                path: "b".into(),
                data: serde_json::json!({
                    "mcp_servers": {
                        "srv2": {"command": "cmd2", "args": [], "env": {}}
                    }
                }),
            },
        ];
        let rc = merge_configs(&entries);
        assert!(rc.mcp_servers.contains_key("srv1"));
        assert!(rc.mcp_servers.contains_key("srv2"));
    }

    #[test]
    fn test_merge_configs_sandbox_partial() {
        let entries = vec![
            ConfigEntry {
                source: ConfigSource::User,
                path: "a".into(),
                data: serde_json::json!({"sandbox": {"enabled": true}}),
            },
            ConfigEntry {
                source: ConfigSource::Local,
                path: "b".into(),
                data: serde_json::json!({"sandbox": {"network_isolation": false}}),
            },
        ];
        let rc = merge_configs(&entries);
        assert_eq!(rc.sandbox.enabled, Some(true));
        assert_eq!(rc.sandbox.network_isolation, Some(false));
    }

    #[test]
    fn test_discover_and_load_runtime() {
        let dir = tempfile::tempdir().unwrap();
        let nocode_dir = dir.path().join(".nocode");
        std::fs::create_dir_all(&nocode_dir).unwrap();
        std::fs::write(
            nocode_dir.join("settings.json"),
            r#"{"model":"sonnet","permission_mode":"auto"}"#,
        )
        .unwrap();
        std::fs::write(
            nocode_dir.join("settings.local.json"),
            r#"{"model":"opus"}"#,
        )
        .unwrap();

        let rc = load_runtime_config(dir.path().to_str().unwrap());
        assert_eq!(rc.model.as_deref(), Some("opus"));
        assert_eq!(rc.permission_mode.as_deref(), Some("auto"));
    }
}
