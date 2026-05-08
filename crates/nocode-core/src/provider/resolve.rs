use crate::config::settings::{Settings, normalize_api_format};
use crate::provider::types::ModelProvider;
use std::env;

/// Unified API key resolution for all providers.
///
/// For Claude/OpenAI/Gemini: returns the corresponding env var.
/// For Custom: resolves via preset env key (if `custom_preset` matches a known
/// preset name), otherwise maps by `custom_api_format` to the appropriate
/// provider key. `NOCODE_CUSTOM_API_KEY` is intentionally NOT consulted.
pub fn resolve_api_key(provider: ModelProvider, settings: &Settings) -> String {
    match provider {
        ModelProvider::Claude => env::var("ANTHROPIC_API_KEY").unwrap_or_default(),
        ModelProvider::OpenAi => env::var("OPENAI_API_KEY").unwrap_or_default(),
        ModelProvider::Gemini => env::var("GEMINI_API_KEY").unwrap_or_default(),
        ModelProvider::Custom => resolve_custom_api_key(settings),
    }
}

/// Resolve the base URL for Custom provider.
/// Returns `Ok(url)` or `Err(message)` when no URL is configured.
pub fn resolve_custom_base_url(settings: &Settings) -> Result<String, String> {
    settings
        .custom_base_url
        .clone()
        .or_else(|| env::var("NOCODE_CUSTOM_BASE_URL").ok())
        .ok_or_else(|| {
            "Custom provider requires a base URL. \
             Set NOCODE_CUSTOM_BASE_URL or custom_base_url in settings."
                .to_string()
        })
}

/// Resolve and normalize the API format for Custom provider.
pub fn resolve_custom_api_format(settings: &Settings) -> String {
    let raw = settings
        .custom_api_format
        .clone()
        .or_else(|| env::var("NOCODE_CUSTOM_API_FORMAT").ok())
        .unwrap_or_else(|| String::from("openai-responses"));
    normalize_api_format(&raw).to_string()
}

fn resolve_custom_api_key(settings: &Settings) -> String {
    // 1. If a preset is configured, use its dedicated env var
    if let Some(preset_name) = settings.custom_preset.as_deref()
        && let Some(env_key) = lookup_preset_env_key(preset_name)
    {
        if env_key.is_empty() {
            return String::new(); // preset needs no key (e.g. Ollama)
        }
        if let Ok(val) = env::var(env_key) {
            return val;
        }
    }
    // 2. Fall back to api_format-based mapping
    let format = resolve_custom_api_format(settings);
    match format.as_str() {
        "anthropic" => env::var("ANTHROPIC_API_KEY").unwrap_or_default(),
        "google" => env::var("GEMINI_API_KEY").unwrap_or_default(),
        _ => env::var("OPENAI_API_KEY").unwrap_or_default(),
    }
}

/// Known preset env key names. This is the single source of truth for
/// preset → env var mapping, shared by all call sites.
const PRESET_ENV_KEYS: &[(&str, &str)] = &[
    ("Anthropic", "ANTHROPIC_API_KEY"),
    ("OpenAI", "OPENAI_API_KEY"),
    ("Gemini", "GEMINI_API_KEY"),
    ("OpenRouter", "OPENROUTER_API_KEY"),
    ("Together", "TOGETHER_API_KEY"),
    ("Groq", "GROQ_API_KEY"),
    ("Fireworks", "FIREWORKS_API_KEY"),
    ("DeepSeek", "DEEPSEEK_API_KEY"),
    ("Mistral", "MISTRAL_API_KEY"),
    ("Ollama", ""),
    ("vLLM", "VLLM_API_KEY"),
    ("LiteLLM", "LITELLM_API_KEY"),
    ("LocalAI", ""),
    ("LM Studio", ""),
];

fn lookup_preset_env_key(preset_name: &str) -> Option<&'static str> {
    PRESET_ENV_KEYS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(preset_name))
        .map(|(_, key)| *key)
}

/// Check if any usable API key is available (for onboarding gate).
/// Only checks actual key env vars, not provider selection vars.
pub fn has_any_api_key() -> bool {
    env::var("ANTHROPIC_API_KEY").is_ok()
        || env::var("OPENAI_API_KEY").is_ok()
        || env::var("GEMINI_API_KEY").is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_key_claude() {
        let settings = Settings::default();
        // Without env var set, returns empty
        let key = resolve_api_key(ModelProvider::Claude, &settings);
        // Just verify it doesn't panic; actual value depends on env
        let _ = key;
    }

    #[test]
    fn resolve_custom_format_default() {
        let settings = Settings::default();
        assert_eq!(resolve_custom_api_format(&settings), "openai-responses");
    }

    #[test]
    fn resolve_custom_format_legacy_normalization() {
        let settings = Settings {
            custom_api_format: Some("openai".to_string()),
            ..Default::default()
        };
        assert_eq!(resolve_custom_api_format(&settings), "openai-responses");
    }

    #[test]
    fn resolve_custom_base_url_missing_is_err() {
        let settings = Settings::default();
        assert!(resolve_custom_base_url(&settings).is_err());
    }

    #[test]
    fn resolve_custom_base_url_from_settings() {
        let settings = Settings {
            custom_base_url: Some("https://example.com".to_string()),
            ..Default::default()
        };
        assert_eq!(
            resolve_custom_base_url(&settings).unwrap(),
            "https://example.com"
        );
    }

    #[test]
    fn lookup_preset_known() {
        assert_eq!(
            lookup_preset_env_key("openrouter"),
            Some("OPENROUTER_API_KEY")
        );
        assert_eq!(lookup_preset_env_key("Ollama"), Some(""));
    }

    #[test]
    fn lookup_preset_unknown() {
        assert_eq!(lookup_preset_env_key("nonexistent"), None);
    }

    #[test]
    fn has_any_api_key_without_env() {
        // This test just verifies the function runs; actual result depends on env
        let _ = has_any_api_key();
    }
}
