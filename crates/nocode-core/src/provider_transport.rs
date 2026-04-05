use crate::provider::{ModelError, ModelProvider, ProviderHttpRequest};
use reqwest::blocking::Client;
use std::collections::BTreeMap;
use std::env;
use std::io::{BufRead, BufReader, Read};
use std::time::Duration;

pub type HeaderMap = BTreeMap<String, String>;

#[derive(Clone, Debug)]
pub struct ProviderTransportConfig {
    pub base_url: String,
    pub default_headers: HeaderMap,
    pub auth: AuthConfig,
}

#[derive(Clone, Debug)]
pub enum AuthConfig {
    None,
    Bearer { token: String },
    ApiKey { header: String, value: String },
    Custom(HeaderMap),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestPlan {
    pub method: HttpMethod,
    pub url: String,
    pub headers: HeaderMap,
    pub body: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResponseEnvelope<T> {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: T,
}

impl<T> ResponseEnvelope<T> {
    pub fn new(status: u16, headers: HeaderMap, body: T) -> Self {
        Self {
            status,
            headers,
            body,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum StreamEvent {
    Data(String),
    Done,
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseFrame {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RetryConfig {
    max_attempts: usize,
    base_backoff_ms: u64,
    max_backoff_ms: u64,
}

impl RetryConfig {
    fn from_env() -> Self {
        Self {
            max_attempts: env_var_optional("NOCODE_PROVIDER_RETRY_ATTEMPTS")
                .and_then(|value| value.parse::<usize>().ok())
                .filter(|attempts| *attempts > 0)
                .unwrap_or(3),
            base_backoff_ms: env_var_optional("NOCODE_PROVIDER_RETRY_BACKOFF_MS")
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|millis| *millis > 0)
                .unwrap_or(250),
            max_backoff_ms: env_var_optional("NOCODE_PROVIDER_RETRY_MAX_BACKOFF_MS")
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|millis| *millis > 0)
                .unwrap_or(2_000),
        }
    }

    fn backoff_for_attempt(self, attempt: usize) -> Duration {
        let exponent = attempt.saturating_sub(1).min(20) as u32;
        let multiplier = 1_u64 << exponent;
        let backoff_ms = self
            .base_backoff_ms
            .saturating_mul(multiplier)
            .min(self.max_backoff_ms.max(self.base_backoff_ms));
        Duration::from_millis(backoff_ms)
    }
}

impl ProviderTransportConfig {
    pub fn for_provider(provider: ModelProvider) -> Self {
        match provider {
            ModelProvider::Mock => Self::new("mock://nocode"),
            ModelProvider::ClaudeMessages => Self::new("https://api.anthropic.com")
                .with_default_header("Accept", "application/json"),
            ModelProvider::OpenAiChatCompletions | ModelProvider::OpenAiResponses => {
                Self::new("https://api.openai.com")
                    .with_default_header("Accept", "application/json")
            }
            ModelProvider::Bedrock => Self::new("https://bedrock-runtime.us-east-1.amazonaws.com")
                .with_default_header("Accept", "application/json"),
            ModelProvider::Vertex => Self::new("https://us-central1-aiplatform.googleapis.com")
                .with_default_header("Accept", "application/json"),
        }
    }

    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            default_headers: HeaderMap::new(),
            auth: AuthConfig::None,
        }
    }

    pub fn from_env(provider: ModelProvider) -> Result<Self, ModelError> {
        let mut config = Self::for_provider(provider);
        match provider {
            ModelProvider::Mock => Ok(config),
            ModelProvider::ClaudeMessages => {
                if let Some(base_url) = env_var_optional("ANTHROPIC_BASE_URL") {
                    config.base_url = base_url;
                }
                config.auth = AuthConfig::ApiKey {
                    header: String::from("x-api-key"),
                    value: env_var_required("ANTHROPIC_API_KEY")?,
                };
                Ok(config)
            }
            ModelProvider::OpenAiChatCompletions | ModelProvider::OpenAiResponses => {
                if let Some(base_url) = env_var_optional("OPENAI_BASE_URL") {
                    config.base_url = base_url;
                }
                config.auth = AuthConfig::Bearer {
                    token: env_var_required("OPENAI_API_KEY")?,
                };
                if let Some(org_id) = env_var_optional("OPENAI_ORG_ID") {
                    config = config.with_default_header("OpenAI-Organization", org_id);
                }
                Ok(config)
            }
            ModelProvider::Bedrock => {
                if let Some(base_url) = env_var_optional("AWS_BEDROCK_BASE_URL") {
                    config.base_url = base_url;
                }
                // Bedrock uses AWS SigV4 — for now, pass region + key as bearer token placeholder.
                if let Ok(token) = std::env::var("AWS_SESSION_TOKEN") {
                    config.auth = AuthConfig::Bearer { token };
                } else if let Ok(key) = std::env::var("AWS_ACCESS_KEY_ID") {
                    config.auth = AuthConfig::ApiKey {
                        header: String::from("X-Aws-Access-Key"),
                        value: key,
                    };
                }
                Ok(config)
            }
            ModelProvider::Vertex => {
                if let Some(base_url) = env_var_optional("VERTEX_BASE_URL") {
                    config.base_url = base_url;
                }
                if let Ok(token) = std::env::var("GOOGLE_ACCESS_TOKEN") {
                    config.auth = AuthConfig::Bearer { token };
                } else if let Ok(token) = std::env::var("GCLOUD_ACCESS_TOKEN") {
                    config.auth = AuthConfig::Bearer { token };
                }
                Ok(config)
            }
        }
    }

    pub fn with_auth(mut self, auth: AuthConfig) -> Self {
        self.auth = auth;
        self
    }

    pub fn with_default_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.default_headers.insert(key.into(), value.into());
        self
    }

    pub fn prepare_request(
        &self,
        endpoint: &str,
        method: HttpMethod,
        body: Option<&str>,
    ) -> RequestPlan {
        let normalized_base = self.base_url.trim_end_matches('/');
        let normalized_endpoint = if endpoint.starts_with('/') {
            endpoint.to_string()
        } else {
            format!("/{}", endpoint)
        };

        let url = format!("{}{}", normalized_base, normalized_endpoint);

        let mut headers = self.default_headers.clone();
        self.apply_auth_headers(&mut headers);

        RequestPlan {
            method,
            url,
            headers,
            body: body.map(|value| value.to_string()),
        }
    }

    pub fn prepare_http_request(&self, request: &ProviderHttpRequest) -> RequestPlan {
        let method = match request.method.as_str() {
            "GET" => HttpMethod::Get,
            "PUT" => HttpMethod::Put,
            "PATCH" => HttpMethod::Patch,
            "DELETE" => HttpMethod::Delete,
            _ => HttpMethod::Post,
        };
        let mut plan = self.prepare_request(&request.path, method, Some(request.body.as_str()));
        for header in &request.headers {
            plan.headers
                .insert(header.name.clone(), header.value.clone());
        }
        plan
    }

    pub fn execute(
        &self,
        request: &ProviderHttpRequest,
    ) -> Result<ResponseEnvelope<String>, ModelError> {
        let plan = self.prepare_http_request(request);
        self.execute_plan_for_provider(Some(request.provider), &plan)
    }

    pub fn execute_plan(&self, plan: &RequestPlan) -> Result<ResponseEnvelope<String>, ModelError> {
        self.execute_plan_for_provider(None, plan)
    }

    fn execute_plan_for_provider(
        &self,
        provider: Option<ModelProvider>,
        plan: &RequestPlan,
    ) -> Result<ResponseEnvelope<String>, ModelError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(provider_timeout_secs()))
            .build()
            .map_err(|error| {
                ModelError::transport_failure(format!("transport client error: {error}"), true)
            })?;
        let retry_config = RetryConfig::from_env();

        execute_with_retry(
            retry_config,
            |attempt| self.execute_plan_once(provider, &client, plan, attempt),
            std::thread::sleep,
        )
    }

