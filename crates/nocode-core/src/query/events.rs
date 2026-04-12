//! Model stream events — high-level events for TUI/REPL consumption.

use crate::message::ContentBlock;
use crate::provider::types::Usage;
use serde_json::Value;

/// High-level events emitted during the agentic loop.
/// These are the events that TUI/REPL consumers should listen to.
#[derive(Debug, Clone)]
pub enum ModelStreamEvent {
    /// Incremental text from the model.
    TextDelta { text: String },
    /// Thinking/reasoning content from extended thinking models.
    ThinkingDelta { thinking: String },
    /// A tool call is starting.
    ToolUseStart { id: String, name: String },
    /// A tool call has finished with a summary of its input.
    ToolUseDone {
        id: String,
        name: String,
        input_summary: String,
    },
    /// A tool execution result (pushed during agentic loop).
    ToolResult {
        tool_use_id: String,
        name: String,
        content: String,
        is_error: bool,
        structured_content: Option<Value>,
    },
    /// Stream error (retryable or not).
    StreamError { message: String, retryable: bool },
    /// Turn completed (turn number).
    TurnComplete { turn: u32 },
    /// Token usage update after a model call.
    UsageUpdate { usage: Usage },
    /// The entire loop has finished.
    Complete,
}

impl ModelStreamEvent {
    pub fn tool_result_from_block(name: &str, block: &ContentBlock) -> Option<Self> {
        if let ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
            structured_content,
        } = block
        {
            Some(Self::ToolResult {
                tool_use_id: tool_use_id.clone(),
                name: name.to_string(),
                content: content.clone(),
                is_error: *is_error,
                structured_content: structured_content.clone(),
            })
        } else {
            None
        }
    }
}
