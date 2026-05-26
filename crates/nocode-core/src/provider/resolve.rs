use crate::config::settings::{ProviderDef, Settings, normalize_api_format};
use crate::provider::types::ModelProvider;
use std::env;

/// Resolved connection info for a single provider invocation. The product of
/// running the precedence chain (`--provider <name>` → `NOCODE_PROVIDER` →
/// active profile → `default_provider` → builtin alias) and looking up the
/// chosen entry in `[providers.<name>]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProvider {
    /// Logical name (e.g. "subfox", "anthropic", "local-vllm").
    pub name: String,
    /// HTTP base URL.
    pub base_url: String,
    /// Wire format. One of `anthropic`, `openai-responses`, `openai-chat`, `google`.
    pub wire_api: String,
    /// API key — already looked up via `api_key_env` or builtin fallback.
    pub api_key: String,
    /// The default model declared by this provider (if any).
    pub default_model: Option<String>,
}

impl ResolvedProvider {
    /// Coarse mapping from `wire_api` to the existing `ModelProvider` enum.
    pub fn legacy_model_provider(&self) -> ModelProvider {
        match self.wire_api.as_str() {
            "anthropic" => ModelProvider::Claude,
            "google" => ModelProvider::Gemini,
            _ => ModelProvider::OpenAi,
        }
    }
}

/// Builtin provider aliases — present even when the user has no
/// `[providers.*]` table. Lets `nocode --provider claude` work out of the box
/// from a single `ANTHROPIC_API_KEY`.
fn builtin_alias(name: &str) -> Option<ProviderDef> {
    match name {
        "anthropic" | "claude" => Some(ProviderDef {
            base_url: env::var("ANTHROPIC_BASE_URL")
                .unwrap_or_else(|_| "https://api.anthropic.com".to_owned()),
            wire_api: "anthropic".to_owned(),
            api_key_env: Some("ANTHROPIC_API_KEY".to_owned()),
            default_model: None,
        }),
        "openai" => Some(ProviderDef {
            base_url: env::var("OPENAI_BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com".to_owned()),
            wire_api: "openai-responses".to_owned(),
            api_key_env: Some("OPENAI_API_KEY".to_owned()),
            default_model: None,
        }),
        "gemini" | "google" => Some(ProviderDef {
            base_url: "https://generativelanguage.googleapis.com".to_owned(),
            wire_api: "google".to_owned(),
            api_key_env: Some("GEMINI_API_KEY".to_owned()),
            default_model: None,
        }),
        _ => None,
    }
}

/// The single, sanctioned provider resolver.
///
/// Precedence:
/// 1. `cli_provider` (`--provider <name>` from argv)
/// 2. `NOCODE_PROVIDER` env var
/// 3. The active profile's `provider` field (if `profile_name` is set)
/// 4. `settings.default_provider`
/// 5. `settings.model_provider` interpreted as a builtin alias
pub fn resolve_named_provider(
    settings: &Settings,
    cli_provider: Option<&str>,
    profile_name: Option<&str>,
) -> Result<ResolvedProvider, String> {
    let active_profile_provider = profile_name
        .and_then(|p| settings.profiles.get(p))
        .map(|p| p.provider.clone());

    let chosen_name = cli_provider
        .map(str::to_owned)
        .or_else(|| env::var("NOCODE_PROVIDER").ok())
        .or(active_profile_provider)
        .or_else(|| settings.default_provider.clone())
        .or_else(|| settings.model_provider.clone())
        .ok_or_else(|| {
            "No provider configured. Run `nocode init` to scaffold ~/.nocode/config.toml."
                .to_owned()
        })?;

    let def = settings
        .providers
        .get(&chosen_name)
        .cloned()
        .or_else(|| builtin_alias(&chosen_name))
        .ok_or_else(|| {
            format!(
                "Unknown provider '{chosen_name}'. \
                 Available: {available}. \
                 Define a [providers.{chosen_name}] table or use claude / openai / gemini.",
                available = available_names(settings).join(", "),
            )
        })?;

    let wire_api = normalize_api_format(&def.wire_api).to_owned();
    if !is_known_wire_api(&wire_api) {
        return Err(format!(
            "Provider '{chosen_name}' has wire_api='{}'. Valid: anthropic, openai-responses, openai-chat, google.",
            def.wire_api
        ));
    }

    let api_key = match &def.api_key_env {
        Some(env_name) => env::var(env_name).unwrap_or_default(),
        None => match wire_api.as_str() {
            "anthropic" => env::var("ANTHROPIC_API_KEY").unwrap_or_default(),
            "google" => env::var("GEMINI_API_KEY").unwrap_or_default(),
            _ => env::var("OPENAI_API_KEY").unwrap_or_default(),
        },
    };

    Ok(ResolvedProvider {
        name: chosen_name,
        base_url: def.base_url,
        wire_api,
        api_key,
        default_model: def.default_model,
    })
}

