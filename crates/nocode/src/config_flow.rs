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

#[derive(Debug, Clone)]
pub struct ConfigFormState {
    pub focus: ConfigField,

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
}

impl ConfigFormState {
    pub fn new() -> Self {
        let mut s = Self {
            focus: ConfigField::Provider,
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

        if let Some(url) = &settings.custom_base_url {
            self.base_url = url.clone();
            self.base_url_source = ValueSource::User;
        } else if let Some(p) = self.preset {
            self.base_url = p.base_url.to_string();
            self.base_url_source = ValueSource::Derived;
        }

        if let Some(fmt) = &settings.custom_api_format {
            self.api_format = fmt.clone();
            self.format_source = ValueSource::User;
        } else if let Some(p) = self.preset {
            self.api_format = p.api_format.to_string();
            self.format_source = ValueSource::Derived;
        }

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
        if let Some(p) = self.preset
            && !p.env_key_name.is_empty()
            && let Ok(val) = std::env::var(p.env_key_name)
            && !val.is_empty()
        {
            self.api_key = val;
            self.api_key_source = ValueSource::Env;
            return;
        }
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

fn mask_key(key: &str) -> String {
    if key.len() <= 8 {
        return format!("···{}", &key[key.len().saturating_sub(4)..]);
    }
    format!("···{}", &key[key.len() - 4..])
}
