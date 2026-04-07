use crate::message::{ContentBlock, Message, SystemBlock};
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
    pub input_tokens: u64,
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

/// Error from a provider call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderError {
    pub message: String,
    pub retryable: bool,
}

impl ProviderError {
    pub fn new(message: impl Into<String>, retryable: bool) -> Self {
        Self {
            message: message.into(),
            retryable,
        }
    }

    pub fn non_retryable(message: impl Into<String>) -> Self {
        Self::new(message, false)
    }

    pub fn retryable(message: impl Into<String>) -> Self {
        Self::new(message, true)
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
}
