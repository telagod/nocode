use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// Runtime configuration merged from 3 tiers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
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
}

impl Settings {
    /// Load settings from a JSON file, returning default if not found.
    pub fn load_from(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Merge another settings on top (later wins for non-None fields).
    pub fn merge(self, other: Self) -> Self {
        Self {
            model: other.model.or(self.model),
            permission_mode: other.permission_mode.or(self.permission_mode),
            max_turns: other.max_turns.or(self.max_turns),
            max_tokens: other.max_tokens.or(self.max_tokens),
            custom_base_url: other.custom_base_url.or(self.custom_base_url),
            custom_api_format: other.custom_api_format.or(self.custom_api_format),
        }
    }

    /// Load 3-tier settings: user → project → local.
    pub fn load_merged(cwd: &str) -> Self {
        let home = std::env::var("HOME").unwrap_or_default();
        let user = Self::load_from(&Path::new(&home).join(".nocode/settings.json"));
        let project = Self::load_from(&Path::new(cwd).join(".nocode/settings.json"));
        let local = Self::load_from(&Path::new(cwd).join(".nocode/settings.local.json"));
        user.merge(project).merge(local)
    }
}