    pub fn execute_streaming<F>(
        &self,
        request: &ProviderHttpRequest,
        on_frame: F,
    ) -> Result<ResponseEnvelope<String>, ModelError>
    where
        F: FnMut(&SseFrame) -> Result<(), ModelError>,
    {
        let plan = self.prepare_http_request(request);
        self.execute_streaming_plan_for_provider(Some(request.provider), &plan, on_frame)
    }

    pub fn execute_streaming_plan<F>(
        &self,
        plan: &RequestPlan,
        on_frame: F,
    ) -> Result<ResponseEnvelope<String>, ModelError>
    where
        F: FnMut(&SseFrame) -> Result<(), ModelError>,
    {
        self.execute_streaming_plan_for_provider(None, plan, on_frame)
    }

    fn execute_streaming_plan_for_provider<F>(
        &self,
        provider: Option<ModelProvider>,
        plan: &RequestPlan,
        on_frame: F,
    ) -> Result<ResponseEnvelope<String>, ModelError>
    where
        F: FnMut(&SseFrame) -> Result<(), ModelError>,
    {
        let client = Client::builder()
            .timeout(Duration::from_secs(provider_timeout_secs()))
            .build()
            .map_err(|error| {
                ModelError::transport_failure(format!("transport client error: {error}"), true)
            })?;

        self.execute_plan_once_with_stream(provider, &client, plan, on_frame)
    }

