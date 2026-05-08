#[derive(Debug, Clone, Copy)]
pub struct ProviderPreset {
    pub name: &'static str,
    pub base_url: &'static str,
    pub api_format: &'static str,
    pub auth_hint: &'static str,
    pub env_key_name: &'static str,
    pub credential_slot: &'static str,
    pub default_model: &'static str,
    pub provider_type: &'static str,
}

pub static ALL_PRESETS: &[ProviderPreset] = &[
    // --- Big 3 providers ---
    ProviderPreset {
        name: "Anthropic",
        base_url: "https://api.anthropic.com",
        api_format: "anthropic",
        auth_hint: "Get key at console.anthropic.com/settings/keys",
        env_key_name: "ANTHROPIC_API_KEY",
        credential_slot: "anthropic",
        default_model: "claude-sonnet-4-20250514",
        provider_type: "anthropic",
    },
    ProviderPreset {
        name: "OpenAI",
        base_url: "https://api.openai.com",
        api_format: "openai-responses",
        auth_hint: "Get key at platform.openai.com/api-keys",
        env_key_name: "OPENAI_API_KEY",
        credential_slot: "openai",
        default_model: "gpt-4.1",
        provider_type: "openai",
    },
    ProviderPreset {
        name: "Gemini",
        base_url: "https://generativelanguage.googleapis.com",
        api_format: "google",
        auth_hint: "Get key at aistudio.google.com/apikey",
        env_key_name: "GEMINI_API_KEY",
        credential_slot: "gemini",
        default_model: "gemini-2.5-pro",
        provider_type: "gemini",
    },
    // --- Cloud API proxies ---
    ProviderPreset {
        name: "OpenRouter",
        base_url: "https://openrouter.ai/api/v1",
        api_format: "openai-chat",
        auth_hint: "Get key at openrouter.ai/keys",
        env_key_name: "OPENROUTER_API_KEY",
        credential_slot: "openrouter",
        default_model: "anthropic/claude-sonnet-4",
        provider_type: "custom",
    },
    ProviderPreset {
        name: "Together",
        base_url: "https://api.together.xyz/v1",
        api_format: "openai-chat",
        auth_hint: "Get key at api.together.xyz/settings/api-keys",
        env_key_name: "TOGETHER_API_KEY",
        credential_slot: "together",
        default_model: "meta-llama/Llama-3-70b-chat-hf",
        provider_type: "custom",
    },
    ProviderPreset {
        name: "Groq",
        base_url: "https://api.groq.com/openai/v1",
        api_format: "openai-chat",
        auth_hint: "Get key at console.groq.com/keys",
        env_key_name: "GROQ_API_KEY",
        credential_slot: "groq",
        default_model: "llama-3.3-70b-versatile",
        provider_type: "custom",
    },
    ProviderPreset {
        name: "Fireworks",
        base_url: "https://api.fireworks.ai/inference/v1",
        api_format: "openai-chat",
        auth_hint: "Get key at fireworks.ai/account/api-keys",
        env_key_name: "FIREWORKS_API_KEY",
        credential_slot: "fireworks",
        default_model: "accounts/fireworks/models/llama-v3p1-70b-instruct",
        provider_type: "custom",
    },
    ProviderPreset {
        name: "DeepSeek",
        base_url: "https://api.deepseek.com/v1",
        api_format: "openai-chat",
        auth_hint: "Get key at platform.deepseek.com/api_keys",
        env_key_name: "DEEPSEEK_API_KEY",
        credential_slot: "deepseek",
        default_model: "deepseek-chat",
        provider_type: "custom",
    },
    ProviderPreset {
        name: "Mistral",
        base_url: "https://api.mistral.ai/v1",
        api_format: "openai-chat",
        auth_hint: "Get key at console.mistral.ai/api-keys",
        env_key_name: "MISTRAL_API_KEY",
        credential_slot: "mistral",
        default_model: "mistral-large-latest",
        provider_type: "custom",
    },
    // --- Local inference ---
    ProviderPreset {
        name: "Ollama",
        base_url: "http://localhost:11434/v1",
        api_format: "openai-chat",
        auth_hint: "No API key needed for local Ollama",
        env_key_name: "",
        credential_slot: "ollama",
        default_model: "",
        provider_type: "custom",
    },
    ProviderPreset {
        name: "vLLM",
        base_url: "http://localhost:8000/v1",
        api_format: "openai-chat",
        auth_hint: "Use --api-key flag if set on server",
        env_key_name: "VLLM_API_KEY",
        credential_slot: "vllm",
        default_model: "",
        provider_type: "custom",
    },
    ProviderPreset {
        name: "LiteLLM",
        base_url: "http://localhost:4000/v1",
        api_format: "openai-chat",
        auth_hint: "Set LITELLM_API_KEY or use proxy key",
        env_key_name: "LITELLM_API_KEY",
        credential_slot: "litellm",
        default_model: "",
        provider_type: "custom",
    },
    ProviderPreset {
        name: "LocalAI",
        base_url: "http://localhost:8080/v1",
        api_format: "openai-chat",
        auth_hint: "Optional, depends on config",
        env_key_name: "",
        credential_slot: "localai",
        default_model: "",
        provider_type: "custom",
    },
    ProviderPreset {
        name: "LM Studio",
        base_url: "http://localhost:1234/v1",
        api_format: "openai-chat",
        auth_hint: "No key required for local LM Studio",
        env_key_name: "",
        credential_slot: "lmstudio",
        default_model: "",
        provider_type: "custom",
    },
];

