//! Session compaction — summarize conversation when context grows too large.

use crate::message::{ContentBlock, Message};

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
}
