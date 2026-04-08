use crate::message::{CacheControl, ContentBlock, Message, SystemBlock};
use serde::{Deserialize, Serialize};

/// Which provider to use for model calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelProvider {
    Claude,
    OpenAi,
    Gemini,
    Custom,
}

impl ModelProvider {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::OpenAi => "openai",
            Self::Gemini => "gemini",
            Self::Custom => "custom",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "claude" | "anthropic" => Some(Self::Claude),
            "openai" => Some(Self::OpenAi),
            "gemini" | "google" => Some(Self::Gemini),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }
}

/// Tool definition sent to the API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

/// Thinking mode configuration for extended thinking models.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThinkingConfig {
    /// Type of thinking: "enabled" or "disabled".
    #[serde(rename = "type")]
    pub thinking_type: String,
    /// Token budget for thinking (required when enabled).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_tokens: Option<u32>,
}

impl ThinkingConfig {
    pub fn enabled(budget_tokens: u32) -> Self {
        Self {
            thinking_type: "enabled".to_string(),
            budget_tokens: Some(budget_tokens),
        }
    }

    pub fn disabled() -> Self {
        Self {
            thinking_type: "disabled".to_string(),
            budget_tokens: None,
        }
    }
}

/// Response format constraint for structured output.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum ResponseFormat {
    /// Plain text (default).
    #[serde(rename = "text")]
    Text,
    /// JSON output constrained by a schema.
    #[serde(rename = "json_schema")]
    JsonSchema {
        /// Schema name for identification.
        name: String,
        /// The JSON Schema to constrain the response.
        schema: serde_json::Value,
        /// Whether to strictly enforce the schema (default: true).
        #[serde(default = "default_strict")]
        strict: bool,
    },
    /// Raw JSON mode (no schema, just valid JSON).
    #[serde(rename = "json_object")]
    JsonObject,
}

fn default_strict() -> bool {
    true
}

impl ResponseFormat {
    /// Create a JSON schema response format.
    pub fn json_schema(name: impl Into<String>, schema: serde_json::Value) -> Self {
        Self::JsonSchema {
            name: name.into(),
            schema,
            strict: true,
        }
    }

    /// Create a raw JSON object response format.
    pub fn json_object() -> Self {
        Self::JsonObject
    }
}

/// Request to create a message (model call).
#[derive(Debug, Clone, Serialize)]
pub struct CreateMessageRequest {
    pub model: String,
    pub max_tokens: u32,
    pub system: Vec<SystemBlock>,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDefinition>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
}

/// Why the model stopped generating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    PauseTurn,
}

/// Token usage from a model call.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
}

/// Response from a model call.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CreateMessageResponse {
    pub id: String,
    #[serde(default)]
    pub content: Vec<ContentBlock>,
    pub stop_reason: StopReason,
    pub usage: Usage,
    pub model: String,
}

impl CreateMessageResponse {
    /// Extract all tool_use blocks from the response content.
    pub fn tool_uses(&self) -> Vec<&ContentBlock> {
        self.content.iter().filter(|b| b.is_tool_use()).collect()
    }

    /// Check if the model wants to use tools.
    pub fn has_tool_use(&self) -> bool {
        self.stop_reason == StopReason::ToolUse
    }

    /// Get the plain text content from the response.
    pub fn text_content(&self) -> String {
        self.content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }
}

/// SSE streaming events from the Claude Messages API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    /// A new content block is starting.
    ContentBlockStart {
        index: u32,
        content_block: ContentBlock,
    },
    /// Incremental delta for a content block.
    ContentBlockDelta { index: u32, delta: StreamDelta },
    /// A content block has finished.
    ContentBlockStop { index: u32 },
    /// Final message-level metadata (stop_reason, usage).
    MessageDelta {
        stop_reason: StopReason,
        usage: Usage,
    },
    /// Message is complete.
    MessageStop,
    /// Ping / keepalive.
    Ping,
}

/// Delta types within a streaming content block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StreamDelta {
    #[serde(rename = "text_delta")]
    TextDelta { text: String },
    #[serde(rename = "input_json_delta")]
    InputJsonDelta { partial_json: String },
    #[serde(rename = "thinking_delta")]
    ThinkingDelta { thinking: String },
}

/// Fine-grained error classification for provider errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Authentication failure (invalid/expired API key). HTTP 401/403.
    Auth,
    /// Rate limited. HTTP 429.
    RateLimit,
    /// Quota exceeded (billing/usage limit). HTTP 402 or specific error codes.
    Quota,
    /// Request timeout or stalled stream.
    Timeout,
    /// Server-side error. HTTP 5xx.
    ServerError,
    /// Invalid request (bad params, too large, etc). HTTP 400.
    InvalidRequest,
    /// Network/connection error.
    NetworkError,
    /// Response parsing failure (malformed JSON, unexpected format).
    ParseError,
    /// Model overloaded (Claude 529). Retryable with longer backoff.
    Overloaded,
    /// Unknown/unclassified error.
    Unknown,
}

