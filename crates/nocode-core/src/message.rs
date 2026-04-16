use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Message role — only User and Assistant per Claude API spec.
/// System messages are sent separately in the `system` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

impl Role {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

/// A single content block within a message.
/// Maps 1:1 to Claude API content block types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },

    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },

    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        is_error: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        structured_content: Option<Value>,
    },

    #[serde(rename = "thinking")]
    Thinking { thinking: String },

    #[serde(rename = "image")]
    Image {
        /// Base64-encoded image data.
        #[serde(rename = "source")]
        source: ImageSource,
    },
}

/// Image source for inline image content blocks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageSource {
    /// Always "base64" for inline images.
    #[serde(rename = "type")]
    pub source_type: String,
    /// MIME type: "image/png", "image/jpeg", "image/gif", "image/webp".
    pub media_type: String,
    /// Base64-encoded image bytes.
    pub data: String,
}

impl ContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    pub fn tool_use(id: impl Into<String>, name: impl Into<String>, input: Value) -> Self {
        Self::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        }
    }

    pub fn tool_result(tool_use_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self::ToolResult {
            tool_use_id: tool_use_id.into(),
            content: content.into(),
            is_error: false,
            structured_content: None,
        }
    }

    pub fn tool_error(tool_use_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self::ToolResult {
            tool_use_id: tool_use_id.into(),
            content: content.into(),
            is_error: true,
            structured_content: None,
        }
    }

    pub fn tool_result_structured(
        tool_use_id: impl Into<String>,
        content: impl Into<String>,
        structured_content: Value,
    ) -> Self {
        Self::ToolResult {
            tool_use_id: tool_use_id.into(),
            content: content.into(),
            is_error: false,
            structured_content: Some(structured_content),
        }
    }

    pub fn tool_error_structured(
        tool_use_id: impl Into<String>,
        content: impl Into<String>,
        structured_content: Value,
    ) -> Self {
        Self::ToolResult {
            tool_use_id: tool_use_id.into(),
            content: content.into(),
            is_error: true,
            structured_content: Some(structured_content),
        }
    }

    pub const fn is_tool_use(&self) -> bool {
        matches!(self, Self::ToolUse { .. })
    }

    pub const fn is_tool_result(&self) -> bool {
        matches!(self, Self::ToolResult { .. })
    }

    /// Create an inline base64 image content block.
    pub fn image(media_type: impl Into<String>, data: impl Into<String>) -> Self {
        Self::Image {
            source: ImageSource {
                source_type: "base64".to_string(),
                media_type: media_type.into(),
                data: data.into(),
            },
        }
    }
}

/// A conversation message with structured content blocks.
/// Maps 1:1 to Claude API message format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

impl Message {
    pub fn user(blocks: Vec<ContentBlock>) -> Self {
        Self {
            role: Role::User,
            content: blocks,
        }
    }

    pub fn user_text(text: impl Into<String>) -> Self {
        Self::user(vec![ContentBlock::text(text)])
    }

    pub fn assistant(blocks: Vec<ContentBlock>) -> Self {
        Self {
            role: Role::Assistant,
            content: blocks,
        }
    }

    pub fn assistant_text(text: impl Into<String>) -> Self {
        Self::assistant(vec![ContentBlock::text(text)])
    }

    /// Extract all tool_use blocks from this message.
    pub fn tool_uses(&self) -> Vec<&ContentBlock> {
        self.content.iter().filter(|b| b.is_tool_use()).collect()
    }

    /// Check if this message contains any tool_use blocks.
    pub fn has_tool_use(&self) -> bool {
        self.content.iter().any(ContentBlock::is_tool_use)
    }

    /// Get the plain text content, joining all text blocks.
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

/// Cache control directive for prompt caching.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheControl {
    #[serde(rename = "type")]
    pub control_type: String,
}

impl CacheControl {
    /// Create an ephemeral cache control (cached for the duration of the request).
    pub fn ephemeral() -> Self {
        Self {
            control_type: String::from("ephemeral"),
        }
    }
}

/// System prompt block — sent in the `system` field, not in messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

impl SystemBlock {
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            block_type: String::from("text"),
            text: content.into(),
            cache_control: None,
        }
    }

    /// Create a text system block with ephemeral cache control.
    pub fn text_cached(content: impl Into<String>) -> Self {
        Self {
            block_type: String::from("text"),
            text: content.into(),
            cache_control: Some(CacheControl::ephemeral()),
        }
    }

    /// Add cache control to this block.
    pub fn with_cache_control(mut self, cc: CacheControl) -> Self {
        self.cache_control = Some(cc);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn text_block_roundtrip() {
        let block = ContentBlock::text("hello");
        let json = serde_json::to_value(&block).unwrap();
        assert_eq!(json, json!({"type": "text", "text": "hello"}));
        let parsed: ContentBlock = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, block);
    }

    #[test]
    fn tool_use_block_roundtrip() {
        let block = ContentBlock::tool_use("id-1", "Bash", json!({"command": "ls"}));
        let json = serde_json::to_value(&block).unwrap();
        assert_eq!(json["type"], "tool_use");
        assert_eq!(json["id"], "id-1");
        assert_eq!(json["name"], "Bash");
        assert_eq!(json["input"]["command"], "ls");
        let parsed: ContentBlock = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, block);
    }

    #[test]
    fn tool_result_block_roundtrip() {
        let block = ContentBlock::tool_result("id-1", "file contents");
        let json = serde_json::to_value(&block).unwrap();
        assert_eq!(json["type"], "tool_result");
        assert_eq!(json["tool_use_id"], "id-1");
        assert!(
            json.get("is_error")
                .is_none_or(|v| v.as_bool() != Some(true))
        );
        let parsed: ContentBlock = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, block);
    }

    #[test]
    fn tool_error_block_has_is_error() {
        let block = ContentBlock::tool_error("id-1", "not found");
        let json = serde_json::to_value(&block).unwrap();
        assert_eq!(json["is_error"], true);
    }

    #[test]
    fn message_user_text() {
        let msg = Message::user_text("hello");
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.text_content(), "hello");
        assert!(!msg.has_tool_use());
    }

    #[test]
    fn message_with_tool_use() {
        let msg = Message::assistant(vec![
            ContentBlock::text("I'll run a command"),
            ContentBlock::tool_use("id-1", "Bash", json!({"command": "ls"})),
        ]);
        assert!(msg.has_tool_use());
        assert_eq!(msg.tool_uses().len(), 1);
        assert_eq!(msg.text_content(), "I'll run a command");
    }

    #[test]
    fn message_roundtrip() {
        let msg = Message::user(vec![ContentBlock::tool_result("id-1", "output")]);
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, msg);
    }

    #[test]
    fn system_block_serialization() {
        let block = SystemBlock::text("You are a coding assistant.");
        let json = serde_json::to_value(&block).unwrap();
        assert_eq!(json["type"], "text");
        assert_eq!(json["text"], "You are a coding assistant.");
        assert!(json.get("cache_control").is_none());
    }

    #[test]
    fn system_block_cached_serialization() {
        let block = SystemBlock::text_cached("You are a coding assistant.");
        let json = serde_json::to_value(&block).unwrap();
        assert_eq!(json["type"], "text");
        assert_eq!(json["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn cache_control_roundtrip() {
        let cc = CacheControl::ephemeral();
        let json = serde_json::to_value(&cc).unwrap();
        assert_eq!(json["type"], "ephemeral");
        let parsed: CacheControl = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, cc);
    }
}
