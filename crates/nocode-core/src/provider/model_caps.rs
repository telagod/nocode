//! Model capability registry — dynamic (OpenRouter API) + static fallback.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCapabilities {
    pub context_window: u32,
    pub max_output_tokens: u32,
    pub supports_thinking: bool,
    pub default_thinking_budget: Option<u32>,
    pub supports_vision: bool,
    pub supports_tool_use: bool,
}

impl Copy for ModelCapabilities {}

const FALLBACK: ModelCapabilities = ModelCapabilities {
    context_window: 128_000,
    max_output_tokens: 16_384,
    supports_thinking: false,
    default_thinking_budget: None,
    supports_vision: false,
    supports_tool_use: true,
};

// --- Static fallback table (prefix match) ---

static MODEL_TABLE: &[(&str, ModelCapabilities)] = &[
    (
        "claude-opus-4",
        ModelCapabilities {
            context_window: 200_000,
            max_output_tokens: 32_768,
            supports_thinking: true,
            default_thinking_budget: Some(10_000),
            supports_vision: true,
            supports_tool_use: true,
        },
    ),
    (
        "claude-sonnet-4",
        ModelCapabilities {
            context_window: 200_000,
            max_output_tokens: 16_384,
            supports_thinking: true,
            default_thinking_budget: Some(8_000),
            supports_vision: true,
            supports_tool_use: true,
        },
    ),
    (
        "claude-haiku-3.5",
        ModelCapabilities {
            context_window: 200_000,
            max_output_tokens: 8_192,
            supports_thinking: false,
            default_thinking_budget: None,
            supports_vision: true,
            supports_tool_use: true,
        },
    ),
    (
        "claude-3",
        ModelCapabilities {
            context_window: 200_000,
            max_output_tokens: 8_192,
            supports_thinking: false,
            default_thinking_budget: None,
            supports_vision: true,
            supports_tool_use: true,
        },
    ),
    (
        "gpt-4.1-nano",
        ModelCapabilities {
            context_window: 1_000_000,
            max_output_tokens: 32_768,
            supports_thinking: false,
            default_thinking_budget: None,
            supports_vision: true,
            supports_tool_use: true,
        },
    ),
    (
        "gpt-4.1-mini",
        ModelCapabilities {
            context_window: 1_000_000,
            max_output_tokens: 32_768,
            supports_thinking: false,
            default_thinking_budget: None,
            supports_vision: true,
            supports_tool_use: true,
        },
    ),
    (
        "gpt-4.1",
        ModelCapabilities {
            context_window: 1_000_000,
            max_output_tokens: 32_768,
            supports_thinking: false,
            default_thinking_budget: None,
            supports_vision: true,
            supports_tool_use: true,
        },
    ),
    (
        "gpt-4o",
        ModelCapabilities {
            context_window: 128_000,
            max_output_tokens: 16_384,
            supports_thinking: false,
            default_thinking_budget: None,
            supports_vision: true,
            supports_tool_use: true,
        },
    ),
    (
        "o4-mini",
        ModelCapabilities {
            context_window: 200_000,
            max_output_tokens: 100_000,
            supports_thinking: true,
            default_thinking_budget: Some(10_000),
            supports_vision: true,
            supports_tool_use: true,
        },
    ),
    (
        "o3",
        ModelCapabilities {
            context_window: 200_000,
            max_output_tokens: 100_000,
            supports_thinking: true,
            default_thinking_budget: Some(10_000),
            supports_vision: true,
            supports_tool_use: true,
        },
    ),
    (
        "o1",
        ModelCapabilities {
            context_window: 200_000,
            max_output_tokens: 100_000,
            supports_thinking: true,
            default_thinking_budget: Some(10_000),
            supports_vision: true,
            supports_tool_use: true,
        },
    ),
    (
        "gemini-2.5-pro",
        ModelCapabilities {
            context_window: 1_000_000,
            max_output_tokens: 65_536,
            supports_thinking: true,
            default_thinking_budget: Some(8_000),
            supports_vision: true,
            supports_tool_use: true,
        },
    ),
    (
        "gemini-2.5-flash",
        ModelCapabilities {
            context_window: 1_000_000,
            max_output_tokens: 65_536,
            supports_thinking: true,
            default_thinking_budget: Some(8_000),
            supports_vision: true,
            supports_tool_use: true,
        },
    ),
    (
        "gemini-2.0",
        ModelCapabilities {
            context_window: 1_000_000,
            max_output_tokens: 8_192,
            supports_thinking: false,
            default_thinking_budget: None,
            supports_vision: true,
            supports_tool_use: true,
        },
    ),
    (
        "gemini-1.5",
        ModelCapabilities {
            context_window: 1_000_000,
            max_output_tokens: 8_192,
            supports_thinking: false,
            default_thinking_budget: None,
            supports_vision: true,
            supports_tool_use: true,
        },
    ),
    (
        "deepseek",
        ModelCapabilities {
            context_window: 64_000,
            max_output_tokens: 8_192,
            supports_thinking: true,
            default_thinking_budget: Some(8_000),
            supports_vision: false,
            supports_tool_use: true,
        },
    ),
    (
        "qwen",
        ModelCapabilities {
            context_window: 128_000,
            max_output_tokens: 8_192,
            supports_thinking: true,
            default_thinking_budget: Some(8_000),
            supports_vision: true,
            supports_tool_use: true,
        },
    ),
    (
        "llama",
        ModelCapabilities {
            context_window: 128_000,
            max_output_tokens: 8_192,
            supports_thinking: false,
            default_thinking_budget: None,
            supports_vision: false,
            supports_tool_use: true,
        },
    ),
    (
        "mistral",
        ModelCapabilities {
            context_window: 128_000,
            max_output_tokens: 8_192,
            supports_thinking: false,
            default_thinking_budget: None,
            supports_vision: false,
            supports_tool_use: true,
        },
    ),
];

