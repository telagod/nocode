use crate::protocol_detect::{self, DetectResult};
use crate::provider_presets::{self, ALL_PRESETS, ProviderPreset};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigStep {
    SelectProvider {
        selected: usize,
        scroll: usize,
    },
    EnterUrl {
        input: String,
    },
    EnterKey {
        input: String,
    },
    Detecting,
    SelectFormat {
        selected: usize,
    },
    SelectModel {
        selected: usize,
        scroll: usize,
        filter: String,
        filtering: bool,
    },
    ManualModel {
        input: String,
    },
    Confirm,
    Done,
}

#[derive(Debug, Clone)]
pub struct ConfigFlowState {
    pub step: ConfigStep,
    pub preset: Option<&'static ProviderPreset>,
    pub base_url: String,
    pub api_key: String,
    pub api_format: String,
    pub model: String,
    pub models: Vec<String>,
    pub filtered_models: Vec<String>,
    pub detect_result: Option<DetectResult>,
    pub error: Option<String>,
    pub status: Option<String>,
}

impl Default for ConfigFlowState {
    fn default() -> Self {
        Self {
            step: ConfigStep::SelectProvider {
                selected: 0,
                scroll: 0,
            },
            preset: None,
            base_url: String::new(),
            api_key: String::new(),
            api_format: String::new(),
            model: String::new(),
            models: Vec::new(),
            filtered_models: Vec::new(),
            detect_result: None,
            error: None,
            status: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowAction {
    None,
    StartDetection,
    FetchModels,
    Save,
}

impl ConfigFlowState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn provider_list() -> &'static [ProviderPreset] {
        ALL_PRESETS
    }

    pub fn select_provider(&mut self, index: usize) -> FlowAction {
        let preset = &ALL_PRESETS[index];
        self.preset = Some(preset);
        self.base_url = preset.base_url.to_string();
        self.api_format = preset.api_format.to_string();

        if preset.requires_api_key {
            self.step = ConfigStep::EnterKey {
                input: String::new(),
            };
            FlowAction::None
        } else {
            self.api_key = String::new();
            self.step = ConfigStep::Detecting;
            self.status = Some("Detecting protocol...".into());
            FlowAction::StartDetection
        }
    }

    pub fn select_custom_url(&mut self) -> FlowAction {
        self.preset = None;
        self.step = ConfigStep::EnterUrl {
            input: String::new(),
        };
        FlowAction::None
    }

    pub fn submit_url(&mut self, url: String) -> FlowAction {
        self.base_url = url.trim().trim_end_matches('/').to_string();
        if let Some(p) = provider_presets::find_preset_by_url(&self.base_url) {
            self.preset = Some(p);
            self.api_format = p.api_format.to_string();
            if !p.requires_api_key {
                self.api_key = String::new();
                self.step = ConfigStep::Detecting;
                self.status = Some("Detecting protocol...".into());
                return FlowAction::StartDetection;
            }
        }
        self.step = ConfigStep::EnterKey {
            input: String::new(),
        };
        FlowAction::None
    }

    pub fn submit_key(&mut self, key: String) -> FlowAction {
        self.api_key = key.trim().to_string();
        self.step = ConfigStep::Detecting;
        self.status = Some("Detecting protocol...".into());
        FlowAction::StartDetection
    }

    pub fn on_detect_complete(&mut self, result: DetectResult) -> FlowAction {
        self.detect_result = Some(result.clone());
        if let Some(fmt) = result.api_format {
            self.api_format = fmt;
            self.models = result.models.clone();
            self.filtered_models = result.models;
            self.error = None;
            if self.models.is_empty() {
                self.status = Some("Connected, but no models returned".into());
            } else {
                self.status = Some(format!("Found {} models", self.models.len()));
            }
            self.step = ConfigStep::SelectModel {
                selected: 0,
                scroll: 0,
                filter: String::new(),
                filtering: false,
            };
            FlowAction::None
        } else {
            self.error = result.error.clone();
            self.status = Some("Auto-detect failed — select format manually".into());
            self.step = ConfigStep::SelectFormat { selected: 0 };
            FlowAction::None
        }
    }

    pub fn select_format(&mut self, index: usize) -> FlowAction {
        let formats = ["openai-chat", "openai-responses", "anthropic", "google"];
        self.api_format = formats[index].to_string();
        self.step = ConfigStep::Detecting;
        self.status = Some("Fetching models...".into());
        FlowAction::FetchModels
    }

    pub fn on_models_fetched(&mut self, result: Result<Vec<String>, String>) -> FlowAction {
        match result {
            Ok(models) => {
                self.models = models.clone();
                self.filtered_models = models;
                self.error = None;
                self.status = Some(format!("Found {} models", self.models.len()));
                self.step = ConfigStep::SelectModel {
                    selected: 0,
                    scroll: 0,
                    filter: String::new(),
                    filtering: false,
                };
            }
            Err(e) => {
                self.error = Some(e.clone());
                self.models = Vec::new();
                self.filtered_models = Vec::new();
                self.status = Some(format!("Fetch failed: {e}"));
                self.step = ConfigStep::SelectModel {
                    selected: 0,
                    scroll: 0,
                    filter: String::new(),
                    filtering: false,
                };
            }
        }
        FlowAction::None
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

    pub fn select_model(&mut self, model: String) -> FlowAction {
        self.model = model;
        self.step = ConfigStep::Confirm;
        FlowAction::None
    }

    pub fn submit_manual_model(&mut self, model: String) -> FlowAction {
        self.model = model.trim().to_string();
        if self.model.is_empty() {
            return FlowAction::None;
        }
        self.step = ConfigStep::Confirm;
        FlowAction::None
    }

    pub fn confirm(&mut self) -> FlowAction {
        self.step = ConfigStep::Done;
        FlowAction::Save
    }

    pub fn go_back(&mut self) -> FlowAction {
        self.step = match &self.step {
            ConfigStep::SelectProvider { .. } => return FlowAction::None,
            ConfigStep::EnterUrl { .. } => ConfigStep::SelectProvider {
                selected: 0,
                scroll: 0,
            },
            ConfigStep::EnterKey { .. } => {
                if self.preset.is_some() {
                    ConfigStep::SelectProvider {
                        selected: 0,
                        scroll: 0,
                    }
                } else {
                    ConfigStep::EnterUrl {
                        input: self.base_url.clone(),
                    }
                }
            }
            ConfigStep::Detecting => {
                if self.preset.is_some_and(|p| !p.requires_api_key) {
                    ConfigStep::SelectProvider {
                        selected: 0,
                        scroll: 0,
                    }
                } else {
                    ConfigStep::EnterKey {
                        input: self.api_key.clone(),
                    }
                }
            }
            ConfigStep::SelectFormat { .. } => ConfigStep::EnterKey {
                input: self.api_key.clone(),
            },
            ConfigStep::SelectModel { .. } => {
                if self.preset.is_some_and(|p| p.requires_api_key) {
                    ConfigStep::EnterKey {
                        input: self.api_key.clone(),
                    }
                } else if self.preset.is_some() {
                    ConfigStep::SelectProvider {
                        selected: 0,
                        scroll: 0,
                    }
                } else {
                    ConfigStep::EnterUrl {
                        input: self.base_url.clone(),
                    }
                }
            }
            ConfigStep::Confirm => ConfigStep::SelectModel {
                selected: 0,
                scroll: 0,
                filter: String::new(),
                filtering: false,
            },
            ConfigStep::ManualModel { .. } => ConfigStep::SelectModel {
                selected: 0,
                scroll: 0,
                filter: String::new(),
                filtering: false,
            },
            ConfigStep::Done => return FlowAction::None,
        };
        self.error = None;
        FlowAction::None
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
