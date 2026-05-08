/// Auto-detect API protocol and fetch available models from a base URL + API key.
use std::sync::mpsc;

#[derive(Debug, Clone)]
pub struct DetectResult {
    pub api_format: Option<String>,
    pub models: Vec<String>,
    pub error: Option<String>,
}

pub fn detect_provider(base_url: &str, api_key: &str) -> DetectResult {
    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return DetectResult {
                api_format: None,
                models: vec![],
                error: Some(format!("HTTP client build failed: {e}")),
            };
        }
    };

    let url = base_url.trim_end_matches('/');

    // Known URL shortcut
    if let Some(format) = known_url_format(url) {
        let models = fetch_models_with_client(&client, url, api_key, &format).unwrap_or_default();
        return DetectResult {
            api_format: Some(format),
            models,
            error: None,
        };
    }

    // OpenAI-compatible probe: try /models then /v1/models
    if let Some(models) = probe_openai(&client, url, api_key) {
        return DetectResult {
            api_format: Some("openai-chat".to_string()),
            models,
            error: None,
        };
    }

    // Anthropic probe
    if let Some(models) = probe_anthropic(&client, url, api_key) {
        return DetectResult {
            api_format: Some("anthropic".to_string()),
            models,
            error: None,
        };
    }

    DetectResult {
        api_format: None,
        models: vec![],
        error: Some(format!(
            "All protocol probes failed for {url}. Could not detect API format."
        )),
    }
}

pub fn fetch_models(
    base_url: &str,
    api_key: &str,
    api_format: &str,
) -> Result<Vec<String>, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .map_err(|e| format!("HTTP client build failed: {e}"))?;
    let url = base_url.trim_end_matches('/');
    fetch_models_with_client(&client, url, api_key, api_format)
}

pub fn spawn_detect_bg(base_url: String, api_key: String) -> mpsc::Receiver<DetectResult> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = detect_provider(&base_url, &api_key);
        let _ = tx.send(result);
    });
    rx
}

pub fn spawn_fetch_models_bg(
    base_url: String,
    api_key: String,
    api_format: String,
) -> mpsc::Receiver<Result<Vec<String>, String>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = fetch_models(&base_url, &api_key, &api_format);
        let _ = tx.send(result);
    });
    rx
}

fn known_url_format(url: &str) -> Option<String> {
    let lower = url.to_ascii_lowercase();
    if lower.contains("api.anthropic.com") {
        return Some("anthropic".to_string());
    }
    if lower.contains("api.openai.com") {
        return Some("openai-responses".to_string());
    }
    if lower.contains("generativelanguage.googleapis.com") {
        return Some("google".to_string());
    }
    let openai_chat_domains = [
        "openrouter.ai",
        "api.together.xyz",
        "api.groq.com",
        "api.fireworks.ai",
        "api.deepseek.com",
        "api.mistral.ai",
    ];
    for domain in &openai_chat_domains {
        if lower.contains(domain) {
            return Some("openai-chat".to_string());
        }
    }
    None
}

fn fetch_models_with_client(
    client: &reqwest::blocking::Client,
    base_url: &str,
    api_key: &str,
    api_format: &str,
) -> Result<Vec<String>, String> {
    match api_format {
        "openai-responses" | "openai-chat" => fetch_openai_models(client, base_url, api_key),
        "anthropic" => fetch_anthropic_models(client, base_url, api_key),
        "google" => fetch_gemini_models(client, api_key),
        _ => Err(format!("Unsupported API format: {api_format}")),
    }
}

fn probe_openai(
    client: &reqwest::blocking::Client,
    base_url: &str,
    api_key: &str,
) -> Option<Vec<String>> {
    // Try /models first
    if let Ok(models) = try_openai_endpoint(client, &format!("{base_url}/models"), api_key) {
        return Some(models);
    }
    // Retry with /v1/models
    if let Ok(models) = try_openai_endpoint(client, &format!("{base_url}/v1/models"), api_key) {
        return Some(models);
    }
    None
}