/// Find a preset by name (case-insensitive).
pub fn find_preset_by_name(name: &str) -> Option<&'static ProviderPreset> {
    ALL_PRESETS
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(name))
}

/// Find a preset whose `base_url` is a prefix of the given URL.
pub fn find_preset_by_url(url: &str) -> Option<&'static ProviderPreset> {
    ALL_PRESETS.iter().find(|p| url.starts_with(p.base_url))
}

/// Look up the environment variable key for a preset by name (case-insensitive).
/// Returns `Some("")` for presets that need no key, `None` if the name is unknown.
#[allow(dead_code)]
pub fn preset_env_key(name: &str) -> Option<&'static str> {
    find_preset_by_name(name).map(|p| p.env_key_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_preset_by_name_case_insensitive() {
        let preset = find_preset_by_name("anthropic").unwrap();
        assert_eq!(preset.name, "Anthropic");
        assert_eq!(preset.provider_type, "anthropic");

        let preset = find_preset_by_name("OPENAI").unwrap();
        assert_eq!(preset.name, "OpenAI");

        let preset = find_preset_by_name("gemini").unwrap();
        assert_eq!(preset.name, "Gemini");

        let preset = find_preset_by_name("openrouter").unwrap();
        assert_eq!(preset.name, "OpenRouter");
        assert_eq!(preset.provider_type, "custom");

        assert!(find_preset_by_name("nonexistent").is_none());
    }

    #[test]
    fn find_preset_by_url_matches_prefix() {
        let preset = find_preset_by_url("https://api.anthropic.com/v1/messages").unwrap();
        assert_eq!(preset.name, "Anthropic");

        let preset = find_preset_by_url("https://api.openai.com/v1/responses").unwrap();
        assert_eq!(preset.name, "OpenAI");

        let preset =
            find_preset_by_url("https://generativelanguage.googleapis.com/v1beta/models").unwrap();
        assert_eq!(preset.name, "Gemini");

        let preset = find_preset_by_url("https://openrouter.ai/api/v1/chat/completions").unwrap();
        assert_eq!(preset.name, "OpenRouter");

        let preset = find_preset_by_url("http://localhost:11434/v1/chat/completions").unwrap();
        assert_eq!(preset.name, "Ollama");

        assert!(find_preset_by_url("https://unknown.example.com").is_none());
    }

    #[test]
    fn preset_env_key_lookup() {
        assert_eq!(preset_env_key("Anthropic"), Some("ANTHROPIC_API_KEY"));
        assert_eq!(preset_env_key("openai"), Some("OPENAI_API_KEY"));
        assert_eq!(preset_env_key("GEMINI"), Some("GEMINI_API_KEY"));
        assert_eq!(preset_env_key("openrouter"), Some("OPENROUTER_API_KEY"));
        assert_eq!(preset_env_key("Ollama"), Some(""));
        assert_eq!(preset_env_key("LocalAI"), Some(""));
        assert_eq!(preset_env_key("LM Studio"), Some(""));
        assert_eq!(preset_env_key("nonexistent"), None);
    }

    #[test]
    fn all_presets_count() {
        // 3 big providers + 11 custom = 14 total
        assert_eq!(ALL_PRESETS.len(), 14);
    }

    #[test]
    fn all_presets_have_valid_provider_type() {
        for preset in ALL_PRESETS {
            assert!(
                matches!(
                    preset.provider_type,
                    "anthropic" | "openai" | "gemini" | "custom"
                ),
                "Invalid provider_type '{}' for preset '{}'",
                preset.provider_type,
                preset.name
            );
        }
    }

    #[test]
    fn no_duplicate_names() {
        let names: Vec<&str> = ALL_PRESETS.iter().map(|p| p.name).collect();
        for (i, name) in names.iter().enumerate() {
            assert!(
                !names[i + 1..].contains(name),
                "Duplicate preset name: {name}"
            );
        }
    }

    #[test]
    fn no_duplicate_base_urls() {
        let urls: Vec<&str> = ALL_PRESETS.iter().map(|p| p.base_url).collect();
        for (i, url) in urls.iter().enumerate() {
            assert!(!urls[i + 1..].contains(url), "Duplicate base_url: {url}");
        }
    }
}