fn is_known_wire_api(s: &str) -> bool {
    matches!(s, "anthropic" | "openai-responses" | "openai-chat" | "google")
}

fn available_names(settings: &Settings) -> Vec<String> {
    let mut v: Vec<String> = settings.providers.keys().cloned().collect();
    for builtin in ["claude", "openai", "gemini"] {
        if !v.iter().any(|n| n == builtin) {
            v.push(builtin.to_owned());
        }
    }
    v
}

/// Unified API key resolution for all providers.
///
/// **Deprecated**: prefer [`resolve_named_provider`] which returns the full
/// `ResolvedProvider`. Kept while the four-variant `ModelProvider` enum still
/// exists in the call graph.
#[deprecated(note = "use resolve_named_provider() instead")]
pub fn resolve_api_key(provider: ModelProvider, _settings: &Settings) -> String {
    match provider {
        ModelProvider::Claude => env::var("ANTHROPIC_API_KEY").unwrap_or_default(),
        ModelProvider::OpenAi => env::var("OPENAI_API_KEY").unwrap_or_default(),
        ModelProvider::Gemini => env::var("GEMINI_API_KEY").unwrap_or_default(),
        ModelProvider::Custom => env::var("OPENAI_API_KEY").unwrap_or_default(),
    }
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
    use crate::config::settings::ProfileDef;

    fn settings_with_default(name: &str) -> Settings {
        Settings {
            default_provider: Some(name.to_owned()),
            ..Default::default()
        }
    }

    #[test]
    fn resolve_named_unknown_returns_err() {
        let s = settings_with_default("does_not_exist");
        let r = resolve_named_provider(&s, None, None);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("Unknown provider"));
    }

    #[test]
    fn resolve_named_with_no_config_returns_err() {
        // SAFETY: clearing inherited env so the test is hermetic.
        unsafe {
            env::remove_var("NOCODE_PROVIDER");
        }
        let s = Settings::default();
        assert!(resolve_named_provider(&s, None, None).is_err());
    }

    #[test]
    fn resolve_named_uses_builtin_alias() {
        unsafe {
            env::remove_var("NOCODE_PROVIDER");
        }
        let s = settings_with_default("claude");
        let r = resolve_named_provider(&s, None, None).expect("claude alias resolves");
        assert_eq!(r.name, "claude");
        assert_eq!(r.wire_api, "anthropic");
        assert!(!r.base_url.is_empty());
    }

    #[test]
    fn resolve_named_cli_arg_overrides_default() {
        unsafe {
            env::remove_var("NOCODE_PROVIDER");
        }
        let s = settings_with_default("claude");
        let r = resolve_named_provider(&s, Some("openai"), None).expect("ok");
        assert_eq!(r.name, "openai");
        assert_eq!(r.wire_api, "openai-responses");
    }

    #[test]
    fn resolve_named_rejects_invalid_wire_api() {
        let mut providers = std::collections::BTreeMap::new();
        providers.insert(
            "weird".to_owned(),
            ProviderDef {
                base_url: "http://x".to_owned(),
                wire_api: "some-bogus-format".to_owned(),
                api_key_env: None,
                default_model: None,
            },
        );
        let s = Settings {
            default_provider: Some("weird".to_owned()),
            providers,
            ..Default::default()
        };
        let r = resolve_named_provider(&s, None, None);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("wire_api"));
    }

    #[test]
    fn resolve_named_profile_overrides_default() {
        unsafe {
            env::remove_var("NOCODE_PROVIDER");
        }
        let mut profiles = std::collections::BTreeMap::new();
        profiles.insert(
            "work".to_owned(),
            ProfileDef {
                provider: "openai".to_owned(),
                model: None,
                permission_mode: None,
                reasoning_effort: None,
            },
        );
        let s = Settings {
            default_provider: Some("claude".to_owned()),
            profiles,
            ..Default::default()
        };
        let r = resolve_named_provider(&s, None, Some("work")).expect("ok");
        assert_eq!(r.name, "openai");
    }

    #[test]
    fn has_any_api_key_runs() {
        let _ = has_any_api_key();
    }
}
