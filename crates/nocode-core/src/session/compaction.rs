//! Session compaction — summarize conversation when context grows too large.

use crate::message::{ContentBlock, Message, SystemBlock};
use crate::provider::Provider;
use crate::provider::types::CreateMessageRequest;

/// Compaction result — a summarized conversation.
#[derive(Debug, Clone)]
pub struct CompactionResult {
    /// Summarized messages replacing the original conversation.
    pub messages: Vec<Message>,
    /// Number of original messages that were compacted.
    pub compacted_count: usize,
    /// Estimated tokens saved.
    pub tokens_saved: u64,
}

/// Trait for compaction strategies.
pub trait Compactor: Send + Sync {
    /// Compact a conversation, returning a shorter version.
    fn compact(&self, messages: &[Message]) -> CompactionResult;
}

/// Simple compactor that keeps the last N messages and summarizes the rest.
pub struct TailCompactor {
    /// Number of recent messages to keep verbatim.
    pub keep_recent: usize,
}

impl TailCompactor {
    pub fn new(keep_recent: usize) -> Self {
        Self { keep_recent }
    }
}

impl Default for TailCompactor {
    fn default() -> Self {
        Self::new(10)
    }
}

impl Compactor for TailCompactor {
    fn compact(&self, messages: &[Message]) -> CompactionResult {
        if messages.len() <= self.keep_recent {
            return CompactionResult {
                messages: messages.to_vec(),
                compacted_count: 0,
                tokens_saved: 0,
            };
        }

        let split = messages.len() - self.keep_recent;
        let old = &messages[..split];
        let recent = &messages[split..];

        // Build a summary of the compacted messages
        let mut summary_parts: Vec<String> = Vec::new();
        for msg in old {
            for block in &msg.content {
                if let ContentBlock::Text { text } = block {
                    let truncated = if text.len() > 100 {
                        format!("{}...", &text[..100])
                    } else {
                        text.clone()
                    };
                    summary_parts.push(format!("[{:?}] {truncated}", msg.role));
                }
            }
        }

        let summary_text = format!(
            "[Compacted {} earlier messages]\n{}",
            old.len(),
            summary_parts.join("\n")
        );

        let estimated_old_tokens: u64 = old
            .iter()
            .flat_map(|m| &m.content)
            .map(|b| match b {
                ContentBlock::Text { text } => text.len() as u64 / 4,
                _ => 50,
            })
            .sum();

        let summary_tokens = summary_text.len() as u64 / 4;

        let mut result_messages = vec![Message::user_text(&summary_text)];
        result_messages.extend_from_slice(recent);

        CompactionResult {
            messages: result_messages,
            compacted_count: old.len(),
            tokens_saved: estimated_old_tokens.saturating_sub(summary_tokens),
        }
    }
}

/// Rich compactor that uses the model to produce structured summaries.
/// Falls back to TailCompactor if the model call fails.
pub struct RichCompactor {
    provider: Box<dyn Provider>,
    model: String,
    keep_recent: usize,
}

impl RichCompactor {
    pub fn new(provider: Box<dyn Provider>, model: &str, keep_recent: usize) -> Self {
        Self {
            provider,
            model: model.to_string(),
            keep_recent,
        }
    }

    fn build_summary_prompt(messages: &[Message]) -> String {
        let mut transcript = String::new();
        for msg in messages {
            let role = format!("{:?}", msg.role);
            for block in &msg.content {
                match block {
                    ContentBlock::Text { text } => {
                        let truncated = if text.len() > 500 {
                            format!("{}...", &text[..500])
                        } else {
                            text.clone()
                        };
                        transcript.push_str(&format!("[{role}] {truncated}\n"));
                    }
                    ContentBlock::ToolUse { name, .. } => {
                        transcript.push_str(&format!("[{role}] (tool_use: {name})\n"));
                    }
                    ContentBlock::ToolResult {
                        content, is_error, ..
                    } => {
                        let status = if *is_error { "error" } else { "ok" };
                        let preview = if content.len() > 200 {
                            format!("{}...", &content[..200])
                        } else {
                            content.clone()
                        };
                        transcript.push_str(&format!("[tool_result:{status}] {preview}\n"));
                    }
                    ContentBlock::Thinking { .. } => {}
                    ContentBlock::Image { source } => {
                        transcript.push_str(&format!("[{role}] (image: {})\n", source.media_type));
                    }
                }
            }
        }
        transcript
    }