impl ErrorKind {
    /// Whether this error kind is retryable.
    pub fn is_retryable(self) -> bool {
        matches!(
            self,
            Self::RateLimit
                | Self::Timeout
                | Self::ServerError
                | Self::NetworkError
                | Self::Overloaded
        )
    }

    /// Classify from HTTP status code.
    pub fn from_status(status: u16) -> Self {
        match status {
            400 => Self::InvalidRequest,
            401 | 403 => Self::Auth,
            402 => Self::Quota,
            429 => Self::RateLimit,
            529 => Self::Overloaded,
            408 | 504 => Self::Timeout,
            s if s >= 500 => Self::ServerError,
            _ => Self::Unknown,
        }
    }
}

/// Error from a provider call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderError {
    pub message: String,
    pub retryable: bool,
    pub status_code: Option<u16>,
    pub kind: ErrorKind,
}

impl ProviderError {
    pub fn new(message: impl Into<String>, retryable: bool) -> Self {
        Self {
            message: message.into(),
            retryable,
            status_code: None,
            kind: ErrorKind::Unknown,
        }
    }

    pub fn with_status(message: impl Into<String>, retryable: bool, status: u16) -> Self {
        let kind = ErrorKind::from_status(status);
        Self {
            message: message.into(),
            retryable: retryable || kind.is_retryable(),
            status_code: Some(status),
            kind,
        }
    }

    pub fn with_kind(message: impl Into<String>, kind: ErrorKind) -> Self {
        Self {
            message: message.into(),
            retryable: kind.is_retryable(),
            status_code: None,
            kind,
        }
    }

    pub fn non_retryable(message: impl Into<String>) -> Self {
        Self::new(message, false)
    }

    pub fn retryable(message: impl Into<String>) -> Self {
        Self::new(message, true)
    }

    pub fn parse_error(message: impl Into<String>) -> Self {
        Self::with_kind(message, ErrorKind::ParseError)
    }

    pub fn network_error(message: impl Into<String>) -> Self {
        Self::with_kind(message, ErrorKind::NetworkError)
    }