fn try_openai_endpoint(
    client: &reqwest::blocking::Client,
    url: &str,
    api_key: &str,
) -> Result<Vec<String>, String> {
    let mut req = client.get(url);
    if !api_key.is_empty() {
        req = req.bearer_auth(api_key);
    }
    let resp = req.send().map_err(|e| format!("Request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let body = resp.text().map_err(|e| format!("Read failed: {e}"))?;
    parse_openai_model_list(&body)
}

fn probe_anthropic(
    client: &reqwest::blocking::Client,
    base_url: &str,
    api_key: &str,
) -> Option<Vec<String>> {
    let url = format!("{base_url}/v1/models");
    let mut req = client.get(&url).header("anthropic-version", "2024-06-01");
    if !api_key.is_empty() {
        req = req.header("x-api-key", api_key);
    }
    let resp = req.send().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body = resp.text().ok()?;
    parse_openai_model_list(&body).ok()
}

fn models_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.ends_with("/v1") || base.ends_with("/v1beta") {
        format!("{base}/models")
    } else {
        format!("{base}/v1/models")
    }
}

fn fetch_openai_models(
    client: &reqwest::blocking::Client,
    base_url: &str,
    api_key: &str,
) -> Result<Vec<String>, String> {
    let url = models_url(base_url);
    let mut req = client.get(&url);
    if !api_key.is_empty() {
        req = req.bearer_auth(api_key);
    }
    let resp = req.send().map_err(|e| format!("Request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let body = resp.text().map_err(|e| format!("Read failed: {e}"))?;
    parse_openai_model_list(&body)
}

fn fetch_anthropic_models(
    client: &reqwest::blocking::Client,
    base_url: &str,
    api_key: &str,
) -> Result<Vec<String>, String> {
    let url = models_url(base_url);
    let mut req = client.get(&url).header("anthropic-version", "2024-06-01");
    if !api_key.is_empty() {
        req = req.header("x-api-key", api_key);
    }
    let resp = req.send().map_err(|e| format!("Request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let body = resp.text().map_err(|e| format!("Read failed: {e}"))?;
    parse_openai_model_list(&body)
}

fn fetch_gemini_models(
    client: &reqwest::blocking::Client,
    api_key: &str,
) -> Result<Vec<String>, String> {
    let url = format!("https://generativelanguage.googleapis.com/v1beta/models?key={api_key}");
    let body = client
        .get(&url)
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

fn parse_openai_model_list(body: &str) -> Result<Vec<String>, String> {
    let json: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("Invalid JSON: {e}"))?;
    let mut models: Vec<String> = json["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item["id"].as_str().map(ToString::to_string))
        .collect();
    if models.is_empty() {
        return Err("No models found in response".to_string());
    }
    models.sort();
    models.dedup();
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_result_construction() {
        let result = DetectResult {
            api_format: Some("openai-chat".to_string()),
            models: vec!["gpt-4".to_string(), "gpt-3.5-turbo".to_string()],
            error: None,
        };
        assert_eq!(result.api_format.as_deref(), Some("openai-chat"));
        assert_eq!(result.models.len(), 2);
        assert!(result.error.is_none());
    }

    #[test]
    fn detect_result_error_construction() {
        let result = DetectResult {
            api_format: None,
            models: vec![],
            error: Some("probe failed".to_string()),
        };
        assert!(result.api_format.is_none());
        assert!(result.models.is_empty());
        assert_eq!(result.error.as_deref(), Some("probe failed"));
    }

    #[test]
    fn known_url_anthropic() {
        assert_eq!(
            known_url_format("https://api.anthropic.com"),
            Some("anthropic".to_string())
        );
        assert_eq!(
            known_url_format("https://api.anthropic.com/v1"),
            Some("anthropic".to_string())
        );
    }

    #[test]
    fn known_url_openai() {
        assert_eq!(
            known_url_format("https://api.openai.com"),
            Some("openai-responses".to_string())
        );
        assert_eq!(
            known_url_format("https://api.openai.com/v1"),
            Some("openai-responses".to_string())
        );
    }

    #[test]
    fn known_url_google() {
        assert_eq!(
            known_url_format("https://generativelanguage.googleapis.com"),
            Some("google".to_string())
        );
    }

    #[test]
    fn known_url_openai_chat_providers() {
        let domains = [
            "https://openrouter.ai/api",
            "https://api.together.xyz",
            "https://api.groq.com/openai",
            "https://api.fireworks.ai/inference",
            "https://api.deepseek.com",
            "https://api.mistral.ai",
        ];
        for url in &domains {
            assert_eq!(
                known_url_format(url),
                Some("openai-chat".to_string()),
                "Failed for {url}"
            );
        }
    }

    #[test]
    fn known_url_unknown_returns_none() {
        assert_eq!(known_url_format("http://localhost:11434"), None);
        assert_eq!(known_url_format("https://my-custom-server.com"), None);
    }

    #[test]
    fn known_url_case_insensitive() {
        assert_eq!(
            known_url_format("https://API.OPENAI.COM"),
            Some("openai-responses".to_string())
        );
        assert_eq!(
            known_url_format("https://Api.Anthropic.Com"),
            Some("anthropic".to_string())
        );
    }

    #[test]
    fn parse_openai_model_list_valid() {
        let body = r#"{"data": [{"id": "gpt-4"}, {"id": "gpt-3.5-turbo"}, {"id": "gpt-4"}]}"#;
        let result = parse_openai_model_list(body).unwrap();
        assert_eq!(result, vec!["gpt-3.5-turbo", "gpt-4"]);
    }

    #[test]
    fn parse_openai_model_list_empty() {
        let body = r#"{"data": []}"#;
        assert!(parse_openai_model_list(body).is_err());
    }

    #[test]
    fn parse_openai_model_list_invalid_json() {
        assert!(parse_openai_model_list("not json").is_err());
    }
}
