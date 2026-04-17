pub fn fetch_model_suggestions(
    provider: &str,
    custom_base_url: &str,
    custom_api_format: &str,
) -> Result<Vec<String>, String> {
    use nocode_core::provider::types::ModelProvider;

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .map_err(|e| format!("HTTP client build failed: {e}"))?;

    let format = if custom_api_format.is_empty() {
        "openai-responses"
    } else {
        nocode_core::config::settings::normalize_api_format(custom_api_format)
    };

    let parsed = ModelProvider::parse(provider);

    if parsed == Some(ModelProvider::Custom) || !custom_base_url.is_empty() {
        let normalized = format;
        if normalized == "openai-responses" || normalized == "openai-chat" {
            let key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
            return fetch_openai_models(&client, custom_base_url, &key);
        }
        if normalized == "anthropic" {
            let key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
            return fetch_anthropic_models(&client, custom_base_url, &key);
        }
        if normalized == "google" {
            let key = std::env::var("GEMINI_API_KEY").unwrap_or_default();
            return fetch_gemini_models(&client, &key);
        }
        return Err(format!("Unsupported custom API format: {normalized}"));
    }

    match parsed {
        Some(ModelProvider::OpenAi) => {
            let key = std::env::var("OPENAI_API_KEY").map_err(|e| e.to_string())?;
            fetch_openai_models(&client, "https://api.openai.com", &key)
        }
        Some(ModelProvider::Gemini) => {
            let key = std::env::var("GEMINI_API_KEY").map_err(|e| e.to_string())?;
            fetch_gemini_models(&client, &key)
        }
        Some(ModelProvider::Claude) => {
            let key = std::env::var("ANTHROPIC_API_KEY").map_err(|e| e.to_string())?;
            fetch_anthropic_models(&client, "https://api.anthropic.com", &key)
        }
        _ => {
            // Auto-detect from available keys
            if std::env::var("OPENAI_API_KEY").is_ok() {
                return fetch_model_suggestions("openai", custom_base_url, custom_api_format);
            }
            if std::env::var("GEMINI_API_KEY").is_ok() {
                return fetch_model_suggestions("gemini", custom_base_url, custom_api_format);
            }
            if std::env::var("ANTHROPIC_API_KEY").is_ok() {
                return fetch_model_suggestions("claude", custom_base_url, custom_api_format);
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
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| format!("HTTP client build failed: {e}"))?;

    let normalized = if api_format.is_empty() {
        "openai-chat"
    } else {
        nocode_core::config::settings::normalize_api_format(api_format)
    };

    match normalized {
        "openai-responses" | "openai-chat" => fetch_openai_models(&client, base_url, api_key),
        "anthropic" => fetch_anthropic_models(&client, base_url, api_key),
        "google" => fetch_gemini_models(&client, api_key),
        _ => Err(format!("Unsupported API format: {normalized}")),
    }
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

pub fn spawn_model_fetch_bg(
    provider: &str,
    custom_base_url: &str,
    custom_api_format: &str,
) -> std::sync::mpsc::Receiver<Result<Vec<String>, String>> {
    let provider = provider.to_string();
    let base_url = custom_base_url.to_string();
    let api_format = custom_api_format.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = fetch_model_suggestions(&provider, &base_url, &api_format);
        let _ = tx.send(result);
    });
    rx
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