    fn estimate_tokens(messages: &[Message]) -> u64 {
        messages
            .iter()
            .flat_map(|m| &m.content)
            .map(|b| match b {
                ContentBlock::Text { text } => text.len() as u64 / 4,
                ContentBlock::ToolUse { input, .. } => input.to_string().len() as u64 / 4,
                ContentBlock::ToolResult { content, .. } => content.len() as u64 / 4,
                ContentBlock::Thinking { thinking } => thinking.len() as u64 / 4,
                ContentBlock::Image { source } => source.data.len() as u64 / 4,
            })
            .sum()
    }
}

impl Compactor for RichCompactor {
    fn compact(&self, messages: &[Message]) -> CompactionResult {
        if messages.len() <= self.keep_recent {
            return CompactionResult {
                messages: messages.to_vec(),
                compacted_count: 0,
                tokens_saved: 0,
            };
        }

        let split = messages.len() - self.keep_recent;
        let old = &messages[..split];
        let recent = &messages[split..];

        let transcript = Self::build_summary_prompt(old);
        let estimated_old_tokens = Self::estimate_tokens(old);

        // Ask the model to summarize
        let summary_request = CreateMessageRequest {
            model: self.model.clone(),
            max_tokens: 1024,
            system: vec![SystemBlock {
                block_type: "text".to_string(),
                text: "You are a conversation summarizer. Produce a structured summary of the conversation transcript below. Include: key decisions made, files modified, tools used, current state of work, and any unresolved issues. Be concise but preserve all actionable context.".to_string(),
                cache_control: None,
            }],
            messages: vec![Message::user_text(format!(
                "Summarize this conversation transcript:\n\n{transcript}"
            ))],
            tools: vec![],
            stream: false,
            thinking: None,
            response_format: None,
        };

        let summary_text = match self.provider.create_message(&summary_request) {
            Ok(response) => {
                let text = response.text_content();
                if text.is_empty() {
                    // Fallback to simple summary
                    format!(
                        "[Compacted {} earlier messages — model returned empty summary]",
                        old.len()
                    )
                } else {
                    format!(
                        "[Structured summary of {} earlier messages]\n\n{text}",
                        old.len()
                    )
                }
            }
            Err(_) => {
                // Fallback: use TailCompactor logic
                let fallback = TailCompactor::new(self.keep_recent);
                return fallback.compact(messages);
            }
        };

        let summary_tokens = summary_text.len() as u64 / 4;

        let mut result_messages = vec![Message::user_text(&summary_text)];
        result_messages.extend_from_slice(recent);

        CompactionResult {
            messages: result_messages,
            compacted_count: old.len(),
            tokens_saved: estimated_old_tokens.saturating_sub(summary_tokens),
        }
    }
}

// ---------------------------------------------------------------------------
// ContextCollapser — automatic context collapse controller
// ---------------------------------------------------------------------------

/// Configuration for automatic context collapse.
#[derive(Debug, Clone)]
pub struct CollapseConfig {
    /// Maximum number of messages before triggering collapse.
    pub max_messages: usize,
    /// Maximum estimated tokens before triggering collapse.
    pub max_tokens: u64,
    /// Number of recent messages to always keep intact.
    pub keep_recent: usize,
    /// Whether collapse is enabled.
    pub enabled: bool,
}

impl Default for CollapseConfig {
    fn default() -> Self {
        Self {
            max_messages: 50,
            max_tokens: 100_000,
            keep_recent: 15,
            enabled: true,
        }
    }
}