    pub fn timeout(message: impl Into<String>) -> Self {
        Self::with_kind(message, ErrorKind::Timeout)
    }
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ProviderError {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stop_reason_serde() {
        let sr: StopReason = serde_json::from_value(json!("end_turn")).unwrap();
        assert_eq!(sr, StopReason::EndTurn);
        let sr: StopReason = serde_json::from_value(json!("tool_use")).unwrap();
        assert_eq!(sr, StopReason::ToolUse);
    }

    #[test]
    fn usage_defaults() {
        let u: Usage = serde_json::from_value(json!({
            "input_tokens": 100,
            "output_tokens": 50
        }))
        .unwrap();
        assert_eq!(u.input_tokens, 100);
        assert_eq!(u.cache_creation_input_tokens, 0);
    }

    #[test]
    fn provider_parse() {
        assert_eq!(ModelProvider::parse("claude"), Some(ModelProvider::Claude));
        assert_eq!(
            ModelProvider::parse("anthropic"),
            Some(ModelProvider::Claude)
        );
        assert_eq!(ModelProvider::parse("openai"), Some(ModelProvider::OpenAi));
        assert_eq!(ModelProvider::parse("google"), Some(ModelProvider::Gemini));
        assert_eq!(ModelProvider::parse("unknown"), None);
    }

    #[test]
    fn response_text_extraction() {
        let resp = CreateMessageResponse {
            id: String::from("msg-1"),
            content: vec![ContentBlock::text("Hello "), ContentBlock::text("world")],
            stop_reason: StopReason::EndTurn,
            usage: Usage::default(),
            model: String::from("claude-opus-4-20250514"),
        };
        assert_eq!(resp.text_content(), "Hello world");
        assert!(!resp.has_tool_use());
    }

    #[test]
    fn error_kind_from_status() {
        assert_eq!(ErrorKind::from_status(400), ErrorKind::InvalidRequest);
        assert_eq!(ErrorKind::from_status(401), ErrorKind::Auth);
        assert_eq!(ErrorKind::from_status(403), ErrorKind::Auth);
        assert_eq!(ErrorKind::from_status(402), ErrorKind::Quota);
        assert_eq!(ErrorKind::from_status(429), ErrorKind::RateLimit);
        assert_eq!(ErrorKind::from_status(500), ErrorKind::ServerError);
        assert_eq!(ErrorKind::from_status(502), ErrorKind::ServerError);
        assert_eq!(ErrorKind::from_status(503), ErrorKind::ServerError);
        assert_eq!(ErrorKind::from_status(529), ErrorKind::Overloaded);
        assert_eq!(ErrorKind::from_status(408), ErrorKind::Timeout);
        assert_eq!(ErrorKind::from_status(504), ErrorKind::Timeout);
        assert_eq!(ErrorKind::from_status(418), ErrorKind::Unknown);
    }

    #[test]
    fn error_kind_retryable() {
        assert!(ErrorKind::RateLimit.is_retryable());
        assert!(ErrorKind::Timeout.is_retryable());
        assert!(ErrorKind::ServerError.is_retryable());
        assert!(ErrorKind::NetworkError.is_retryable());
        assert!(ErrorKind::Overloaded.is_retryable());
        assert!(!ErrorKind::Auth.is_retryable());
        assert!(!ErrorKind::InvalidRequest.is_retryable());
        assert!(!ErrorKind::Quota.is_retryable());
        assert!(!ErrorKind::ParseError.is_retryable());
    }

    #[test]
    fn provider_error_with_status_sets_kind() {
        let e = ProviderError::with_status("rate limited", false, 429);
        assert_eq!(e.kind, ErrorKind::RateLimit);
        assert!(e.retryable); // overridden by kind

        let e = ProviderError::with_status("bad request", false, 400);
        assert_eq!(e.kind, ErrorKind::InvalidRequest);
        assert!(!e.retryable);

        let e = ProviderError::with_status("overloaded", false, 529);
        assert_eq!(e.kind, ErrorKind::Overloaded);
        assert!(e.retryable);
    }

    #[test]
    fn provider_error_convenience_constructors() {
        let e = ProviderError::parse_error("bad json");
        assert_eq!(e.kind, ErrorKind::ParseError);
        assert!(!e.retryable);

        let e = ProviderError::network_error("connection refused");
        assert_eq!(e.kind, ErrorKind::NetworkError);
        assert!(e.retryable);

        let e = ProviderError::timeout("timed out");
        assert_eq!(e.kind, ErrorKind::Timeout);
        assert!(e.retryable);
    }

    #[test]
    fn thinking_config_enabled_serialization() {
        let tc = ThinkingConfig::enabled(10000);
        let json = serde_json::to_value(&tc).unwrap();
        assert_eq!(json["type"], "enabled");
        assert_eq!(json["budget_tokens"], 10000);
    }

    #[test]
    fn thinking_config_disabled_serialization() {
        let tc = ThinkingConfig::disabled();
        let json = serde_json::to_value(&tc).unwrap();
        assert_eq!(json["type"], "disabled");
        assert!(json.get("budget_tokens").is_none());
    }

    #[test]
    fn request_without_thinking_omits_field() {
        let req = CreateMessageRequest {
            model: "test".to_string(),
            max_tokens: 1024,
            system: vec![],
            messages: vec![],
            tools: vec![],
            stream: false,
            thinking: None,
            response_format: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert!(json.get("thinking").is_none());
    }

    #[test]
    fn request_with_thinking_includes_field() {
        let req = CreateMessageRequest {
            model: "test".to_string(),
            max_tokens: 1024,
            system: vec![],
            messages: vec![],
            tools: vec![],
            stream: false,
            thinking: Some(ThinkingConfig::enabled(8192)),
            response_format: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["thinking"]["type"], "enabled");
        assert_eq!(json["thinking"]["budget_tokens"], 8192);
    }

    #[test]
    fn response_format_json_schema_serialization() {
        let rf = ResponseFormat::json_schema(
            "my_schema",
            serde_json::json!({"type": "object", "properties": {"name": {"type": "string"}}}),
        );
        let json = serde_json::to_value(&rf).unwrap();
        assert_eq!(json["type"], "json_schema");
        assert_eq!(json["name"], "my_schema");
        assert_eq!(json["strict"], true);
        assert!(json.get("schema").is_some());
    }

    #[test]
    fn response_format_json_object_serialization() {
        let rf = ResponseFormat::json_object();
        let json = serde_json::to_value(&rf).unwrap();
        assert_eq!(json["type"], "json_object");
    }

    #[test]
    fn request_without_response_format_omits_field() {
        let req = CreateMessageRequest {
            model: "test".to_string(),
            max_tokens: 1024,
            system: vec![],
            messages: vec![],
            tools: vec![],
            stream: false,
            thinking: None,
            response_format: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert!(json.get("response_format").is_none());
    }

    #[test]
    fn request_with_response_format_includes_field() {
        let req = CreateMessageRequest {
            model: "test".to_string(),
            max_tokens: 1024,
            system: vec![],
            messages: vec![],
            tools: vec![],
            stream: false,
            thinking: None,
            response_format: Some(ResponseFormat::json_schema(
                "test",
                serde_json::json!({"type": "object"}),
            )),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["response_format"]["type"], "json_schema");
    }
}
