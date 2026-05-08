use nocode_core::provider::types::ModelProvider;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelFetchRequest {
    pub provider: String,
    pub base_url: String,
    pub api_format: String,
    pub api_key: String,
}

impl ModelFetchRequest {
    pub fn new(
        provider: impl Into<String>,
        base_url: impl Into<String>,
        api_format: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        Self {
            provider: provider.into(),
            base_url: base_url.into(),
            api_format: api_format.into(),
            api_key: api_key.into(),
        }
    }

    pub fn from_env(provider: &str, custom_base_url: &str, custom_api_format: &str) -> Self {
        let provider_string = provider.to_string();
        let base_url = custom_base_url.to_string();
        let normalized = if custom_api_format.is_empty() {
            "openai-responses".to_string()
        } else {
            nocode_core::config::settings::normalize_api_format(custom_api_format).to_string()
        };
        let api_key = resolve_api_key_for_request(provider, &base_url, &normalized, None);
        Self::new(provider_string, base_url, normalized, api_key)
    }

    fn normalized_api_format(&self) -> String {
        if self.api_format.is_empty() {
            "openai-responses".to_string()
        } else {
            nocode_core::config::settings::normalize_api_format(&self.api_format).to_string()
        }
    }
}

pub fn fetch_model_suggestions(
    provider: &str,
    custom_base_url: &str,
    custom_api_format: &str,
) -> Result<Vec<String>, String> {
    let request = ModelFetchRequest::from_env(provider, custom_base_url, custom_api_format);
    fetch_model_suggestions_for_request(&request)
}

pub fn fetch_model_suggestions_for_request(
    request: &ModelFetchRequest,
) -> Result<Vec<String>, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .map_err(|e| format!("HTTP client build failed: {e}"))?;

    let provider = ModelProvider::parse(&request.provider);
    let normalized = request.normalized_api_format();

    match provider {
        Some(ModelProvider::Custom) => {
            match normalized.as_str() {
                "openai-responses" | "openai-chat" => {
                    if request.base_url.trim().is_empty() {
                        return Err("Custom provider requires a base URL before fetching models"
                            .to_string());
                    }
                    fetch_openai_models(&client, &request.base_url, &request.api_key)
                }
                "anthropic" => {
                    if request.base_url.trim().is_empty() {
                        return Err("Custom provider requires a base URL before fetching models"
                            .to_string());
                    }
                    fetch_anthropic_models(&client, &request.base_url, &request.api_key)
                }
                "google" => fetch_gemini_models(&client, &request.api_key),
                _ => Err(format!("Unsupported custom API format: {normalized}")),
            }
        }
        Some(ModelProvider::OpenAi) => {
            fetch_openai_models(&client, "https://api.openai.com", &request.api_key)
        }
        Some(ModelProvider::Gemini) => fetch_gemini_models(&client, &request.api_key),
        Some(ModelProvider::Claude) => {
            fetch_anthropic_models(&client, "https://api.anthropic.com", &request.api_key)
        }
        None => {
            if std::env::var("OPENAI_API_KEY").is_ok() {
                return fetch_model_suggestions("openai", "", "");
            }
            if std::env::var("GEMINI_API_KEY").is_ok() {
                return fetch_model_suggestions("gemini", "", "");
            }
            if std::env::var("ANTHROPIC_API_KEY").is_ok() {
                return fetch_model_suggestions("claude", "", "");
            }
            Err("No provider credentials found to fetch models".to_string())
        }
    }
}

pub fn fetch_model_suggestions_with_key(
    base_url: &str,
    api_format: &str,
    api_key: &str,
) -> Result<Vec<String>, String> {
    let request = ModelFetchRequest::new("custom", base_url, api_format, api_key);
    fetch_model_suggestions_for_request(&request)
}

pub fn apply_model_filter(all_models: &[String], filter: &str) -> Vec<String> {
    let needle = filter.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return all_models.to_vec();
    }
    all_models
        .iter()
        .filter(|model| model.to_ascii_lowercase().contains(&needle))
        .cloned()
        .collect()
}