    fn execute_plan_once(
        &self,
        provider: Option<ModelProvider>,
        client: &Client,
        plan: &RequestPlan,
        _attempt: usize,
    ) -> Result<ResponseEnvelope<String>, ModelError> {
        self.execute_plan_once_with_stream(provider, client, plan, |_| Ok(()))
    }

    fn execute_plan_once_with_stream<F>(
        &self,
        provider: Option<ModelProvider>,
        client: &Client,
        plan: &RequestPlan,
        mut on_frame: F,
    ) -> Result<ResponseEnvelope<String>, ModelError>
    where
        F: FnMut(&SseFrame) -> Result<(), ModelError>,
    {
        let mut builder = match plan.method {
            HttpMethod::Get => client.get(&plan.url),
            HttpMethod::Post => client.post(&plan.url),
            HttpMethod::Put => client.put(&plan.url),
            HttpMethod::Patch => client.patch(&plan.url),
            HttpMethod::Delete => client.delete(&plan.url),
        };
        for (key, value) in &plan.headers {
            builder = builder.header(key, value);
        }
        if let Some(body) = &plan.body {
            builder = builder.body(body.clone());
        }

        let response = builder.send().map_err(map_transport_error)?;
        let status = response.status();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.to_string(), value.to_string()))
            })
            .collect::<HeaderMap>();
        let content_type = headers
            .get("content-type")
            .or_else(|| headers.get("Content-Type"))
            .cloned();
        let body = read_response_body(response, content_type.as_deref(), |frame| on_frame(frame))?;

        if !status.is_success() {
            return Err(http_status_error(provider, status.as_u16(), &body));
        }

        Ok(ResponseEnvelope::new(status.as_u16(), headers, body))
    }

    pub fn apply_auth_headers(&self, headers: &mut HeaderMap) {
        match &self.auth {
            AuthConfig::None => {}
            AuthConfig::Bearer { token } => {
                headers.insert("Authorization".to_string(), format!("Bearer {}", token));
            }
            AuthConfig::ApiKey { header, value } => {
                headers.insert(header.clone(), value.clone());
            }
            AuthConfig::Custom(custom_headers) => {
                for (key, value) in custom_headers {
                    headers.insert(key.clone(), value.clone());
                }
            }
        }
    }
}