fn static_lookup(model_name: &str) -> Option<ModelCapabilities> {
    let lower = model_name.to_ascii_lowercase();
    let mut best: Option<&ModelCapabilities> = None;
    let mut best_len = 0;
    for &(prefix, ref caps) in MODEL_TABLE {
        if lower.starts_with(prefix) && prefix.len() > best_len {
            best = Some(caps);
            best_len = prefix.len();
        }
    }
    best.copied()
}

// --- Dynamic cache (OpenRouter API) ---

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedRegistry {
    fetched_at: i64,
    models: HashMap<String, ModelCapabilities>,
}

static GLOBAL_CACHE: OnceLock<Arc<Mutex<HashMap<String, ModelCapabilities>>>> = OnceLock::new();

fn global_cache() -> &'static Arc<Mutex<HashMap<String, ModelCapabilities>>> {
    GLOBAL_CACHE.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

fn cache_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    std::path::PathBuf::from(home)
        .join(".nocode")
        .join("model_caps_cache.json")
}

const CACHE_TTL_SECS: i64 = 86400; // 24h

/// Load cache from disk if fresh enough.
fn load_disk_cache() -> Option<HashMap<String, ModelCapabilities>> {
    let path = cache_path();
    let data = std::fs::read_to_string(&path).ok()?;
    let cached: CachedRegistry = serde_json::from_str(&data).ok()?;
    let now = chrono::Utc::now().timestamp();
    if now - cached.fetched_at > CACHE_TTL_SECS {
        return None;
    }
    Some(cached.models)
}

/// Save cache to disk.
fn save_disk_cache(models: &HashMap<String, ModelCapabilities>) {
    let cached = CachedRegistry {
        fetched_at: chrono::Utc::now().timestamp(),
        models: models.clone(),
    };
    if let Ok(json) = serde_json::to_string(&cached) {
        let path = cache_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, json);
    }
}

