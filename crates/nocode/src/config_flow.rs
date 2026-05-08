use crate::protocol_detect::{self, DetectResult};
use crate::provider_presets::{self, ALL_PRESETS, ProviderPreset};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigField {
    Provider,
    ApiKey,
    BaseUrl,
    Format,
    Model,
}

impl ConfigField {
    pub const ALL: [Self; 5] = [
        Self::Provider,
        Self::ApiKey,
        Self::BaseUrl,
        Self::Format,
        Self::Model,
    ];

    pub fn index(self) -> usize {
        match self {
            Self::Provider => 0,
            Self::ApiKey => 1,
            Self::BaseUrl => 2,
            Self::Format => 3,
            Self::Model => 4,
        }
    }

    pub fn from_index(i: usize) -> Self {
        match i {
            0 => Self::Provider,
            1 => Self::ApiKey,
            2 => Self::BaseUrl,
            3 => Self::Format,
            _ => Self::Model,
        }
    }

    pub fn next(self) -> Self {
        Self::from_index((self.index() + 1).min(Self::ALL.len() - 1))
    }

    pub fn prev(self) -> Self {
        Self::from_index(self.index().saturating_sub(1))
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Provider => "Provider",
            Self::ApiKey => "API Key",
            Self::BaseUrl => "Base URL",
            Self::Format => "Format",
            Self::Model => "Model",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueSource {
    Env,
    User,
    Default,
    Derived,
}

impl ValueSource {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Env => "env",
            Self::User => "user",
            Self::Default => "default",
            Self::Derived => "derived",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditMode {
    Normal,
    EditingText(ConfigField),
    BrowsingModels {
        selected: usize,
        scroll: usize,
    },
    FilteringModels {
        selected: usize,
        scroll: usize,
        filter: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowAction {
    None,
    StartDetection,
    FetchModels,
    Save,
}

#[derive(Debug, Clone)]
pub struct ConfigFormState {
    pub focus: ConfigField,
    pub mode: EditMode,

    pub preset_index: usize,
    pub preset: Option<&'static ProviderPreset>,
    pub provider_source: ValueSource,

    pub api_key: String,
    pub api_key_source: ValueSource,

    pub base_url: String,
    pub base_url_source: ValueSource,

    pub api_format: String,
    pub format_source: ValueSource,

    pub model: String,
    pub model_source: ValueSource,

    pub models: Vec<String>,
    pub filtered_models: Vec<String>,

    pub edit_buffer: String,
    pub error: Option<String>,
    pub status: Option<String>,

    pub done: bool,
}

impl ConfigFormState {
    pub fn new() -> Self {
        let mut s = Self {
            focus: ConfigField::Provider,
            mode: EditMode::Normal,
            preset_index: 0,
            preset: None,
            provider_source: ValueSource::Default,
            api_key: String::new(),
            api_key_source: ValueSource::Default,
            base_url: String::new(),
            base_url_source: ValueSource::Default,
            api_format: String::new(),
            format_source: ValueSource::Default,
            model: String::new(),
            model_source: ValueSource::Default,
            models: Vec::new(),
            filtered_models: Vec::new(),
            edit_buffer: String::new(),
            error: None,
            status: None,
            done: false,
        };
        s.load_current();
        s
    }

    fn load_current(&mut self) {
        use nocode_core::config::settings::Settings;

        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let settings = Settings::load_merged(&cwd);

        // Resolve preset from custom_preset or model_provider
        if let Some(preset_name) = &settings.custom_preset
            && let Some(p) = provider_presets::find_preset_by_name(preset_name)
        {
            self.preset = Some(p);
            self.provider_source = ValueSource::User;
        }
        if self.preset.is_none()
            && let Some(mp) = &settings.model_provider
        {
            let found = ALL_PRESETS.iter().find(|p| p.provider_type == mp.as_str());
            if let Some(p) = found {
                self.preset = Some(p);
                self.provider_source = ValueSource::User;
            }
        }

        // Sync preset_index
        if let Some(p) = self.preset {
            self.preset_index = ALL_PRESETS
                .iter()
                .position(|x| x.name == p.name)
                .unwrap_or(0);
        }

        // Base URL
        if let Some(url) = &settings.custom_base_url {
            self.base_url = url.clone();
            self.base_url_source = ValueSource::User;
        } else if let Some(p) = self.preset {
            self.base_url = p.base_url.to_string();
            self.base_url_source = ValueSource::Derived;
        }

        // API format
        if let Some(fmt) = &settings.custom_api_format {
            self.api_format = fmt.clone();
            self.format_source = ValueSource::User;
        } else if let Some(p) = self.preset {
            self.api_format = p.api_format.to_string();
            self.format_source = ValueSource::Derived;
        }

        // Model
        if let Some(m) = &settings.model {
            self.model = m.clone();
            self.model_source = ValueSource::User;
        } else if let Some(p) = self.preset {
            self.model = p.default_model.to_string();
            self.model_source = ValueSource::Default;
        }

        self.load_api_key();
    }

    fn load_api_key(&mut self) {
        // Check env var first
        if let Some(p) = self.preset
            && !p.env_key_name.is_empty()
            && let Ok(val) = std::env::var(p.env_key_name)
            && !val.is_empty()
        {
            self.api_key = val;
            self.api_key_source = ValueSource::Env;
            return;
        }
        // Check credential store
        let slot = self.preset.map_or("custom", |p| p.credential_slot);
        let cred_path = nocode_core::storage::credentials::CredentialStore::default_path();
        if let Ok(store) = nocode_core::storage::credentials::CredentialStore::load(&cred_path)
            && let Some(key) = store.get_key(slot)
        {
            self.api_key = key;
            self.api_key_source = ValueSource::User;
            return;
        }
        self.api_key.clear();
        self.api_key_source = ValueSource::Default;
    }

    pub fn provider_list() -> &'static [ProviderPreset] {
        ALL_PRESETS
    }

    pub fn cycle_provider_forward(&mut self) {
        self.preset_index = (self.preset_index + 1) % ALL_PRESETS.len();
        self.apply_preset(self.preset_index);
    }

    pub fn cycle_provider_backward(&mut self) {
        if self.preset_index == 0 {
            self.preset_index = ALL_PRESETS.len() - 1;
        } else {
            self.preset_index -= 1;
        }
        self.apply_preset(self.preset_index);
    }

    fn apply_preset(&mut self, index: usize) {
        let preset = &ALL_PRESETS[index];
        self.preset = Some(preset);
        self.provider_source = ValueSource::User;
        self.base_url = preset.base_url.to_string();
        self.base_url_source = ValueSource::Derived;
        self.api_format = preset.api_format.to_string();
        self.format_source = ValueSource::Derived;
        if !preset.default_model.is_empty() {
            self.model = preset.default_model.to_string();
            self.model_source = ValueSource::Default;
        }
        self.models.clear();
        self.filtered_models.clear();
        self.status = None;
        self.error = None;
        self.load_api_key();
    }

    pub fn submit_base_url(&mut self) {
        let url = self.edit_buffer.trim().trim_end_matches('/').to_string();
        if !url.is_empty() {
            self.base_url = url;
            self.base_url_source = ValueSource::User;
            if let Some(p) = provider_presets::find_preset_by_url(&self.base_url) {
                self.preset = Some(p);
                self.provider_source = ValueSource::Derived;
                self.preset_index = ALL_PRESETS
                    .iter()
                    .position(|x| x.name == p.name)
                    .unwrap_or(0);
                self.api_format = p.api_format.to_string();
                self.format_source = ValueSource::Derived;
            }
        }
        self.edit_buffer.clear();
        self.mode = EditMode::Normal;
    }

    pub fn submit_api_key(&mut self) {
        self.api_key = self.edit_buffer.trim().to_string();
        self.api_key_source = ValueSource::User;
        self.edit_buffer.clear();
        self.mode = EditMode::Normal;
    }

    pub fn submit_model_manual(&mut self) {
        let m = self.edit_buffer.trim().to_string();
        if !m.is_empty() {
            self.model = m;
            self.model_source = ValueSource::User;
        }
        self.edit_buffer.clear();
        self.mode = EditMode::Normal;
    }

    pub fn select_model_from_list(&mut self, model: String) {
        self.model = model;
        self.model_source = ValueSource::User;
        self.mode = EditMode::Normal;
    }

    pub fn cycle_format_forward(&mut self) {
        let formats = nocode_core::config::settings::API_FORMATS;
        let cur = formats.iter().position(|&f| f == self.api_format);
        let next = match cur {
            Some(i) => (i + 1) % formats.len(),
            None => 0,
        };
        self.api_format = formats[next].to_string();
        self.format_source = ValueSource::User;
    }

    pub fn cycle_format_backward(&mut self) {
        let formats = nocode_core::config::settings::API_FORMATS;
        let cur = formats.iter().position(|&f| f == self.api_format);
        let next = match cur {
            Some(0) | None => formats.len() - 1,
            Some(i) => i - 1,
        };
        self.api_format = formats[next].to_string();
        self.format_source = ValueSource::User;
    }

    pub fn apply_filter(&mut self, filter: &str) {
        let needle = filter.trim().to_ascii_lowercase();
        self.filtered_models = if needle.is_empty() {
            self.models.clone()
        } else {
            self.models
                .iter()
                .filter(|m| m.to_ascii_lowercase().contains(&needle))
                .cloned()
                .collect()
        };
    }

    pub fn start_detection(&self) -> FlowAction {
        if self.base_url.is_empty() {
            return FlowAction::None;
        }
        FlowAction::StartDetection
    }

    pub fn start_fetch_models(&self) -> FlowAction {
        if self.base_url.is_empty() || self.api_format.is_empty() {
            return FlowAction::None;
        }
        FlowAction::FetchModels
    }

    pub fn on_detect_complete(&mut self, result: DetectResult) {
        if let Some(fmt) = result.api_format {
            self.api_format = fmt;
            self.format_source = ValueSource::Derived;
            self.models = result.models.clone();
            self.filtered_models = result.models;
            if self.models.is_empty() {
                self.status = Some("Connected — no models returned".into());
            } else {
                self.status = Some(format!("Found {} models", self.models.len()));
            }
            self.error = None;
        } else {
            self.error = result.error;
            self.status = Some("Auto-detect failed".into());
        }
    }

    pub fn on_models_fetched(&mut self, result: Result<Vec<String>, String>) {
        match result {
            Ok(models) => {
                self.models = models.clone();
                self.filtered_models = models;
                self.error = None;
                self.status = Some(format!("Found {} models", self.models.len()));
            }
            Err(e) => {
                self.error = Some(e.clone());
                self.models = Vec::new();
                self.filtered_models = Vec::new();
                self.status = Some(format!("Fetch failed: {e}"));
            }
        }
    }

    pub fn provider_type_str(&self) -> &str {
        self.preset.map_or("custom", |p| p.provider_type)
    }

    pub fn credential_slot(&self) -> &str {
        self.preset.map_or("custom", |p| p.credential_slot)
    }

    pub fn needs_custom_settings(&self) -> bool {
        self.preset.is_none_or(|p| p.provider_type == "custom")
    }

    pub fn display_provider(&self) -> &str {
        self.preset.map_or("(not set)", |p| p.name)
    }

    pub fn display_api_key(&self) -> String {
        if self.api_key.is_empty() {
            "(not set)".to_string()
        } else {
            mask_key(&self.api_key)
        }
    }

    pub fn display_base_url(&self) -> &str {
        if self.base_url.is_empty() {
            "(not set)"
        } else {
            &self.base_url
        }
    }

    pub fn display_format(&self) -> &str {
        if self.api_format.is_empty() {
            "(not set)"
        } else {
            &self.api_format
        }
    }

    pub fn display_model(&self) -> &str {
        if self.model.is_empty() {
            "(not set)"
        } else {
            &self.model
        }
    }
}

pub fn spawn_detect(base_url: String, api_key: String) -> std::sync::mpsc::Receiver<DetectResult> {
    protocol_detect::spawn_detect_bg(base_url, api_key)
}

pub fn spawn_fetch_models(
    base_url: String,
    api_key: String,
    api_format: String,
) -> std::sync::mpsc::Receiver<Result<Vec<String>, String>> {
    protocol_detect::spawn_fetch_models_bg(base_url, api_key, api_format)
}

fn mask_key(key: &str) -> String {
    if key.len() <= 8 {
        return format!("···{}", &key[key.len().saturating_sub(4)..]);
    }
    format!("···{}", &key[key.len() - 4..])
}