fn env_var_optional(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

fn env_var_required(name: &str) -> Result<String, ModelError> {
    env_var_optional(name).ok_or_else(|| {
        ModelError::configuration_failure(format!("missing required environment variable {name}"))
    })
}

fn provider_timeout_secs() -> u64 {
    env_var_optional("NOCODE_PROVIDER_TIMEOUT_SECS")
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .unwrap_or(60)
}

fn execute_with_retry<F, S>(
    retry_config: RetryConfig,
    mut execute: F,
    mut sleep: S,
) -> Result<ResponseEnvelope<String>, ModelError>
where
    F: FnMut(usize) -> Result<ResponseEnvelope<String>, ModelError>,
    S: FnMut(Duration),
{
    let max_attempts = retry_config.max_attempts.max(1);

    for attempt in 1..=max_attempts {
        match execute(attempt) {
            Ok(response) => return Ok(response),
            Err(error) if error.retryable && attempt < max_attempts => {
                sleep(retry_config.backoff_for_attempt(attempt));
            }
            Err(error) => return Err(finalize_retry_error(error, attempt)),
        }
    }

    Err(ModelError::provider_failure(
        "provider transport exhausted without result",
        true,
    ))
}

fn finalize_retry_error(error: ModelError, attempts: usize) -> ModelError {
    if attempts <= 1 {
        return error;
    }
    ModelError {
        message: format!("{} (after {attempts} attempts)", error.message),
        ..error
    }
}

fn map_transport_error(error: reqwest::Error) -> ModelError {
    let retryable = error.is_timeout() || error.is_connect() || error.is_request();
    ModelError::transport_failure(format!("transport request failed: {error}"), retryable)
}

fn http_status_error(provider: Option<ModelProvider>, status: u16, body: &str) -> ModelError {
    let retryable = status == 429 || status >= 500;
    let snippet = body.chars().take(240).collect::<String>();
    ModelError::http_status(provider, status, snippet, retryable)
}

pub fn parse_stream_chunk(chunk: &str) -> StreamEvent {
    let trimmed = chunk.trim();

    if trimmed.is_empty() {
        return StreamEvent::Done;
    }

    if let Some(rest) = trimmed.strip_prefix("error:") {
        return StreamEvent::Error(rest.trim().to_string());
    }

    StreamEvent::Data(trimmed.to_string())
}

pub fn parse_sse_frames(body: &str) -> Vec<SseFrame> {
    let cursor = std::io::Cursor::new(body.as_bytes());
    collect_sse_frames(BufReader::new(cursor), |_| Ok(())).unwrap_or_default()
}

fn read_response_body<F>(
    response: reqwest::blocking::Response,
    content_type: Option<&str>,
    on_frame: F,
) -> Result<String, ModelError>
where
    F: FnMut(&SseFrame) -> Result<(), ModelError>,
{
    if content_type.is_some_and(|value| value.contains("text/event-stream")) {
        return collect_sse_frames(BufReader::new(response), on_frame).map(render_sse_frames);
    }

    let mut reader = BufReader::new(response);
    let mut prelude = String::new();
    loop {
        let mut line = String::new();
        let bytes_read = reader.read_line(&mut line).map_err(map_io_error)?;
        if bytes_read == 0 {
            return Ok(prelude);
        }
        prelude.push_str(line.as_str());
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed.is_empty() {
            continue;
        }
        if looks_like_sse_line(trimmed) {
            return collect_sse_frames_with_prelude(reader, prelude, on_frame)
                .map(render_sse_frames);
        }
        break;
    }

    reader.read_to_string(&mut prelude).map_err(map_io_error)?;
    Ok(prelude)
}

fn collect_sse_frames<R, F>(mut reader: R, mut on_frame: F) -> Result<Vec<SseFrame>, ModelError>
where
    R: BufRead,
    F: FnMut(&SseFrame) -> Result<(), ModelError>,
{
    let mut frames = Vec::new();
    let mut current_event: Option<String> = None;
    let mut current_data = Vec::new();
    let mut line = String::new();

    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line).map_err(map_io_error)?;
        if bytes_read == 0 {
            flush_sse_frame(
                &mut frames,
                &mut current_event,
                &mut current_data,
                &mut on_frame,
            )?;
            break;
        }

        process_sse_line(
            line.trim_end_matches(['\n', '\r']),
            &mut frames,
            &mut current_event,
            &mut current_data,
            &mut on_frame,
        )?;
    }

    Ok(frames)
}

fn collect_sse_frames_with_prelude<R, F>(
    mut reader: R,
    prelude: String,
    mut on_frame: F,
) -> Result<Vec<SseFrame>, ModelError>
where
    R: BufRead,
    F: FnMut(&SseFrame) -> Result<(), ModelError>,
{
    let mut frames = Vec::new();
    let mut current_event: Option<String> = None;
    let mut current_data = Vec::new();
    for line in prelude.lines() {
        process_sse_line(
            line.trim_end_matches(['\n', '\r']),
            &mut frames,
            &mut current_event,
            &mut current_data,
            &mut on_frame,
        )?;
    }

    let mut line = String::new();
    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line).map_err(map_io_error)?;
        if bytes_read == 0 {
            flush_sse_frame(
                &mut frames,
                &mut current_event,
                &mut current_data,
                &mut on_frame,
            )?;
            break;
        }
        process_sse_line(
            line.trim_end_matches(['\n', '\r']),
            &mut frames,
            &mut current_event,
            &mut current_data,
            &mut on_frame,
        )?;
    }
    Ok(frames)
}

fn process_sse_line<F>(
    trimmed: &str,
    frames: &mut Vec<SseFrame>,
    current_event: &mut Option<String>,
    current_data: &mut Vec<String>,
    on_frame: &mut F,
) -> Result<(), ModelError>
where
    F: FnMut(&SseFrame) -> Result<(), ModelError>,
{
    if trimmed.is_empty() {
        return flush_sse_frame(frames, current_event, current_data, on_frame);
    }
    if trimmed.starts_with(':') {
        return Ok(());
    }
    if let Some(event) = trimmed.strip_prefix("event:") {
        *current_event = Some(event.trim().to_string());
        return Ok(());
    }
    if let Some(data) = trimmed.strip_prefix("data:") {
        current_data.push(data.trim_start().to_string());
    }
    Ok(())
}