/// Parse OpenRouter API response into our capabilities map.
fn parse_openrouter(body: &str) -> Option<HashMap<String, ModelCapabilities>> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let data = v.get("data")?.as_array()?;
    let mut map = HashMap::new();
    for entry in data {
        let id = entry.get("id")?.as_str()?;
        // Strip provider prefix: "anthropic/claude-sonnet-4" → "claude-sonnet-4"
        let model_name = id.rsplit('/').next().unwrap_or(id);
        let ctx = entry
            .get("context_length")
            .and_then(|v| v.as_u64())
            .unwrap_or(128_000) as u32;
        let max_out = entry
            .pointer("/top_provider/max_completion_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(16_384) as u32;
        let modalities = entry
            .pointer("/architecture/input_modalities")
            .and_then(|v| v.as_array());
        let vision = modalities
            .map(|arr| arr.iter().any(|m| m.as_str() == Some("image")))
            .unwrap_or(false);
        let params = entry.get("supported_parameters").and_then(|v| v.as_array());
        let thinking = params
            .as_ref()
            .map(|arr| {
                arr.iter().any(|p| {
                    let s = p.as_str().unwrap_or("");
                    s == "reasoning" || s == "include_reasoning"
                })
            })
            .unwrap_or(false);
        let tools = params
            .as_ref()
            .map(|arr| arr.iter().any(|p| p.as_str() == Some("tools")))
            .unwrap_or(true);
        let budget = if thinking { Some(8_000) } else { None };
        map.insert(
            model_name.to_string(),
            ModelCapabilities {
                context_window: ctx,
                max_output_tokens: max_out,
                supports_thinking: thinking,
                default_thinking_budget: budget,
                supports_vision: vision,
                supports_tool_use: tools,
            },
        );
    }
    Some(map)
}

/// Fetch model capabilities from OpenRouter API (blocking).
fn fetch_openrouter() -> Option<HashMap<String, ModelCapabilities>> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .ok()?;
    let resp = client
        .get("https://openrouter.ai/api/v1/models")
        .send()
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body = resp.text().ok()?;
    parse_openrouter(&body)
}

/// Initialize the global cache: load from disk or fetch from API.
/// Call this once at startup (can be async via thread).
pub fn init_cache() {
    if let Some(models) = load_disk_cache() {
        let cache = global_cache();
        if let Ok(mut c) = cache.lock() {
            *c = models;
        }
        return;
    }
    if let Some(models) = fetch_openrouter() {
        save_disk_cache(&models);
        let cache = global_cache();
        if let Ok(mut c) = cache.lock() {
            *c = models;
        }
    }
}

/// Spawn cache init on a background thread (non-blocking).
pub fn init_cache_async() {
    std::thread::spawn(init_cache);
}

/// Lookup model capabilities: dynamic cache → static table → fallback.
pub fn lookup(model_name: &str) -> ModelCapabilities {
    // 1. Check dynamic cache (exact match)
    let cache = global_cache();
    if let Ok(c) = cache.lock() {
        if let Some(caps) = c.get(model_name) {
            return *caps;
        }
        // Try without provider prefix
        let short = model_name.rsplit('/').next().unwrap_or(model_name);
        if short != model_name
            && let Some(caps) = c.get(short)
        {
            return *caps;
        }
    }
    // 2. Static prefix table
    if let Some(caps) = static_lookup(model_name) {
        return caps;
    }
    // 3. Fallback
    FALLBACK
}

/// Get the fallback capabilities (for unknown models).
pub fn fallback() -> ModelCapabilities {
    FALLBACK
}

/// Check if the dynamic cache has been populated.
pub fn cache_loaded() -> bool {
    let cache = global_cache();
    cache.lock().map(|c| !c.is_empty()).unwrap_or(false)
}

/// Number of models in the dynamic cache.
pub fn cache_count() -> usize {
    let cache = global_cache();
    cache.lock().map(|c| c.len()).unwrap_or(0)
}
