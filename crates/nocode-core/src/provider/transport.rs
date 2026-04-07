use crate::provider::types::ProviderError;
use reqwest::blocking::Client;
use std::time::Duration;

/// HTTP transport layer for provider API calls.
pub struct HttpTransport {
    client: Client,
    base_url: String,
    api_key: String,
    extra_headers: Vec<(String, String)>,
}

impl HttpTransport {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("HTTP client should build");
        Self {
            client,
            base_url: base_url.into(),
            api_key: api_key.into(),
            extra_headers: Vec::new(),
        }
    }

    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_headers.push((key.into(), value.into()));
        self
    }

    /// POST JSON and return the response body as string.
    pub fn post_json(&self, path: &str, body: &str) -> Result<String, ProviderError> {
        let url = format!("{}{path}", self.base_url);
        let mut req = self
            .client
            .post(&url)
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {}", self.api_key));

        for (k, v) in &self.extra_headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req.body(body.to_string()).send().map_err(|e| {
            let retryable = e.is_timeout() || e.is_connect();
            ProviderError::new(format!("HTTP error: {e}"), retryable)
        })?;

        let status = resp.status();
        let text = resp
            .text()
            .map_err(|e| ProviderError::retryable(format!("Failed to read response body: {e}")))?;

        if status.is_success() {
            Ok(text)
        } else if status.as_u16() == 429 {
            Err(ProviderError::retryable(format!(
                "Rate limited (429): {text}"
            )))
        } else if status.is_server_error() {
            Err(ProviderError::retryable(format!(
                "Server error ({status}): {text}"
            )))
        } else {
            Err(ProviderError::non_retryable(format!(
                "API error ({status}): {text}"
            )))
        }
    }

    /// POST JSON and return a streaming response reader for SSE.
    pub fn post_json_stream(
        &self,
        path: &str,
        body: &str,
    ) -> Result<impl std::io::Read, ProviderError> {
        let url = format!("{}{path}", self.base_url);
        let mut req = self
            .client
            .post(&url)
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {}", self.api_key));

        for (k, v) in &self.extra_headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req.body(body.to_string()).send().map_err(|e| {
            let retryable = e.is_timeout() || e.is_connect();
            ProviderError::new(format!("HTTP error: {e}"), retryable)
        })?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().unwrap_or_default();
            return Err(if status.as_u16() == 429 || status.is_server_error() {
                ProviderError::retryable(format!("API error ({status}): {text}"))
            } else {
                ProviderError::non_retryable(format!("API error ({status}): {text}"))
            });
        }

        Ok(resp)
    }
}

/// Retry a fallible operation with exponential backoff.
pub fn with_retry<F, T>(max_attempts: u32, mut f: F) -> Result<T, ProviderError>
where
    F: FnMut() -> Result<T, ProviderError>,
{
    let mut last_err = ProviderError::non_retryable("no attempts made");
    for attempt in 0..max_attempts {
        match f() {
            Ok(val) => return Ok(val),
            Err(e) if e.retryable && attempt + 1 < max_attempts => {
                let delay = Duration::from_millis(500 * 2u64.pow(attempt));
                std::thread::sleep(delay);
                last_err = e;
            }
            Err(e) => return Err(e),
        }
    }
    Err(last_err)
}