fn looks_like_sse_line(trimmed: &str) -> bool {
    trimmed.starts_with("event:") || trimmed.starts_with("data:") || trimmed.starts_with(':')
}

fn flush_sse_frame<F>(
    frames: &mut Vec<SseFrame>,
    event: &mut Option<String>,
    data: &mut Vec<String>,
    on_frame: &mut F,
) -> Result<(), ModelError>
where
    F: FnMut(&SseFrame) -> Result<(), ModelError>,
{
    if data.is_empty() {
        *event = None;
        return Ok(());
    }

    let frame = SseFrame {
        event: event.take(),
        data: data.join("\n"),
    };
    on_frame(&frame)?;
    frames.push(frame);
    data.clear();
    Ok(())
}

fn render_sse_frames(frames: Vec<SseFrame>) -> String {
    let mut body = String::new();
    for frame in frames {
        if let Some(event) = frame.event {
            body.push_str("event: ");
            body.push_str(event.as_str());
            body.push('\n');
        }
        for line in frame.data.lines() {
            body.push_str("data: ");
            body.push_str(line);
            body.push('\n');
        }
        body.push('\n');
    }
    body
}

fn map_io_error(error: std::io::Error) -> ModelError {
    ModelError::transport_failure(format!("transport stream read failed: {error}"), true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{ModelProvider, ProviderHttpRequest};

    #[test]
    fn prepare_request_applies_headers_and_auth() {
        let config = ProviderTransportConfig::new("https://api.example.com/v1")
            .with_default_header("Content-Type", "application/json")
            .with_auth(AuthConfig::Bearer {
                token: "tok".to_string(),
            });

        let plan = config.prepare_request("items", HttpMethod::Post, Some("{\"foo\":\"bar\"}"));

        assert_eq!(plan.url, "https://api.example.com/v1/items");
        assert_eq!(plan.method, HttpMethod::Post);
        assert_eq!(plan.body.unwrap(), "{\"foo\":\"bar\"}".to_string());
        assert_eq!(
            plan.headers.get("Content-Type").map(String::as_str),
            Some("application/json")
        );
        assert_eq!(
            plan.headers.get("Authorization").map(String::as_str),
            Some("Bearer tok")
        );
    }

    #[test]
    fn response_envelope_preserves_state() {
        let mut headers = HeaderMap::new();
        headers.insert("x-trace".to_string(), "abc".to_string());

        let envelope = ResponseEnvelope::new(200, headers.clone(), "ok".to_string());

        assert_eq!(envelope.status, 200);
        assert_eq!(envelope.headers, headers);
        assert_eq!(envelope.body, "ok".to_string());
    }

    #[test]
    fn parse_stream_chunk_detects_data_and_done() {
        assert_eq!(
            parse_stream_chunk("payload"),
            StreamEvent::Data("payload".to_string())
        );
        assert_eq!(parse_stream_chunk(""), StreamEvent::Done);
    }

    #[test]
    fn parse_stream_chunk_detects_error() {
        assert_eq!(
            parse_stream_chunk("error: bad thing"),
            StreamEvent::Error("bad thing".to_string())
        );
    }

    #[test]
    fn parse_sse_frames_reads_event_and_data_blocks() {
        let frames = parse_sse_frames(concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\"}\n\n",
            "data: {\"type\":\"message_delta\"}\n",
            "data: {\"delta\":\"x\"}\n\n"
        ));

        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].event.as_deref(), Some("message_start"));
        assert_eq!(frames[0].data, "{\"type\":\"message_start\"}");
        assert_eq!(
            frames[1].data,
            "{\"type\":\"message_delta\"}\n{\"delta\":\"x\"}"
        );
    }

    #[test]
    fn collect_sse_frames_invokes_callback_live() {
        let cursor = std::io::Cursor::new(b"data: one\n\ndata: two\n\n".as_slice());
        let mut seen = Vec::new();
        let frames = collect_sse_frames(BufReader::new(cursor), |frame| {
            seen.push(frame.data.clone());
            Ok(())
        })
        .expect("frames should parse");

        assert_eq!(seen, vec![String::from("one"), String::from("two")]);
        assert_eq!(frames.len(), 2);
    }

    #[test]
    fn looks_like_sse_line_detects_event_stream_markers() {
        assert!(looks_like_sse_line("event: message_start"));
        assert!(looks_like_sse_line("data: {\"delta\":\"x\"}"));
        assert!(looks_like_sse_line(": ping"));
        assert!(!looks_like_sse_line("{\"json\":true}"));
    }

    #[test]
    fn collect_sse_frames_with_prelude_keeps_live_callbacks() {
        let cursor = std::io::Cursor::new(b"data: two\n\n".as_slice());
        let mut seen = Vec::new();
        let frames = collect_sse_frames_with_prelude(
            BufReader::new(cursor),
            String::from("data: one\n\n"),
            |frame| {
                seen.push(frame.data.clone());
                Ok(())
            },
        )
        .expect("prelude frames should parse");

        assert_eq!(seen, vec![String::from("one"), String::from("two")]);
        assert_eq!(frames.len(), 2);
    }

    #[test]
    fn default_provider_config_uses_expected_base_url() {
        let anthropic = ProviderTransportConfig::for_provider(ModelProvider::ClaudeMessages);
        let openai = ProviderTransportConfig::for_provider(ModelProvider::OpenAiResponses);

        assert_eq!(anthropic.base_url, "https://api.anthropic.com");
        assert_eq!(openai.base_url, "https://api.openai.com");
    }

    #[test]
    fn prepare_http_request_merges_protocol_headers() {
        let config = ProviderTransportConfig::for_provider(ModelProvider::ClaudeMessages)
            .with_auth(AuthConfig::ApiKey {
                header: "x-api-key".to_string(),
                value: "secret".to_string(),
            });
        let request = ProviderHttpRequest::new(
            "POST",
            ModelProvider::ClaudeMessages,
            serde_json::json!({"model":"claude"}),
        )
        .with_header("anthropic-version", "2023-06-01");

        let plan = config.prepare_http_request(&request);

        assert_eq!(plan.url, "https://api.anthropic.com/v1/messages");
        assert_eq!(
            plan.headers.get("anthropic-version").map(String::as_str),
            Some("2023-06-01")
        );
        assert_eq!(
            plan.headers.get("x-api-key").map(String::as_str),
            Some("secret")
        );
    }

    #[test]
    fn execute_with_retry_retries_retryable_failures() {
        let retry = RetryConfig {
            max_attempts: 3,
            base_backoff_ms: 25,
            max_backoff_ms: 200,
        };
        let mut attempts = 0usize;
        let mut sleeps = Vec::new();

        let response = execute_with_retry(
            retry,
            |_| {
                attempts += 1;
                if attempts == 1 {
                    Err(ModelError::provider_failure("timeout", true))
                } else {
                    Ok(ResponseEnvelope::new(
                        200,
                        HeaderMap::new(),
                        "ok".to_string(),
                    ))
                }
            },
            |duration| sleeps.push(duration),
        )
        .expect("retry should recover");

        assert_eq!(attempts, 2);
        assert_eq!(response.body, "ok");
        assert_eq!(sleeps, vec![Duration::from_millis(25)]);
    }

    #[test]
    fn execute_with_retry_stops_on_non_retryable_failure() {
        let retry = RetryConfig {
            max_attempts: 4,
            base_backoff_ms: 10,
            max_backoff_ms: 100,
        };
        let mut attempts = 0usize;
        let mut sleeps = Vec::new();

        let error = execute_with_retry(
            retry,
            |_| {
                attempts += 1;
                Err(ModelError::provider_failure("bad request", false))
            },
            |duration| sleeps.push(duration),
        )
        .expect_err("non-retryable errors should stop immediately");

        assert_eq!(attempts, 1);
        assert!(sleeps.is_empty());
        assert_eq!(error.message, "bad request");
    }

    #[test]
    fn execute_with_retry_caps_backoff_and_reports_attempts() {
        let retry = RetryConfig {
            max_attempts: 3,
            base_backoff_ms: 100,
            max_backoff_ms: 150,
        };
        let mut attempts = 0usize;
        let mut sleeps = Vec::new();

        let error = execute_with_retry(
            retry,
            |_| {
                attempts += 1;
                Err(ModelError::provider_failure("provider timeout", true))
            },
            |duration| sleeps.push(duration),
        )
        .expect_err("retries should eventually exhaust");

        assert_eq!(attempts, 3);
        assert_eq!(
            sleeps,
            vec![Duration::from_millis(100), Duration::from_millis(150)]
        );
        assert!(error.retryable);
        assert!(error.message.contains("after 3 attempts"));
    }
}
