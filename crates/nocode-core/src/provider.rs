use crate::message::{QueryMessage, QueryMessageRole};
use crate::provider_transport::{HeaderMap, RequestPlan, SseFrame, parse_sse_frames};
use crate::query_loop::{QuerySource, TaskBudget};
use jsonschema::JSONSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModelProvider {
    #[default]
    Mock,
    ClaudeMessages,
    OpenAiChatCompletions,
    OpenAiResponses,
    Bedrock,
    Vertex,
}

impl ModelProvider {
    pub const ALL: [Self; 6] = [
        Self::Mock,
        Self::ClaudeMessages,
        Self::OpenAiChatCompletions,
        Self::OpenAiResponses,
        Self::Bedrock,
        Self::Vertex,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mock => "mock",
            Self::ClaudeMessages => "claude-messages",
            Self::OpenAiChatCompletions => "openai-chat-completions",
            Self::OpenAiResponses => "openai-responses",
            Self::Bedrock => "bedrock",
            Self::Vertex => "vertex",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "mock" => Some(Self::Mock),
            "claude-messages" | "claude" | "anthropic" => Some(Self::ClaudeMessages),
            "openai-chat-completions" | "openai-chat" => Some(Self::OpenAiChatCompletions),
            "openai-responses" | "openai" | "responses" => Some(Self::OpenAiResponses),
            "bedrock" | "aws-bedrock" => Some(Self::Bedrock),
            "vertex" | "google-vertex" | "vertex-ai" => Some(Self::Vertex),
            _ => None,
        }
    }

    pub const fn request_path(self) -> &'static str {
        match self {
            Self::Mock => "/mock",
            Self::ClaudeMessages => "/v1/messages",
            Self::OpenAiChatCompletions => "/v1/chat/completions",
            Self::OpenAiResponses => "/v1/responses",
            Self::Bedrock => "/model/{model}/invoke",
            Self::Vertex => {
                "/v1/projects/{project}/locations/{location}/publishers/anthropic/models/{model}:streamRawPredict"
            }
        }
    }

    pub const fn capabilities(self) -> ModelProviderCapabilities {
        match self {
            Self::Mock => ModelProviderCapabilities {
                supports_streaming: true,
                live_streaming: false,
                uses_sse_transport: false,
                supports_tool_use: false,
                supports_json_schema: false,
                supports_reasoning_effort: false,
            },
            Self::ClaudeMessages => ModelProviderCapabilities {
                supports_streaming: true,
                live_streaming: true,
                uses_sse_transport: true,
                supports_tool_use: true,
                supports_json_schema: false,
                supports_reasoning_effort: false,
            },
            Self::OpenAiChatCompletions => ModelProviderCapabilities {
                supports_streaming: true,
                live_streaming: true,
                uses_sse_transport: true,
                supports_tool_use: true,
                supports_json_schema: true,
                supports_reasoning_effort: true,
            },
            Self::OpenAiResponses => ModelProviderCapabilities {
                supports_streaming: true,
                live_streaming: true,
                uses_sse_transport: true,
                supports_tool_use: true,
                supports_json_schema: true,
                supports_reasoning_effort: true,
            },
            Self::Bedrock => ModelProviderCapabilities {
                supports_streaming: true,
                live_streaming: true,
                uses_sse_transport: true,
                supports_tool_use: false,
                supports_json_schema: false,
                supports_reasoning_effort: false,
            },
            Self::Vertex => ModelProviderCapabilities {
                supports_streaming: true,
                live_streaming: true,
                uses_sse_transport: true,
                supports_tool_use: false,
                supports_json_schema: false,
                supports_reasoning_effort: false,
            },
        }
    }

    pub fn capability_summary(self) -> String {
        self.capabilities().summary()
    }

    pub fn capability_matrix_entry(self) -> String {
        format!("{}[{}]", self.as_str(), self.capability_summary())
    }

    pub fn capability_matrix_summary() -> String {
        Self::ALL
            .iter()
            .map(|provider| provider.capability_matrix_entry())
            .collect::<Vec<_>>()
            .join(" | ")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelProviderCapabilities {
    pub supports_streaming: bool,
    pub live_streaming: bool,
    pub uses_sse_transport: bool,
    pub supports_tool_use: bool,
    pub supports_json_schema: bool,
    pub supports_reasoning_effort: bool,
}

impl ModelProviderCapabilities {
    pub fn summary(self) -> String {
        format!(
            "stream(request={},live={},sse={}) tool-use={} json-schema={} reasoning={}",
            bool_label(self.supports_streaming),
            bool_label(self.live_streaming),
            bool_label(self.uses_sse_transport),
            bool_label(self.supports_tool_use),
            bool_label(self.supports_json_schema),
            bool_label(self.supports_reasoning_effort)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModelStreamMode {
    #[default]
    Disabled,
    Enabled,
}

impl ModelStreamMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Enabled => "enabled",
        }
    }

    pub const fn as_bool(self) -> bool {
        matches!(self, Self::Enabled)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSelection {
    pub provider: ModelProvider,
    pub requested_model: Option<String>,
    pub fallback_model: Option<String>,
}

impl ModelSelection {
    pub fn selected_model(&self) -> Option<&str> {
        self.requested_model
            .as_deref()
            .or(self.fallback_model.as_deref())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRequest {
    pub selection: ModelSelection,
    pub system_prompt: Vec<QueryMessage>,
    pub conversation: Vec<QueryMessage>,
    pub model_reasoning_effort: Option<String>,
    pub json_schema: Option<String>,
    pub query_source: QuerySource,
    pub stream_mode: ModelStreamMode,
    pub max_turns: Option<u32>,
    pub task_budget: Option<TaskBudget>,
    pub verbose: bool,
    pub replay_user_messages: bool,
    pub include_partial_messages: bool,
    pub tool_definitions: Vec<ToolSchema>,
}

/// Minimal tool schema for provider API requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

impl ModelRequest {
    pub fn reply_target(&self) -> Option<&str> {
        self.conversation
            .iter()
            .rev()
            .find(|message| message.role == QueryMessageRole::User)
            .map(|message| message.content.as_str())
    }

    pub fn to_http_request(&self) -> Result<ProviderHttpRequest, ModelError> {
        self.to_http_request_for_stream_mode(self.stream_mode)
    }

    pub fn to_transport_http_request(&self) -> Result<ProviderHttpRequest, ModelError> {
        self.to_http_request_for_stream_mode(self.transport_stream_mode())
    }

    fn transport_stream_mode(&self) -> ModelStreamMode {
        self.stream_mode
    }

    fn to_http_request_for_stream_mode(
        &self,
        stream_mode: ModelStreamMode,
    ) -> Result<ProviderHttpRequest, ModelError> {
        self.validate_capabilities()?;
        let model = self
            .selection
            .selected_model()
            .ok_or_else(ModelError::no_model_configured)?;
        match self.selection.provider {
            ModelProvider::Mock => Ok(ProviderHttpRequest::new(
                "POST",
                self.selection.provider,
                json!({
                    "provider": self.selection.provider.as_str(),
                    "model": model,
                    "reply_target": self.reply_target(),
                    "stream": stream_mode.as_bool(),
                }),
            )),
            ModelProvider::ClaudeMessages => {
                let mut body = json!({
                    "model": model,
                    "max_tokens": 4096,
                    "system": join_message_content(self.system_prompt.as_slice()),
                    "messages": self
                        .conversation
                        .iter()
                        .map(ClaudeMessage::from_query_message)
                        .collect::<Vec<_>>(),
                    "stream": stream_mode.as_bool(),
                });
                if !self.tool_definitions.is_empty() {
                    body["tools"] = json!(
                        self.tool_definitions
                            .iter()
                            .map(|t| json!({
                                "name": t.name,
                                "description": t.description,
                                "input_schema": t.parameters,
                            }))
                            .collect::<Vec<_>>()
                    );
                }
                Ok(
                    ProviderHttpRequest::new("POST", self.selection.provider, body)
                        .with_header("anthropic-version", "2023-06-01"),
                )
            }
            ModelProvider::OpenAiChatCompletions => Ok(ProviderHttpRequest::new(
                "POST",
                self.selection.provider,
                openai_chat_request_body(self, model, stream_mode)?,
            )),
            ModelProvider::OpenAiResponses => Ok(ProviderHttpRequest::new(
                "POST",
                self.selection.provider,
                openai_responses_request_body(self, model, stream_mode)?,
            )),
            // Bedrock uses Claude Messages format with different endpoint.
            ModelProvider::Bedrock => Ok(ProviderHttpRequest::new(
                "POST",
                self.selection.provider,
                json!({
                    "anthropic_version": "bedrock-2023-05-31",
                    "model": model,
                    "system": join_message_content(self.system_prompt.as_slice()),
                    "messages": self
                        .conversation
                        .iter()
                        .map(ClaudeMessage::from_query_message)
                        .collect::<Vec<_>>(),
                    "max_tokens": 4096,
                    "stream": stream_mode.as_bool(),
                }),
            )),
            // Vertex uses Claude Messages format with Google endpoint.
            ModelProvider::Vertex => Ok(ProviderHttpRequest::new(
                "POST",
                self.selection.provider,
                json!({
                    "anthropic_version": "vertex-2023-10-16",
                    "model": model,
                    "system": join_message_content(self.system_prompt.as_slice()),
                    "messages": self
                        .conversation
                        .iter()
                        .map(ClaudeMessage::from_query_message)
                        .collect::<Vec<_>>(),
                    "max_tokens": 4096,
                    "stream": stream_mode.as_bool(),
                }),
            )),
        }
    }

    fn validate_capabilities(&self) -> Result<(), ModelError> {
        let caps = self.selection.provider.capabilities();
        if self.stream_mode.as_bool() && !caps.supports_streaming {
            return Err(ModelError::configuration_failure(format!(
                "provider {} does not support streaming requests",
                self.selection.provider.as_str()
            )));
        }
        if self.json_schema.is_some() && !caps.supports_json_schema {
            return Err(ModelError::configuration_failure(format!(
                "provider {} does not support json schema requests",
                self.selection.provider.as_str()
            )));
        }
        if self.model_reasoning_effort.is_some() && !caps.supports_reasoning_effort {
            return Err(ModelError::configuration_failure(format!(
                "provider {} does not support reasoning effort requests",
                self.selection.provider.as_str()
            )));
        }
        Ok(())
    }

    fn json_schema_response_format(&self) -> Result<Option<Value>, ModelError> {
        let Some(schema) = self.json_schema.as_deref() else {
            return Ok(None);
        };
        let parsed = parse_json_schema(schema, self.selection.provider)?;
        Ok(Some(json!({
            "type": "json_schema",
            "json_schema": {
                "name": "structured_output",
                "schema": parsed,
            }
        })))
    }

    fn responses_text_format(&self) -> Result<Option<Value>, ModelError> {
        let Some(schema) = self.json_schema.as_deref() else {
            return Ok(None);
        };
        let parsed = parse_json_schema(schema, self.selection.provider)?;
        Ok(Some(json!({
            "type": "json_schema",
            "name": "structured_output",
            "schema": parsed,
        })))
    }
}

fn openai_chat_request_body(
    request: &ModelRequest,
    model: &str,
    stream_mode: ModelStreamMode,
) -> Result<Value, ModelError> {
    let mut body = json!({
        "model": model,
        "messages": openai_chat_messages(request),
        "stream": stream_mode.as_bool(),
    });
    if let Some(reasoning_effort) = request.model_reasoning_effort.as_deref() {
        body["reasoning_effort"] = json!(reasoning_effort);
    }
    if let Some(schema) = request.json_schema_response_format()? {
        body["response_format"] = schema;
    }
    Ok(body)
}

fn openai_responses_request_body(
    request: &ModelRequest,
    model: &str,
    stream_mode: ModelStreamMode,
) -> Result<Value, ModelError> {
    let mut body = json!({
        "model": model,
        "input": openai_responses_input(request),
        "stream": stream_mode.as_bool(),
    });
    if let Some(reasoning_effort) = request.model_reasoning_effort.as_deref() {
        body["reasoning"] = json!({ "effort": reasoning_effort });
    }
    if let Some(schema) = request.responses_text_format()? {
        body["text"] = json!({ "format": schema });
    }
    Ok(body)
}

fn parse_json_schema(schema: &str, provider: ModelProvider) -> Result<Value, ModelError> {
    serde_json::from_str::<Value>(schema).map_err(|error| {
        ModelError::configuration_failure(format!(
            "provider {} received invalid json schema: {error}",
            provider.as_str()
        ))
    })
}

fn finalize_model_output(
    request: &ModelRequest,
    model: impl Into<String>,
    text: String,
) -> Result<ModelCallOutput, ModelError> {
    let provider = request.selection.provider;
    let model = model.into();
    if let Some(response_result) = validate_structured_output(request, text.as_str())? {
        let canonical = response_result.to_string();
        return Ok(
            ModelCallOutput::new(provider, model, QueryMessage::assistant(canonical))
                .with_response_result(response_result),
        );
    }
    Ok(ModelCallOutput::new(
        provider,
        model,
        QueryMessage::assistant(text),
    ))
}

fn validate_structured_output(
    request: &ModelRequest,
    text: &str,
) -> Result<Option<Value>, ModelError> {
    let Some(schema) = request.json_schema.as_deref() else {
        return Ok(None);
    };
    let provider = request.selection.provider;
    let schema = parse_json_schema(schema, provider)?;
    let candidate = parse_structured_output_candidate(text, provider)?;
    let validator = JSONSchema::compile(&schema).map_err(|error| {
        ModelError::configuration_failure(format!(
            "provider {} received unsupported json schema: {error}",
            provider.as_str()
        ))
    })?;
    let errors = validator
        .validate(&candidate)
        .err()
        .into_iter()
        .flatten()
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    if !errors.is_empty() {
        return Err(ModelError::structured_output_failure(
            provider,
            format!(
                "structured output did not match schema: {}",
                errors.join(", ")
            ),
        ));
    }
    Ok(Some(candidate))
}

fn parse_structured_output_candidate(
    text: &str,
    provider: ModelProvider,
) -> Result<Value, ModelError> {
    let trimmed = text.trim();
    let candidate = [Some(trimmed), strip_markdown_code_fence(trimmed)]
        .into_iter()
        .flatten()
        .find_map(|candidate| serde_json::from_str::<Value>(candidate).ok());
    candidate.ok_or_else(|| {
        ModelError::structured_output_failure(provider, "structured output was not valid JSON")
    })
}

fn strip_markdown_code_fence(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    if !trimmed.starts_with("```") || !trimmed.ends_with("```") {
        return None;
    }
    let inner = &trimmed[3..trimmed.len().saturating_sub(3)];
    let inner = inner.trim_start();
    let inner = match inner.find('\n') {
        Some(newline) => &inner[newline + 1..],
        None => inner,
    };
    Some(inner.trim())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelStreamEvent {
    Start {
        provider: ModelProvider,
        model: String,
    },
    Delta {
        text: String,
    },
    Complete {
        message: QueryMessage,
    },
}

impl ModelStreamEvent {
    pub const fn kind_label(&self) -> &'static str {
        match self {
            Self::Start { .. } => "start",
            Self::Delta { .. } => "delta",
            Self::Complete { .. } => "complete",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ModelStreamEventWire {
    Start { provider: String, model: String },
    Delta { text: String },
    Complete { role: String, content: String },
}

impl ModelStreamEventWire {
    pub fn into_event(self) -> Result<ModelStreamEvent, String> {
        match self {
            Self::Start { provider, model } => {
                let provider = ModelProvider::parse(provider.as_str())
                    .ok_or_else(|| format!("unknown model provider: {provider}"))?;
                Ok(ModelStreamEvent::Start { provider, model })
            }
            Self::Delta { text } => Ok(ModelStreamEvent::Delta { text }),
            Self::Complete { role, content } => {
                let role = QueryMessageRole::parse(role.as_str())
                    .ok_or_else(|| format!("unknown query message role: {role}"))?;
                Ok(ModelStreamEvent::Complete {
                    message: QueryMessage::new(role, content),
                })
            }
        }
    }
}

impl From<&ModelStreamEvent> for ModelStreamEventWire {
    fn from(value: &ModelStreamEvent) -> Self {
        match value {
            ModelStreamEvent::Start { provider, model } => Self::Start {
                provider: provider.as_str().to_string(),
                model: model.clone(),
            },
            ModelStreamEvent::Delta { text } => Self::Delta { text: text.clone() },
            ModelStreamEvent::Complete { message } => Self::Complete {
                role: message.role.as_str().to_string(),
                content: message.content.clone(),
            },
        }
    }
}

pub trait ModelStreamSink {
    fn push(&mut self, event: ModelStreamEvent);
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RecordingModelStream {
    pub events: Vec<ModelStreamEvent>,
}

impl ModelStreamSink for RecordingModelStream {
    fn push(&mut self, event: ModelStreamEvent) {
        self.events.push(event);
    }
}

/// A stream sink that sends events through an `mpsc` channel for cross-thread consumption.
pub struct ChannelModelStreamSink {
    sender: std::sync::mpsc::Sender<ModelStreamEvent>,
}

impl ChannelModelStreamSink {
    pub fn new(sender: std::sync::mpsc::Sender<ModelStreamEvent>) -> Self {
        Self { sender }
    }
}

impl ModelStreamSink for ChannelModelStreamSink {
    fn push(&mut self, event: ModelStreamEvent) {
        let _ = self.sender.send(event);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderHttpRequest {
    pub method: String,
    pub provider: ModelProvider,
    pub path: String,
    pub headers: Vec<ProviderHeader>,
    pub body: String,
}

impl ProviderHttpRequest {
    pub fn new(method: impl Into<String>, provider: ModelProvider, body: Value) -> Self {
        Self {
            method: method.into(),
            provider,
            path: provider.request_path().to_string(),
            headers: vec![ProviderHeader {
                name: String::from("content-type"),
                value: String::from("application/json"),
            }],
            body: body.to_string(),
        }
    }

    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push(ProviderHeader {
            name: name.into(),
            value: value.into(),
        });
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCallOutput {
    pub provider: ModelProvider,
    pub model: String,
    pub message: QueryMessage,
    pub response_result: Option<Value>,
}

impl ModelCallOutput {
    pub fn new(provider: ModelProvider, model: impl Into<String>, message: QueryMessage) -> Self {
        Self {
            provider,
            model: model.into(),
            message,
            response_result: None,
        }
    }

    pub fn with_response_result(mut self, response_result: Value) -> Self {
        self.response_result = Some(response_result);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInvocation {
    pub provider: ModelProvider,
    pub requested_model: Option<String>,
    pub model: String,
    pub fallback_model: Option<String>,
    pub stream_mode: ModelStreamMode,
    pub stream_events: Vec<ModelStreamEvent>,
    pub http_request: ProviderHttpRequest,
    pub transport_request: RequestPlan,
    pub output_message: QueryMessage,
    pub response_result: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelErrorWire {
    pub surface: String,
    pub kind: String,
    pub provider: Option<String>,
    pub status_code: Option<u16>,
    pub status_class: Option<String>,
    pub retryable: bool,
    pub message: String,
}

impl From<&ModelError> for ModelErrorWire {
    fn from(value: &ModelError) -> Self {
        Self {
            surface: value.surface_label().to_string(),
            kind: value.kind.as_str().to_string(),
            provider: value.provider.map(|provider| provider.as_str().to_string()),
            status_code: value.status_code,
            status_class: value.status_class.map(|class| class.as_str().to_string()),
            retryable: value.retryable,
            message: value.message.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModelStreamStats {
    pub total_events: usize,
    pub delta_events: usize,
    pub delta_chars: usize,
    pub started: bool,
    pub completed: bool,
}

impl ModelStreamStats {
    pub fn summary(&self) -> String {
        format!(
            "total={} delta={} chars={} start={} complete={}",
            self.total_events,
            self.delta_events,
            self.delta_chars,
            bool_label(self.started),
            bool_label(self.completed)
        )
    }
}

impl ModelInvocation {
    pub fn from_call(
        request: &ModelRequest,
        http_request: ProviderHttpRequest,
        transport_request: RequestPlan,
        output: ModelCallOutput,
        stream_events: Vec<ModelStreamEvent>,
    ) -> Self {
        let response_result = output.response_result.clone();
        Self {
            provider: output.provider,
            requested_model: request.selection.requested_model.clone(),
            model: output.model,
            fallback_model: request.selection.fallback_model.clone(),
            stream_mode: request.stream_mode,
            stream_events,
            http_request,
            transport_request,
            output_message: output.message,
            response_result,
        }
    }

    pub fn stream_stats(&self) -> ModelStreamStats {
        let mut stats = ModelStreamStats::default();
        for event in &self.stream_events {
            stats.total_events += 1;
            match event {
                ModelStreamEvent::Start { .. } => stats.started = true,
                ModelStreamEvent::Delta { text } => {
                    stats.delta_events += 1;
                    stats.delta_chars += text.chars().count();
                }
                ModelStreamEvent::Complete { .. } => stats.completed = true,
            }
        }
        stats
    }

    pub fn stream_summary(&self) -> String {
        self.stream_stats().summary()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelErrorKind {
    NoModelConfigured,
    Configuration,
    StructuredOutputFailure,
    Transport,
    HttpStatus,
    InvalidProviderResponse,
    ProviderFailure,
}

impl ModelErrorKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoModelConfigured => "no_model_configured",
            Self::Configuration => "configuration",
            Self::StructuredOutputFailure => "structured_output_failure",
            Self::Transport => "transport",
            Self::HttpStatus => "http_status",
            Self::InvalidProviderResponse => "invalid_provider_response",
            Self::ProviderFailure => "provider_failure",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpStatusClass {
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    RateLimited,
    OtherClient,
    ServerError,
    Other,
}

impl HttpStatusClass {
    pub const fn classify(status: u16) -> Self {
        match status {
            401 => Self::Unauthorized,
            403 => Self::Forbidden,
            404 => Self::NotFound,
            409 => Self::Conflict,
            429 => Self::RateLimited,
            400..=499 => Self::OtherClient,
            500..=599 => Self::ServerError,
            _ => Self::Other,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unauthorized => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::RateLimited => "rate_limited",
            Self::OtherClient => "other_client",
            Self::ServerError => "server_error",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelError {
    pub kind: ModelErrorKind,
    pub message: String,
    pub retryable: bool,
    pub provider: Option<ModelProvider>,
    pub status_code: Option<u16>,
    pub status_class: Option<HttpStatusClass>,
}

impl ModelError {
    pub fn no_model_configured() -> Self {
        Self {
            kind: ModelErrorKind::NoModelConfigured,
            message: String::from("no model configured"),
            retryable: false,
            provider: None,
            status_code: None,
            status_class: None,
        }
    }

    pub fn provider_failure(message: impl Into<String>, retryable: bool) -> Self {
        Self {
            kind: ModelErrorKind::ProviderFailure,
            message: message.into(),
            retryable,
            provider: None,
            status_code: None,
            status_class: None,
        }
    }

    pub fn provider_response_error(
        provider: ModelProvider,
        context: &str,
        message: impl Into<String>,
        retryable: bool,
    ) -> Self {
        Self {
            kind: ModelErrorKind::ProviderFailure,
            message: format!(
                "provider {} {} error: {}",
                provider.as_str(),
                context,
                message.into()
            ),
            retryable,
            provider: Some(provider),
            status_code: None,
            status_class: None,
        }
    }

    pub fn configuration_failure(message: impl Into<String>) -> Self {
        Self {
            kind: ModelErrorKind::Configuration,
            message: message.into(),
            retryable: false,
            provider: None,
            status_code: None,
            status_class: None,
        }
    }

    pub fn transport_failure(message: impl Into<String>, retryable: bool) -> Self {
        Self {
            kind: ModelErrorKind::Transport,
            message: message.into(),
            retryable,
            provider: None,
            status_code: None,
            status_class: None,
        }
    }

    pub fn structured_output_failure(provider: ModelProvider, message: impl Into<String>) -> Self {
        Self {
            kind: ModelErrorKind::StructuredOutputFailure,
            message: message.into(),
            retryable: false,
            provider: Some(provider),
            status_code: None,
            status_class: None,
        }
    }

    pub fn http_status(
        provider: Option<ModelProvider>,
        status: u16,
        message: impl Into<String>,
        retryable: bool,
    ) -> Self {
        Self {
            kind: ModelErrorKind::HttpStatus,
            message: format!("provider returned HTTP {status}: {}", message.into()),
            retryable,
            provider,
            status_code: Some(status),
            status_class: Some(HttpStatusClass::classify(status)),
        }
    }

    pub fn invalid_provider_response(provider: ModelProvider, detail: impl Into<String>) -> Self {
        Self {
            kind: ModelErrorKind::InvalidProviderResponse,
            message: format!("invalid {} response: {}", provider.as_str(), detail.into()),
            retryable: false,
            provider: Some(provider),
            status_code: None,
            status_class: None,
        }
    }

    pub fn surface_label(&self) -> &'static str {
        match self.kind {
            ModelErrorKind::NoModelConfigured => "no_model_configured",
            ModelErrorKind::Configuration => "configuration",
            ModelErrorKind::StructuredOutputFailure => "structured_output_failure",
            ModelErrorKind::InvalidProviderResponse => "invalid_provider_response",
            ModelErrorKind::ProviderFailure => {
                classify_provider_failure_surface(self.message.as_str())
            }
            ModelErrorKind::Transport => classify_transport_surface(self.message.as_str()),
            ModelErrorKind::HttpStatus => self
                .status_class
                .map(HttpStatusClass::as_str)
                .unwrap_or("http_status"),
        }
    }
}

fn classify_transport_surface(message: &str) -> &'static str {
    let lowered = message.to_ascii_lowercase();
    if lowered.contains("timed out") || lowered.contains("timeout") {
        "timeout"
    } else if lowered.contains("connect") || lowered.contains("connection") {
        "connectivity"
    } else if lowered.contains("stream read") || lowered.contains("decode") {
        "transport_io"
    } else {
        "transport"
    }
}

fn classify_provider_failure_surface(message: &str) -> &'static str {
    let lowered = message.to_ascii_lowercase();
    if lowered.contains("rate limit")
        || lowered.contains("too many requests")
        || lowered.contains("rate_limit")
    {
        "rate_limited"
    } else if lowered.contains("unauthorized") || lowered.contains("authentication") {
        "unauthorized"
    } else if lowered.contains("forbidden") {
        "forbidden"
    } else if lowered.contains("timed out") || lowered.contains("timeout") {
        "timeout"
    } else if lowered.contains("overloaded")
        || lowered.contains("unavailable")
        || lowered.contains("internal")
        || lowered.contains("server")
    {
        "server_error"
    } else {
        "provider_failure"
    }
}

fn provider_payload_error(
    provider: ModelProvider,
    payload: &Value,
    context: &str,
) -> Option<ModelError> {
    let error = payload.get("error")?;
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| error.get("detail").and_then(Value::as_str))
        .unwrap_or("provider returned error payload");
    let error_type = error.get("type").and_then(Value::as_str);
    let error_code = error.get("code").and_then(Value::as_str);
    let retryable = [message, error_type.unwrap_or(""), error_code.unwrap_or("")]
        .iter()
        .any(|value| {
            let lowered = value.to_ascii_lowercase();
            lowered.contains("timeout")
                || lowered.contains("timed out")
                || lowered.contains("rate limit")
                || lowered.contains("rate_limit")
                || lowered.contains("too_many_requests")
                || lowered.contains("unavailable")
                || lowered.contains("overloaded")
                || lowered.contains("internal")
        });
    let decorated = match (error_type, error_code) {
        (Some(kind), Some(code)) => format!("type={kind} code={code} message={message}"),
        (Some(kind), None) => format!("type={kind} message={message}"),
        (None, Some(code)) => format!("code={code} message={message}"),
        (None, None) => message.to_string(),
    };
    Some(ModelError::provider_response_error(
        provider, context, decorated, retryable,
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedStreamResponse {
    pub output: ModelCallOutput,
    pub events: Vec<ModelStreamEvent>,
}

#[derive(Debug, Serialize)]
struct ClaudeTextBlock<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    text: &'a str,
}

#[derive(Debug, Serialize)]
struct ClaudeMessage<'a> {
    role: &'a str,
    content: Vec<ClaudeTextBlock<'a>>,
}

impl<'a> ClaudeMessage<'a> {
    fn from_query_message(message: &'a QueryMessage) -> Self {
        Self {
            role: openai_role_label(message.role),
            content: vec![ClaudeTextBlock {
                kind: "text",
                text: message.content.as_str(),
            }],
        }
    }
}

fn join_message_content(messages: &[QueryMessage]) -> String {
    messages
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn openai_chat_messages(request: &ModelRequest) -> Vec<Value> {
    let mut messages = Vec::new();
    messages.extend(
        request
            .system_prompt
            .iter()
            .map(|message| json!({"role": "system", "content": message.content})),
    );
    messages.extend(request.conversation.iter().map(|message| {
        json!({
            "role": openai_role_label(message.role),
            "content": message.content,
        })
    }));
    messages
}

fn openai_responses_input(request: &ModelRequest) -> Vec<Value> {
    let mut input = Vec::new();
    input.extend(request.system_prompt.iter().map(|message| {
        json!({
            "role": "system",
            "content": [{"type": "input_text", "text": message.content}],
        })
    }));
    input.extend(request.conversation.iter().map(|message| {
        json!({
            "role": openai_role_label(message.role),
            "content": [{"type": "input_text", "text": message.content}],
        })
    }));
    input
}

fn openai_role_label(role: QueryMessageRole) -> &'static str {
    match role {
        QueryMessageRole::System => "system",
        QueryMessageRole::User => "user",
        QueryMessageRole::Assistant => "assistant",
        QueryMessageRole::Tool => "tool",
    }
}

const fn bool_label(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

pub(crate) fn parse_model_response(
    request: &ModelRequest,
    body: &str,
) -> Result<ModelCallOutput, ModelError> {
    let payload: Value = serde_json::from_str(body).map_err(|error| {
        ModelError::invalid_provider_response(
            request.selection.provider,
            format!("body was not valid JSON: {error}"),
        )
    })?;
    if let Some(error) = provider_payload_error(request.selection.provider, &payload, "response") {
        return Err(error);
    }
    let model = payload
        .get("model")
        .and_then(Value::as_str)
        .or(request.selection.selected_model())
        .unwrap_or("unknown");
    let text = match request.selection.provider {
        ModelProvider::Mock => request
            .reply_target()
            .map(|target| format!("nocode response: {target}"))
            .unwrap_or_else(|| format!("nocode response: {model}")),
        ModelProvider::ClaudeMessages | ModelProvider::Bedrock | ModelProvider::Vertex => {
            parse_claude_message_text(&payload)?
        }
        ModelProvider::OpenAiChatCompletions => parse_openai_chat_message_text(&payload)?,
        ModelProvider::OpenAiResponses => parse_openai_responses_text(&payload)?,
    };

    finalize_model_output(request, model, text)
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn response_looks_like_stream(headers: &HeaderMap, body: &str) -> bool {
    headers
        .get("content-type")
        .or_else(|| headers.get("Content-Type"))
        .map(|value| value.contains("text/event-stream"))
        .unwrap_or(false)
        || body.lines().any(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with("event:") || trimmed.starts_with("data:")
        })
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn parse_streaming_model_response(
    request: &ModelRequest,
    body: &str,
) -> Result<ParsedStreamResponse, ModelError> {
    let frames = parse_sse_frames(body);
    if frames.is_empty() {
        return Err(ModelError::invalid_provider_response(
            request.selection.provider,
            "empty SSE stream body",
        ));
    }

    let mut parser = StreamingModelParser::new(request);
    let mut stream = RecordingModelStream::default();
    for frame in frames {
        parser.push_frame(&frame, &mut stream)?;
    }

    Ok(ParsedStreamResponse {
        output: parser.finish(&mut stream)?,
        events: stream.events,
    })
}

fn parse_claude_message_text(payload: &Value) -> Result<String, ModelError> {
    let provider = ModelProvider::ClaudeMessages;
    let text = payload
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("")
        })
        .filter(|text| !text.is_empty());
    text.ok_or_else(|| {
        ModelError::invalid_provider_response(provider, "missing text block in content array")
    })
}

fn parse_openai_chat_message_text(payload: &Value) -> Result<String, ModelError> {
    let provider = ModelProvider::OpenAiChatCompletions;
    let content = payload
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"));

    extract_openai_text_content(content).ok_or_else(|| {
        ModelError::invalid_provider_response(provider, "missing assistant message content")
    })
}

fn parse_openai_responses_text(payload: &Value) -> Result<String, ModelError> {
    let provider = ModelProvider::OpenAiResponses;
    if let Some(text) = payload
        .get("output_text")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        return Ok(text.to_string());
    }

    let text = payload
        .get("output")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|item| {
                    item.get("type").and_then(Value::as_str) == Some("message")
                        || item.get("role").is_some()
                })
                .filter_map(|item| extract_openai_text_content(item.get("content")))
                .collect::<Vec<_>>()
                .join("")
        })
        .filter(|text| !text.is_empty());

    text.ok_or_else(|| {
        ModelError::invalid_provider_response(provider, "missing output text in response body")
    })
}

fn extract_openai_text_content(content: Option<&Value>) -> Option<String> {
    match content {
        Some(Value::String(text)) => Some(text.clone()),
        Some(Value::Array(items)) => {
            let text = items
                .iter()
                .filter_map(extract_text_fragment)
                .collect::<Vec<_>>()
                .join("");
            (!text.is_empty()).then_some(text)
        }
        _ => None,
    }
}

fn extract_text_fragment(item: &Value) -> Option<String> {
    if let Some(text) = item.get("text").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    item.get("text")
        .and_then(|value| value.get("value"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| {
            item.get("type")
                .and_then(Value::as_str)
                .filter(|kind| *kind == "output_text" || *kind == "text" || *kind == "input_text")
                .and_then(|_| item.get("value").and_then(Value::as_str))
                .map(ToString::to_string)
        })
}

pub(crate) struct StreamingModelParser<'a> {
    request: &'a ModelRequest,
    model: String,
    text: String,
    fallback_payload: Option<Value>,
}

impl<'a> StreamingModelParser<'a> {
    pub(crate) fn new(request: &'a ModelRequest) -> Self {
        Self {
            request,
            model: request
                .selection
                .selected_model()
                .unwrap_or("unknown")
                .to_string(),
            text: String::new(),
            fallback_payload: None,
        }
    }

    pub(crate) fn push_frame(
        &mut self,
        frame: &SseFrame,
        stream: &mut dyn ModelStreamSink,
    ) -> Result<(), ModelError> {
        if frame.data.trim() == "[DONE]" {
            return Ok(());
        }
        let payload: Value = serde_json::from_str(frame.data.as_str()).map_err(|error| {
            ModelError::invalid_provider_response(
                self.request.selection.provider,
                format!("stream event was not valid JSON: {error}"),
            )
        })?;

        match self.request.selection.provider {
            ModelProvider::Mock => Ok(()),
            ModelProvider::ClaudeMessages | ModelProvider::Bedrock | ModelProvider::Vertex => {
                self.parse_claude_stream_frame(&payload, stream)
            }
            ModelProvider::OpenAiChatCompletions => {
                self.parse_openai_chat_stream_frame(&payload, stream)
            }
            ModelProvider::OpenAiResponses => {
                self.parse_openai_responses_stream_frame(&payload, frame.event.as_deref(), stream)
            }
        }
    }

    pub(crate) fn finish(
        self,
        stream: &mut dyn ModelStreamSink,
    ) -> Result<ModelCallOutput, ModelError> {
        let mut text = self.text;
        if text.is_empty()
            && let Some(payload) = self.fallback_payload.as_ref()
        {
            let fallback_text = match self.request.selection.provider {
                ModelProvider::ClaudeMessages | ModelProvider::Bedrock | ModelProvider::Vertex => {
                    parse_claude_message_text(payload)?
                }
                ModelProvider::OpenAiChatCompletions => parse_openai_chat_message_text(payload)?,
                ModelProvider::OpenAiResponses => parse_openai_responses_text(payload)?,
                ModelProvider::Mock => String::new(),
            };
            if !fallback_text.is_empty() {
                stream.push(ModelStreamEvent::Delta {
                    text: fallback_text.clone(),
                });
                text = fallback_text;
            }
        }

        if text.is_empty() {
            return Err(ModelError::invalid_provider_response(
                self.request.selection.provider,
                "stream produced no assistant text",
            ));
        }

        finalize_model_output(self.request, self.model, text)
    }

    fn parse_claude_stream_frame(
        &mut self,
        payload: &Value,
        stream: &mut dyn ModelStreamSink,
    ) -> Result<(), ModelError> {
        if let Some(next_model) = payload
            .get("message")
            .and_then(|message| message.get("model"))
            .and_then(Value::as_str)
        {
            self.model = next_model.to_string();
        }

        if let Some(error) =
            provider_payload_error(ModelProvider::ClaudeMessages, payload, "stream")
        {
            return Err(error);
        }

        if let Some(delta) = payload
            .get("delta")
            .and_then(|delta| delta.get("text"))
            .and_then(Value::as_str)
            .filter(|delta| !delta.is_empty())
        {
            self.push_stream_delta(delta, stream);
            return Ok(());
        }

        if let Some(block_text) = payload
            .get("content_block")
            .and_then(|block| block.get("text"))
            .and_then(Value::as_str)
            .filter(|delta| !delta.is_empty())
        {
            self.push_stream_delta(block_text, stream);
        }

        Ok(())
    }

    fn parse_openai_chat_stream_frame(
        &mut self,
        payload: &Value,
        stream: &mut dyn ModelStreamSink,
    ) -> Result<(), ModelError> {
        if let Some(next_model) = payload.get("model").and_then(Value::as_str) {
            self.model = next_model.to_string();
        }

        if let Some(error) =
            provider_payload_error(ModelProvider::OpenAiChatCompletions, payload, "stream")
        {
            return Err(error);
        }

        let delta_content = payload
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("delta"))
            .and_then(|delta| delta.get("content"));

        if let Some(fragment) =
            extract_openai_text_content(delta_content).filter(|delta| !delta.is_empty())
        {
            self.push_stream_delta(fragment.as_str(), stream);
        }

        Ok(())
    }

    fn parse_openai_responses_stream_frame(
        &mut self,
        payload: &Value,
        event_name: Option<&str>,
        stream: &mut dyn ModelStreamSink,
    ) -> Result<(), ModelError> {
        if let Some(next_model) = payload.get("model").and_then(Value::as_str).or_else(|| {
            payload
                .get("response")
                .and_then(|response| response.get("model"))
                .and_then(Value::as_str)
        }) {
            self.model = next_model.to_string();
        }

        if let Some(error) =
            provider_payload_error(ModelProvider::OpenAiResponses, payload, "stream")
        {
            return Err(error);
        }

        let kind = payload.get("type").and_then(Value::as_str).or(event_name);
        if let Some(delta) = payload
            .get("delta")
            .and_then(Value::as_str)
            .filter(|delta| !delta.is_empty())
        {
            self.push_stream_delta(delta, stream);
            return Ok(());
        }

        if let Some(part_text) =
            extract_openai_text_content(payload.get("part")).filter(|delta| !delta.is_empty())
            && kind.is_some_and(|value| value.contains("delta") || value.contains("added"))
        {
            self.push_stream_delta(part_text.as_str(), stream);
            return Ok(());
        }

        if let Some(content_text) =
            extract_openai_text_content(payload.get("content")).filter(|delta| !delta.is_empty())
            && kind.is_some_and(|value| value.contains("delta") || value.contains("added"))
        {
            self.push_stream_delta(content_text.as_str(), stream);
            return Ok(());
        }

        if let Some(response) = payload.get("response") {
            self.fallback_payload = Some(response.clone());
        } else if kind.is_some_and(|value| value.contains("completed")) {
            self.fallback_payload = Some(payload.clone());
        }

        Ok(())
    }

    fn push_stream_delta(&mut self, fragment: &str, stream: &mut dyn ModelStreamSink) {
        self.text.push_str(fragment);
        stream.push(ModelStreamEvent::Delta {
            text: fragment.to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{
        HttpStatusClass, ModelCallOutput, ModelError, ModelErrorKind, ModelInvocation,
        ModelProvider, ModelRequest, ModelSelection, ModelStreamEvent, ModelStreamMode,
        ModelStreamSink, RecordingModelStream, parse_model_response,
        parse_streaming_model_response, response_looks_like_stream,
    };
    use crate::message::QueryMessage;
    use crate::provider_transport::HeaderMap;
    use crate::query_loop::{QuerySource, TaskBudget};
    use serde_json::{Value, json};

    fn sample_request(provider: ModelProvider) -> ModelRequest {
        ModelRequest {
            selection: ModelSelection {
                provider,
                requested_model: Some(String::from("sonnet")),
                fallback_model: Some(String::from("haiku")),
            },
            system_prompt: vec![QueryMessage::system("system")],
            conversation: vec![
                QueryMessage::user("first"),
                QueryMessage::assistant("ack"),
                QueryMessage::user("second"),
            ],
            model_reasoning_effort: None,
            json_schema: None,
            query_source: QuerySource::Sdk,
            stream_mode: ModelStreamMode::Enabled,
            max_turns: Some(4),
            task_budget: Some(TaskBudget { total: 10_000 }),
            verbose: true,
            replay_user_messages: false,
            include_partial_messages: false,
            tool_definitions: Vec::new(),
        }
    }

    #[test]
    fn model_selection_prefers_requested_model() {
        let selection = ModelSelection {
            provider: ModelProvider::Mock,
            requested_model: Some(String::from("sonnet")),
            fallback_model: Some(String::from("haiku")),
        };

        assert_eq!(selection.selected_model(), Some("sonnet"));
    }

    #[test]
    fn provider_capabilities_report_live_streaming_support() {
        let mock = ModelProvider::Mock.capabilities();
        let chat = ModelProvider::OpenAiChatCompletions.capabilities();
        let responses = ModelProvider::OpenAiResponses.capabilities();

        assert!(!mock.live_streaming);
        assert!(chat.supports_json_schema);
        assert!(chat.supports_reasoning_effort);
        assert!(responses.live_streaming);
        assert!(responses.uses_sse_transport);
        assert!(responses.supports_json_schema);
        assert!(responses.supports_reasoning_effort);
        assert_eq!(
            ModelProvider::ClaudeMessages.capability_summary(),
            "stream(request=yes,live=yes,sse=yes) tool-use=yes json-schema=no reasoning=no"
        );
        assert_eq!(
            ModelProvider::OpenAiChatCompletions.capability_summary(),
            "stream(request=yes,live=yes,sse=yes) tool-use=yes json-schema=yes reasoning=yes"
        );
        assert_eq!(
            ModelProvider::OpenAiResponses.capability_summary(),
            "stream(request=yes,live=yes,sse=yes) tool-use=yes json-schema=yes reasoning=yes"
        );
        assert!(ModelProvider::capability_matrix_summary().contains("mock["));
        assert!(ModelProvider::capability_matrix_summary().contains("claude-messages["));
        assert!(ModelProvider::capability_matrix_summary().contains("openai-chat-completions["));
        assert!(ModelProvider::capability_matrix_summary().contains("openai-responses["));
    }

    #[test]
    fn reasoning_effort_is_rejected_for_unsupported_provider() {
        let mut request = sample_request(ModelProvider::ClaudeMessages);
        request.model_reasoning_effort = Some(String::from("high"));

        let error = request
            .to_http_request()
            .expect_err("unsupported reasoning should fail");

        assert_eq!(error.kind, ModelErrorKind::Configuration);
        assert!(error.message.contains("does not support reasoning effort"));
        assert_eq!(error.surface_label(), "configuration");
    }

    #[test]
    fn json_schema_is_rejected_for_unsupported_provider() {
        let mut request = sample_request(ModelProvider::ClaudeMessages);
        request.json_schema = Some(String::from("{\"type\":\"object\"}"));

        let error = request
            .to_http_request()
            .expect_err("unsupported json schema should fail");

        assert_eq!(error.kind, ModelErrorKind::Configuration);
        assert!(error.message.contains("does not support json schema"));
    }

    #[test]
    fn invalid_json_schema_is_rejected_for_supported_provider() {
        let mut request = sample_request(ModelProvider::OpenAiChatCompletions);
        request.json_schema = Some(String::from("{not-json"));

        let error = request
            .to_http_request()
            .expect_err("invalid schema should fail before request build");

        assert_eq!(error.kind, ModelErrorKind::Configuration);
        assert!(error.message.contains("invalid json schema"));
    }

    #[test]
    fn invalid_json_schema_is_rejected_for_supported_responses_provider() {
        let mut request = sample_request(ModelProvider::OpenAiResponses);
        request.json_schema = Some(String::from("{not-json"));

        let error = request
            .to_http_request()
            .expect_err("invalid schema should fail before responses request build");

        assert_eq!(error.kind, ModelErrorKind::Configuration);
        assert!(error.message.contains("invalid json schema"));
    }

    #[test]
    fn model_request_reply_target_uses_last_user_message() {
        let request = sample_request(ModelProvider::Mock);
        assert_eq!(request.reply_target(), Some("second"));
    }

    #[test]
    fn model_invocation_keeps_stream_events() {
        let request = sample_request(ModelProvider::Mock);
        let mut stream = RecordingModelStream::default();
        stream.push(ModelStreamEvent::Start {
            provider: ModelProvider::Mock,
            model: String::from("sonnet"),
        });
        let invocation = ModelInvocation::from_call(
            &request,
            request
                .to_http_request()
                .expect("http request should build"),
            crate::provider_transport::ProviderTransportConfig::for_provider(ModelProvider::Mock)
                .prepare_http_request(
                    &request
                        .to_http_request()
                        .expect("http request should build"),
                ),
            ModelCallOutput::new(
                ModelProvider::Mock,
                "sonnet",
                QueryMessage::assistant("done"),
            )
            .with_response_result(json!({"ok": true})),
            stream.events,
        );

        assert_eq!(invocation.model, "sonnet");
        assert_eq!(invocation.stream_mode, ModelStreamMode::Enabled);
        assert_eq!(invocation.stream_events.len(), 1);
        assert_eq!(
            invocation.stream_summary(),
            "total=1 delta=0 chars=0 start=yes complete=no"
        );
        assert_eq!(invocation.http_request.path, "/mock");
        assert_eq!(invocation.transport_request.url, "mock://nocode/mock");
        assert_eq!(invocation.response_result, Some(json!({"ok": true})));
    }

    #[test]
    fn provider_failure_preserves_retryability() {
        let error = ModelError::provider_failure("timeout", true);

        assert_eq!(error.kind, ModelErrorKind::ProviderFailure);
        assert!(error.retryable);
        assert_eq!(error.surface_label(), "timeout");
    }

    #[test]
    fn invalid_provider_response_has_specific_kind() {
        let error =
            ModelError::invalid_provider_response(ModelProvider::OpenAiResponses, "bad payload");

        assert_eq!(error.kind, ModelErrorKind::InvalidProviderResponse);
        assert!(!error.retryable);
        assert_eq!(error.provider, Some(ModelProvider::OpenAiResponses));
    }

    #[test]
    fn structured_output_failure_has_specific_kind() {
        let error =
            ModelError::structured_output_failure(ModelProvider::OpenAiResponses, "bad output");

        assert_eq!(error.kind, ModelErrorKind::StructuredOutputFailure);
        assert!(!error.retryable);
        assert_eq!(error.provider, Some(ModelProvider::OpenAiResponses));
    }

    #[test]
    fn http_status_error_tracks_classification() {
        let error =
            ModelError::http_status(Some(ModelProvider::ClaudeMessages), 429, "slow down", true);

        assert_eq!(error.kind, ModelErrorKind::HttpStatus);
        assert_eq!(error.provider, Some(ModelProvider::ClaudeMessages));
        assert_eq!(error.status_code, Some(429));
        assert_eq!(error.status_class, Some(HttpStatusClass::RateLimited));
        assert_eq!(
            HttpStatusClass::classify(401),
            HttpStatusClass::Unauthorized
        );
        assert_eq!(HttpStatusClass::classify(503), HttpStatusClass::ServerError);
        assert_eq!(error.surface_label(), "rate_limited");
    }

    #[test]
    fn claude_messages_adapter_uses_messages_endpoint() {
        let request = sample_request(ModelProvider::ClaudeMessages);
        let http = request
            .to_http_request()
            .expect("http request should build");
        let body: Value = serde_json::from_str(&http.body).expect("body should parse");

        assert_eq!(http.path, "/v1/messages");
        assert_eq!(body["model"], "sonnet");
        assert_eq!(body["system"], "system");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"][0]["type"], "text");
    }

    #[test]
    fn openai_chat_completions_adapter_uses_messages_array() {
        let request = sample_request(ModelProvider::OpenAiChatCompletions);
        let http = request
            .to_http_request()
            .expect("http request should build");
        let body: Value = serde_json::from_str(&http.body).expect("body should parse");

        assert_eq!(http.path, "/v1/chat/completions");
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["messages"][3]["content"], "second");
    }

    #[test]
    fn openai_chat_completions_adapter_encodes_json_schema_response_format() {
        let mut request = sample_request(ModelProvider::OpenAiChatCompletions);
        request.json_schema = Some(String::from(
            "{\"type\":\"object\",\"properties\":{\"name\":{\"type\":\"string\"}},\"required\":[\"name\"]}",
        ));
        let http = request
            .to_http_request()
            .expect("json schema request should build");
        let body: Value = serde_json::from_str(&http.body).expect("body should parse");

        assert_eq!(http.path, "/v1/chat/completions");
        assert_eq!(body["response_format"]["type"], "json_schema");
        assert_eq!(
            body["response_format"]["json_schema"]["name"],
            "structured_output"
        );
        assert_eq!(
            body["response_format"]["json_schema"]["schema"]["type"],
            "object"
        );
        assert_eq!(
            body["response_format"]["json_schema"]["schema"]["properties"]["name"]["type"],
            "string"
        );
    }

    #[test]
    fn openai_responses_adapter_uses_input_array() {
        let request = sample_request(ModelProvider::OpenAiResponses);
        let http = request
            .to_http_request()
            .expect("http request should build");
        let body: Value = serde_json::from_str(&http.body).expect("body should parse");

        assert_eq!(http.path, "/v1/responses");
        assert_eq!(body["input"][0]["role"], "system");
        assert_eq!(body["input"][1]["content"][0]["type"], "input_text");
        assert_eq!(body["input"][3]["content"][0]["text"], "second");
    }

    #[test]
    fn openai_responses_adapter_encodes_json_schema_text_format() {
        let mut request = sample_request(ModelProvider::OpenAiResponses);
        request.json_schema = Some(String::from(
            "{\"type\":\"object\",\"properties\":{\"name\":{\"type\":\"string\"}},\"required\":[\"name\"]}",
        ));
        let http = request
            .to_http_request()
            .expect("json schema request should build");
        let body: Value = serde_json::from_str(&http.body).expect("body should parse");

        assert_eq!(http.path, "/v1/responses");
        assert_eq!(body["text"]["format"]["type"], "json_schema");
        assert_eq!(body["text"]["format"]["name"], "structured_output");
        assert_eq!(body["text"]["format"]["schema"]["type"], "object");
        assert_eq!(
            body["text"]["format"]["schema"]["properties"]["name"]["type"],
            "string"
        );
    }

    #[test]
    fn transport_request_keeps_remote_streaming_enabled() {
        let request = sample_request(ModelProvider::OpenAiResponses);
        let http = request
            .to_transport_http_request()
            .expect("transport http request should build");
        let body: Value = serde_json::from_str(&http.body).expect("body should parse");

        assert_eq!(body["stream"], true);
    }

    #[test]
    fn parse_claude_response_extracts_text_blocks() {
        let request = sample_request(ModelProvider::ClaudeMessages);
        let output = parse_model_response(
            &request,
            r#"{"model":"claude-3-7-sonnet","content":[{"type":"text","text":"alpha "},{"type":"text","text":"beta"}]}"#,
        )
        .expect("claude response should parse");

        assert_eq!(output.model, "claude-3-7-sonnet");
        assert_eq!(output.message, QueryMessage::assistant("alpha beta"));
    }

    #[test]
    fn parse_openai_chat_response_extracts_message_content() {
        let request = sample_request(ModelProvider::OpenAiChatCompletions);
        let output = parse_model_response(
            &request,
            r#"{"model":"gpt-4.1","choices":[{"message":{"content":"chat output"}}]}"#,
        )
        .expect("chat response should parse");

        assert_eq!(output.model, "gpt-4.1");
        assert_eq!(output.message, QueryMessage::assistant("chat output"));
    }

    #[test]
    fn parse_openai_chat_response_surfaces_provider_payload_error() {
        let request = sample_request(ModelProvider::OpenAiChatCompletions);
        let error = parse_model_response(
            &request,
            r#"{"error":{"type":"rate_limit_error","code":"too_many_requests","message":"slow down"}}"#,
        )
        .expect_err("payload error should map to provider failure");

        assert_eq!(error.kind, ModelErrorKind::ProviderFailure);
        assert!(error.retryable);
        assert_eq!(error.provider, Some(ModelProvider::OpenAiChatCompletions));
        assert_eq!(error.surface_label(), "rate_limited");
        assert!(error.message.contains("type=rate_limit_error"));
        assert!(error.message.contains("code=too_many_requests"));
    }

    #[test]
    fn parse_openai_chat_response_validates_structured_output() {
        let mut request = sample_request(ModelProvider::OpenAiChatCompletions);
        request.json_schema = Some(String::from(
            "{\"type\":\"object\",\"properties\":{\"ok\":{\"type\":\"boolean\"}},\"required\":[\"ok\"]}",
        ));
        let output = parse_model_response(
            &request,
            r#"{"model":"gpt-4.1","choices":[{"message":{"content":"{\"ok\":true}"}}]}"#,
        )
        .expect("chat structured output should parse");

        assert_eq!(output.message, QueryMessage::assistant("{\"ok\":true}"));
        assert_eq!(output.response_result, Some(json!({"ok": true})));
    }

    #[test]
    fn parse_openai_chat_response_accepts_markdown_fenced_structured_output() {
        let mut request = sample_request(ModelProvider::OpenAiChatCompletions);
        request.json_schema = Some(String::from(
            "{\"type\":\"object\",\"properties\":{\"ok\":{\"type\":\"boolean\"}},\"required\":[\"ok\"]}",
        ));
        let output = parse_model_response(
            &request,
            r#"{"model":"gpt-4.1","choices":[{"message":{"content":"```json\n{\"ok\":true}\n```"}}]}"#,
        )
        .expect("fenced json should parse");

        assert_eq!(output.message, QueryMessage::assistant("{\"ok\":true}"));
        assert_eq!(output.response_result, Some(json!({"ok": true})));
    }

    #[test]
    fn parse_openai_chat_response_rejects_schema_mismatch_as_structured_output_failure() {
        let mut request = sample_request(ModelProvider::OpenAiChatCompletions);
        request.json_schema = Some(String::from(
            "{\"type\":\"object\",\"properties\":{\"ok\":{\"type\":\"boolean\"}},\"required\":[\"ok\"]}",
        ));
        let error = parse_model_response(
            &request,
            r#"{"model":"gpt-4.1","choices":[{"message":{"content":"{\"ok\":\"wrong\"}"}}]}"#,
        )
        .expect_err("schema mismatch should fail");

        assert_eq!(error.kind, ModelErrorKind::StructuredOutputFailure);
        assert!(error.message.contains("did not match schema"));
    }

    #[test]
    fn parse_openai_chat_response_rejects_non_json_as_structured_output_failure() {
        let mut request = sample_request(ModelProvider::OpenAiChatCompletions);
        request.json_schema = Some(String::from(
            "{\"type\":\"object\",\"properties\":{\"ok\":{\"type\":\"boolean\"}},\"required\":[\"ok\"]}",
        ));
        let error = parse_model_response(
            &request,
            r#"{"model":"gpt-4.1","choices":[{"message":{"content":"not-json"}}]}"#,
        )
        .expect_err("non-json output should fail");

        assert_eq!(error.kind, ModelErrorKind::StructuredOutputFailure);
        assert!(error.message.contains("not valid JSON"));
    }

    #[test]
    fn parse_openai_responses_response_extracts_output_array_text() {
        let request = sample_request(ModelProvider::OpenAiResponses);
        let output = parse_model_response(
            &request,
            r#"{"model":"gpt-4.1","output":[{"type":"message","content":[{"type":"output_text","text":"response output"}]}]}"#,
        )
        .expect("responses output should parse");

        assert_eq!(output.model, "gpt-4.1");
        assert_eq!(output.message, QueryMessage::assistant("response output"));
    }

    #[test]
    fn parse_openai_responses_response_validates_structured_output() {
        let mut request = sample_request(ModelProvider::OpenAiResponses);
        request.json_schema = Some(String::from(
            "{\"type\":\"object\",\"properties\":{\"ok\":{\"type\":\"boolean\"}},\"required\":[\"ok\"]}",
        ));
        let output = parse_model_response(
            &request,
            r#"{"model":"gpt-4.1","output":[{"type":"message","content":[{"type":"output_text","text":"{\"ok\":true}"}]}]}"#,
        )
        .expect("responses structured output should parse");

        assert_eq!(output.message, QueryMessage::assistant("{\"ok\":true}"));
        assert_eq!(output.response_result, Some(json!({"ok": true})));
    }

    #[test]
    fn stream_detector_uses_header_or_sse_lines() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "content-type".to_string(),
            "text/event-stream; charset=utf-8".to_string(),
        );

        assert!(response_looks_like_stream(&headers, ""));
        assert!(response_looks_like_stream(
            &HeaderMap::new(),
            "data: {\"delta\":\"x\"}\n\n"
        ));
    }

    #[test]
    fn parse_claude_streaming_response_extracts_text_deltas() {
        let request = sample_request(ModelProvider::ClaudeMessages);
        let parsed = parse_streaming_model_response(
            &request,
            concat!(
                "event: message_start\n",
                "data: {\"type\":\"message_start\",\"message\":{\"model\":\"claude-3-7-sonnet\"}}\n\n",
                "event: content_block_delta\n",
                "data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"alpha \"}}\n\n",
                "event: content_block_delta\n",
                "data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"beta\"}}\n\n",
                "event: message_stop\n",
                "data: {\"type\":\"message_stop\"}\n\n"
            ),
        )
        .expect("claude stream should parse");

        assert_eq!(parsed.output.model, "claude-3-7-sonnet");
        assert_eq!(parsed.output.message, QueryMessage::assistant("alpha beta"));
        assert_eq!(parsed.events.len(), 2);
    }

    #[test]
    fn parse_openai_chat_streaming_response_extracts_text_deltas() {
        let request = sample_request(ModelProvider::OpenAiChatCompletions);
        let parsed = parse_streaming_model_response(
            &request,
            concat!(
                "data: {\"model\":\"gpt-4.1\",\"choices\":[{\"delta\":{\"content\":\"chat \"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"stream\"}}]}\n\n",
                "data: [DONE]\n\n"
            ),
        )
        .expect("chat stream should parse");

        assert_eq!(parsed.output.model, "gpt-4.1");
        assert_eq!(
            parsed.output.message,
            QueryMessage::assistant("chat stream")
        );
        assert_eq!(parsed.events.len(), 2);
    }

    #[test]
    fn parse_openai_chat_streaming_response_validates_structured_output() {
        let mut request = sample_request(ModelProvider::OpenAiChatCompletions);
        request.json_schema = Some(String::from(
            "{\"type\":\"object\",\"properties\":{\"ok\":{\"type\":\"boolean\"}},\"required\":[\"ok\"]}",
        ));
        let parsed = parse_streaming_model_response(
            &request,
            concat!(
                "data: {\"model\":\"gpt-4.1\",\"choices\":[{\"delta\":{\"content\":\"{\\\"ok\\\":\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"true}\"}}]}\n\n",
                "data: [DONE]\n\n"
            ),
        )
        .expect("chat structured stream should parse");

        assert_eq!(
            parsed.output.message,
            QueryMessage::assistant("{\"ok\":true}")
        );
        assert_eq!(parsed.output.response_result, Some(json!({"ok": true})));
    }

    #[test]
    fn parse_openai_responses_streaming_response_extracts_text_deltas() {
        let request = sample_request(ModelProvider::OpenAiResponses);
        let parsed = parse_streaming_model_response(
            &request,
            concat!(
                "event: response.output_text.delta\n",
                "data: {\"type\":\"response.output_text.delta\",\"response\":{\"model\":\"gpt-4.1\"},\"delta\":\"response \"}\n\n",
                "event: response.output_text.delta\n",
                "data: {\"type\":\"response.output_text.delta\",\"delta\":\"stream\"}\n\n",
                "event: response.completed\n",
                "data: {\"type\":\"response.completed\",\"response\":{\"model\":\"gpt-4.1\",\"output\":[{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"response stream\"}]}]}}\n\n"
            ),
        )
        .expect("responses stream should parse");

        assert_eq!(parsed.output.model, "gpt-4.1");
        assert_eq!(
            parsed.output.message,
            QueryMessage::assistant("response stream")
        );
        assert_eq!(parsed.events.len(), 2);
    }

    #[test]
    fn parse_openai_responses_streaming_response_rejects_schema_mismatch() {
        let mut request = sample_request(ModelProvider::OpenAiResponses);
        request.json_schema = Some(String::from(
            "{\"type\":\"object\",\"properties\":{\"ok\":{\"type\":\"boolean\"}},\"required\":[\"ok\"]}",
        ));
        let error = parse_streaming_model_response(
            &request,
            concat!(
                "event: response.output_text.delta\n",
                "data: {\"type\":\"response.output_text.delta\",\"response\":{\"model\":\"gpt-4.1\"},\"delta\":\"{\\\"ok\\\":\\\"wrong\\\"}\"}\n\n",
                "event: response.completed\n",
                "data: {\"type\":\"response.completed\",\"response\":{\"model\":\"gpt-4.1\",\"output\":[{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"{\\\"ok\\\":\\\"wrong\\\"}\"}]}]}}\n\n"
            ),
        )
        .expect_err("schema mismatch should fail");

        assert_eq!(error.kind, ModelErrorKind::StructuredOutputFailure);
        assert!(error.message.contains("did not match schema"));
    }

    #[test]
    fn parse_openai_responses_streaming_response_surfaces_provider_payload_error() {
        let request = sample_request(ModelProvider::OpenAiResponses);
        let error = parse_streaming_model_response(
            &request,
            concat!(
                "event: error\n",
                "data: {\"error\":{\"type\":\"server_error\",\"message\":\"overloaded upstream\"}}\n\n"
            ),
        )
        .expect_err("stream payload error should fail");

        assert_eq!(error.kind, ModelErrorKind::ProviderFailure);
        assert!(error.retryable);
        assert_eq!(error.surface_label(), "server_error");
        assert_eq!(error.provider, Some(ModelProvider::OpenAiResponses));
    }
}