pub fn spawn_model_fetch_request_bg(
    request: ModelFetchRequest,
) -> std::sync::mpsc::Receiver<Result<Vec<String>, String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = fetch_model_suggestions_for_request(&request);
        let _ = tx.send(result);
    });
    rx
}

#[allow(dead_code)]
pub fn spawn_model_fetch_bg(
    provider: &str,
    custom_base_url: &str,
    custom_api_format: &str,
) -> std::sync::mpsc::Receiver<Result<Vec<String>, String>> {
    spawn_model_fetch_request_bg(ModelFetchRequest::from_env(
        provider,
        custom_base_url,
        custom_api_format,
    ))
}

fn resolve_api_key_for_request(
    provider: &str,
    base_url: &str,
    normalized_api_format: &str,
    explicit_api_key: Option<&str>,
) -> String {
    if let Some(api_key) = explicit_api_key
        && !api_key.trim().is_empty()
    {
        return api_key.trim().to_string();
    }

    let parsed = ModelProvider::parse(provider);
    if parsed == Some(ModelProvider::Custom) {
        return match normalized_api_format {
            "anthropic" => std::env::var("ANTHROPIC_API_KEY").unwrap_or_default(),
            "google" => std::env::var("GEMINI_API_KEY").unwrap_or_default(),
            _ => std::env::var("OPENAI_API_KEY").unwrap_or_default(),
        };
    }

    if !base_url.trim().is_empty() && parsed.is_none() {
        return match normalized_api_format {
            "anthropic" => std::env::var("ANTHROPIC_API_KEY").unwrap_or_default(),
            "google" => std::env::var("GEMINI_API_KEY").unwrap_or_default(),
            _ => std::env::var("OPENAI_API_KEY").unwrap_or_default(),
        };
    }

    match parsed {
        Some(ModelProvider::OpenAi) => std::env::var("OPENAI_API_KEY").unwrap_or_default(),
        Some(ModelProvider::Gemini) => std::env::var("GEMINI_API_KEY").unwrap_or_default(),
        Some(ModelProvider::Claude) => std::env::var("ANTHROPIC_API_KEY").unwrap_or_default(),
        Some(ModelProvider::Custom) => std::env::var("OPENAI_API_KEY").unwrap_or_default(),
        None => String::new(),
    }
}

fn fetch_openai_models(
    client: &reqwest::blocking::Client,
    base_url: &str,
    key: &str,
) -> Result<Vec<String>, String> {
    let body = client
        .get(format!("{}/v1/models", base_url.trim_end_matches('/')))
        .bearer_auth(key)
        .send()
        .map_err(|e| format!("Request failed: {e}"))?
        .text()
        .map_err(|e| format!("Read failed: {e}"))?;
    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("Invalid JSON: {e}"))?;
    let mut models: Vec<String> = json["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item["id"].as_str().map(ToString::to_string))
        .collect();
    models.sort();
    models.dedup();
    Ok(models)
}

fn fetch_anthropic_models(
    client: &reqwest::blocking::Client,
    base_url: &str,
    key: &str,
) -> Result<Vec<String>, String> {
    let body = client
        .get(format!("{}/v1/models", base_url.trim_end_matches('/')))
        .header("x-api-key", key)
        .header("anthropic-version", "2024-06-01")
        .send()
        .map_err(|e| format!("Request failed: {e}"))?
        .text()
        .map_err(|e| format!("Read failed: {e}"))?;
    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("Invalid JSON: {e}"))?;
    let mut models: Vec<String> = json["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item["id"].as_str().map(ToString::to_string))
        .collect();
    models.sort();
    models.dedup();
    Ok(models)
}

fn fetch_gemini_models(
    client: &reqwest::blocking::Client,
    key: &str,
) -> Result<Vec<String>, String> {
    let body = client
        .get(format!(
            "https://generativelanguage.googleapis.com/v1beta/models?key={key}"
        ))
        .send()
        .map_err(|e| format!("Request failed: {e}"))?
        .text()
        .map_err(|e| format!("Read failed: {e}"))?;
    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("Invalid JSON: {e}"))?;
    let mut models: Vec<String> = json["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            item["name"]
                .as_str()
                .map(|s| s.trim_start_matches("models/").to_string())
        })
        .collect();
    models.sort();
    models.dedup();
    Ok(models)
}