/// Automatic context collapse controller.
/// Monitors conversation size and triggers compaction when thresholds are exceeded.
pub struct ContextCollapser {
    config: CollapseConfig,
    compactor: Box<dyn Compactor>,
    /// Number of times collapse has been triggered.
    pub collapse_count: u32,
    /// Total tokens saved across all collapses.
    pub total_tokens_saved: u64,
}

impl ContextCollapser {
    pub fn new(config: CollapseConfig, compactor: Box<dyn Compactor>) -> Self {
        Self {
            config,
            compactor,
            collapse_count: 0,
            total_tokens_saved: 0,
        }
    }

    /// Create with default config and TailCompactor.
    pub fn with_defaults() -> Self {
        let config = CollapseConfig::default();
        let keep = config.keep_recent;
        Self::new(config, Box::new(TailCompactor::new(keep)))
    }

    /// Estimate token count for a message list.
    fn estimate_tokens(messages: &[Message]) -> u64 {
        messages
            .iter()
            .flat_map(|m| &m.content)
            .map(|b| match b {
                ContentBlock::Text { text } => text.len() as u64 / 4,
                ContentBlock::ToolUse { input, .. } => input.to_string().len() as u64 / 4,
                ContentBlock::ToolResult { content, .. } => content.len() as u64 / 4,
                ContentBlock::Thinking { thinking } => thinking.len() as u64 / 4,
                ContentBlock::Image { source } => source.data.len() as u64 / 4,
            })
            .sum()
    }

    /// Check if collapse should be triggered.
    pub fn should_collapse(&self, messages: &[Message]) -> bool {
        if !self.config.enabled {
            return false;
        }
        if messages.len() > self.config.max_messages {
            return true;
        }
        Self::estimate_tokens(messages) > self.config.max_tokens
    }

    /// Attempt to collapse the conversation if thresholds are exceeded.
    /// Returns Some(result) if collapse happened, None if not needed.
    pub fn maybe_collapse(&mut self, messages: &[Message]) -> Option<CompactionResult> {
        if !self.should_collapse(messages) {
            return None;
        }
        let result = self.compactor.compact(messages);
        if result.compacted_count > 0 {
            self.collapse_count += 1;
            self.total_tokens_saved += result.tokens_saved;
            Some(result)
        } else {
            None
        }
    }

    /// Get collapse statistics.
    pub fn stats(&self) -> (u32, u64) {
        (self.collapse_count, self.total_tokens_saved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_compaction_when_under_limit() {
        let msgs = vec![Message::user_text("hello"), Message::user_text("world")];
        let c = TailCompactor::new(5);
        let result = c.compact(&msgs);
        assert_eq!(result.compacted_count, 0);
        assert_eq!(result.messages.len(), 2);
    }

    #[test]
    fn compacts_old_messages() {
        let msgs: Vec<Message> = (0..20)
            .map(|i| {
                Message::user_text(format!(
                    "This is a longer message number {i} with enough content to have meaningful token count for compaction testing purposes."
                ))
            })
            .collect();
        let c = TailCompactor::new(5);
        let result = c.compact(&msgs);
        assert_eq!(result.compacted_count, 15);
        assert_eq!(result.messages.len(), 6); // 1 summary + 5 recent
    }

    #[test]
    fn summary_contains_compacted_info() {
        let msgs = vec![
            Message::user_text("first message"),
            Message::user_text("second message"),
            Message::user_text("third message"),
        ];
        let c = TailCompactor::new(1);
        let result = c.compact(&msgs);
        assert_eq!(result.messages.len(), 2);
        // First message should be the summary
        if let Some(ContentBlock::Text { text }) = result.messages[0].content.first() {
            assert!(text.contains("Compacted 2"));
        } else {
            panic!("Expected text summary");
        }
    }

    #[test]
    fn rich_compactor_builds_summary_prompt() {
        let msgs = vec![
            Message::user_text("hello"),
            Message::assistant_text("hi there"),
        ];
        let prompt = RichCompactor::build_summary_prompt(&msgs);
        assert!(prompt.contains("[User]"));
        assert!(prompt.contains("[Assistant]"));
        assert!(prompt.contains("hello"));
        assert!(prompt.contains("hi there"));
    }

    #[test]
    fn rich_compactor_includes_tool_use_in_prompt() {
        let msgs = vec![Message::assistant(vec![ContentBlock::ToolUse {
            id: "t1".to_string(),
            name: "Bash".to_string(),
            input: serde_json::json!({"command": "ls"}),
        }])];
        let prompt = RichCompactor::build_summary_prompt(&msgs);
        assert!(prompt.contains("tool_use: Bash"));
    }

    #[test]
    fn estimate_tokens_reasonable() {
        let msgs = vec![Message::user_text("hello world this is a test message")];
        let tokens = RichCompactor::estimate_tokens(&msgs);
        // ~34 chars / 4 ≈ 8 tokens
        assert!(tokens > 0);
        assert!(tokens < 100);
    }

    // --- ContextCollapser ---
    #[test]
    fn collapser_no_collapse_under_threshold() {
        let mut c = ContextCollapser::with_defaults();
        let msgs: Vec<Message> = (0..5)
            .map(|i| Message::user_text(format!("msg {i}")))
            .collect();
        assert!(!c.should_collapse(&msgs));
        assert!(c.maybe_collapse(&msgs).is_none());
    }

    #[test]
    fn collapser_triggers_on_message_count() {
        let config = CollapseConfig {
            max_messages: 10,
            max_tokens: 1_000_000,
            keep_recent: 5,
            enabled: true,
        };
        let mut c = ContextCollapser::new(config, Box::new(TailCompactor::new(5)));
        let msgs: Vec<Message> = (0..20)
            .map(|i| Message::user_text(format!("message number {i} with some content")))
            .collect();
        assert!(c.should_collapse(&msgs));
        let result = c.maybe_collapse(&msgs).unwrap();
        assert!(result.compacted_count > 0);
        assert_eq!(c.collapse_count, 1);
    }

    #[test]
    fn collapser_triggers_on_token_count() {
        let config = CollapseConfig {
            max_messages: 1000,
            max_tokens: 100, // very low threshold
            keep_recent: 2,
            enabled: true,
        };
        let mut c = ContextCollapser::new(config, Box::new(TailCompactor::new(2)));
        let msgs: Vec<Message> = (0..10)
            .map(|i| {
                Message::user_text(format!(
                    "message {i} with enough text to exceed the very low token threshold we set"
                ))
            })
            .collect();
        assert!(c.should_collapse(&msgs));
        let result = c.maybe_collapse(&msgs).unwrap();
        assert!(result.compacted_count > 0);
    }

    #[test]
    fn collapser_disabled_never_triggers() {
        let config = CollapseConfig {
            max_messages: 1,
            max_tokens: 1,
            keep_recent: 1,
            enabled: false,
        };
        let mut c = ContextCollapser::new(config, Box::new(TailCompactor::new(1)));
        let msgs: Vec<Message> = (0..100)
            .map(|i| Message::user_text(format!("msg {i}")))
            .collect();
        assert!(!c.should_collapse(&msgs));
        assert!(c.maybe_collapse(&msgs).is_none());
    }

    #[test]
    fn collapser_stats_accumulate() {
        let config = CollapseConfig {
            max_messages: 5,
            max_tokens: 1_000_000,
            keep_recent: 3,
            enabled: true,
        };
        let mut c = ContextCollapser::new(config, Box::new(TailCompactor::new(3)));
        let msgs: Vec<Message> = (0..10)
            .map(|i| Message::user_text(format!(
                "This is a substantially longer message number {i} that contains enough text content to ensure the compaction summary is shorter than the original messages combined, which is necessary for tokens_saved to be positive in this test case."
            )))
            .collect();
        c.maybe_collapse(&msgs);
        c.maybe_collapse(&msgs);
        let (count, _saved) = c.stats();
        assert_eq!(count, 2);
    }
}
