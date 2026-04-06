//! Rich structured session compaction.
//!
//! Replaces the simple [`TruncatingCompactor`](crate::query_deps::TruncatingCompactor) with a
//! compactor that generates a structured summary of removed messages — including message counts,
//! tool names, recent user requests, inferred TODOs, and key file paths.

use crate::message::{QueryMessage, QueryMessageRole};
use crate::query_deps::Compactor;
use crate::summary_compression::{SummaryCompressionBudget, compress_summary};
use std::collections::BTreeSet;

// ---------------------------------------------------------------------------
// Config & Result
// ---------------------------------------------------------------------------

/// Configuration knobs for rich session compaction.
#[derive(Debug, Clone)]
pub struct CompactionConfig {
    /// Number of recent messages to preserve verbatim (default 4).
    pub preserve_recent_messages: usize,
    /// Approximate token ceiling that triggers compaction (default 10 000).
    pub max_estimated_tokens: usize,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            preserve_recent_messages: 4,
            max_estimated_tokens: 10_000,
        }
    }
}

/// Outcome of a compaction pass.
#[derive(Debug, Clone)]
pub struct CompactionResult {
    /// The structured summary that was generated for the removed messages.
    pub summary: String,
    /// How many messages were removed (replaced by the summary).
    pub removed_message_count: usize,
    /// The final message list after compaction.
    pub compacted_messages: Vec<QueryMessage>,
}

// ---------------------------------------------------------------------------
// Core functions
// ---------------------------------------------------------------------------

/// Heuristic token estimate: `len / 4 + 1` per message.
pub fn estimate_message_tokens(messages: &[QueryMessage]) -> usize {
    messages.iter().map(|m| m.content.len() / 4 + 1).sum()
}

/// Returns `true` when the estimated token count exceeds the configured ceiling.
pub fn should_compact(messages: &[QueryMessage], config: &CompactionConfig) -> bool {
    estimate_message_tokens(messages) > config.max_estimated_tokens
}

/// Build a structured summary of the given messages.
///
/// Sections produced:
/// - Message counts by role
/// - Tool names (extracted from `tool-message:` prefixes, deduplicated & sorted)
/// - Last 3 user requests
/// - Inferred TODOs (lines containing todo/next/pending/remaining)
/// - Key files (paths ending in `.rs`, `.ts`, `.js`, `.json`, `.md`)
pub fn summarize_messages(messages: &[QueryMessage]) -> String {
    let mut system_count: usize = 0;
    let mut user_count: usize = 0;
    let mut assistant_count: usize = 0;
    let mut tool_count: usize = 0;

    let mut tool_names: BTreeSet<String> = BTreeSet::new();
    let mut user_requests: Vec<String> = Vec::new();
    let mut todos: Vec<String> = Vec::new();
    let mut key_files: BTreeSet<String> = BTreeSet::new();

    let todo_keywords = ["todo", "next", "pending", "remaining"];
    let file_extensions = [".rs", ".ts", ".js", ".json", ".md"];

    for msg in messages {
        // Count by role
        match msg.role {
            QueryMessageRole::System => system_count += 1,
            QueryMessageRole::User => {
                user_count += 1;
                user_requests.push(msg.content.clone());
            }
            QueryMessageRole::Assistant => assistant_count += 1,
            QueryMessageRole::Tool => tool_count += 1,
        }

        // Extract tool names from "tool-message: <name>" prefix
        for line in msg.content.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("tool-message:") {
                let name = rest.split_whitespace().next().unwrap_or("");
                if !name.is_empty() {
                    tool_names.insert(name.to_string());
                }
            }
        }

        // Infer TODOs
        let content_lower = msg.content.to_ascii_lowercase();
        for kw in &todo_keywords {
            if content_lower.contains(kw) {
                // Take the first line containing the keyword as the todo hint.
                for line in msg.content.lines() {
                    if line.to_ascii_lowercase().contains(kw) {
                        let snippet = line.trim();
                        if !snippet.is_empty() {
                            todos.push(snippet.to_string());
                        }
                        break;
                    }
                }
                break; // one todo entry per message
            }
        }

        // Extract key file paths
        extract_file_paths(&msg.content, &file_extensions, &mut key_files);
    }

    // --- assemble summary ---
    let mut out = String::from("[Session Compaction Summary]\n");

    // Message counts
    out.push_str(&format!(
        "Messages: {user_count} user, {assistant_count} assistant, \
         {tool_count} tool, {system_count} system\n"
    ));

    // Tools used
    if !tool_names.is_empty() {
        let names: Vec<&str> = tool_names.iter().map(String::as_str).collect();
        out.push_str(&format!("Tools used: {}\n", names.join(", ")));
    }

    // Recent user requests (last 3)
    let recent: Vec<&String> = user_requests.iter().rev().take(3).collect::<Vec<_>>();
    if !recent.is_empty() {
        out.push_str("Recent requests:\n");
        for req in recent.into_iter().rev() {
            let preview = truncate_preview(req, 120);
            out.push_str(&format!("  - {preview}\n"));
        }
    }

    // TODOs
    if !todos.is_empty() {
        out.push_str("Inferred TODOs:\n");
        for t in todos.iter().take(5) {
            let preview = truncate_preview(t, 120);
            out.push_str(&format!("  - {preview}\n"));
        }
    }

    // Key files
    if !key_files.is_empty() {
        let files: Vec<&str> = key_files.iter().map(String::as_str).collect();
        out.push_str(&format!("Key files: {}\n", files.join(", ")));
    }

    out
}

/// Run compaction: preserve recent messages, summarize the rest, insert continuation marker.
pub fn compact_session(messages: &[QueryMessage], config: &CompactionConfig) -> CompactionResult {
    if messages.is_empty() || !should_compact(messages, config) {
        return CompactionResult {
            summary: String::new(),
            removed_message_count: 0,
            compacted_messages: messages.to_vec(),
        };
    }

    let keep = config.preserve_recent_messages.min(messages.len());
    let split = messages.len().saturating_sub(keep);
    let old_messages = &messages[..split];
    let recent_messages = &messages[split..];

    let raw_summary = summarize_messages(old_messages);
    let compressed = compress_summary(&raw_summary, &SummaryCompressionBudget::default());
    let summary = compressed.summary;

    let mut compacted = Vec::with_capacity(keep + 1);
    compacted.push(QueryMessage::system(format!(
        "[Continuation] This session was compacted. Summary of {split} earlier messages:\n\n{summary}"
    )));
    compacted.extend_from_slice(recent_messages);

    CompactionResult {
        summary,
        removed_message_count: split,
        compacted_messages: compacted,
    }
}

// ---------------------------------------------------------------------------
// RichCompactor — implements Compactor trait
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct RichCompactor {
    pub config: CompactionConfig,
}

impl RichCompactor {
    pub fn new(config: CompactionConfig) -> Self {
        Self { config }
    }
}

impl Compactor for RichCompactor {
    fn compact(&self, messages: &[QueryMessage]) -> Vec<QueryMessage> {
        compact_session(messages, &self.config).compacted_messages
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract file paths from content that end with one of the given extensions.
fn extract_file_paths(content: &str, extensions: &[&str], out: &mut BTreeSet<String>) {
    for word in content.split_whitespace() {
        // Strip common surrounding punctuation
        let cleaned = word.trim_matches(|c: char| {
            c == '('
                || c == ')'
                || c == '['
                || c == ']'
                || c == '`'
                || c == '\''
                || c == '"'
                || c == ','
                || c == ';'
                || c == ':'
        });
        if cleaned.is_empty() {
            continue;
        }
        for ext in extensions {
            if cleaned.ends_with(ext) && cleaned.len() > ext.len() {
                out.insert(cleaned.to_string());
                break;
            }
        }
    }
}

/// Truncate a string to at most `max_len` characters, appending "..." if truncated.
fn truncate_preview(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let mut end = max_len;
        while !s.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::QueryMessage;

    #[test]
    fn token_estimation() {
        let msgs = vec![
            QueryMessage::user("hello world"),          // 11 / 4 + 1 = 3
            QueryMessage::assistant("hi there friend"), // 15 / 4 + 1 = 4
        ];
        assert_eq!(estimate_message_tokens(&msgs), 7);
    }

    #[test]
    fn should_compact_threshold() {
        let config = CompactionConfig {
            preserve_recent_messages: 2,
            max_estimated_tokens: 5,
        };
        let small = vec![QueryMessage::user("hi")]; // 2/4+1 = 1
        assert!(!should_compact(&small, &config));

        let big = vec![
            QueryMessage::user("a]".repeat(20)), // 40/4+1 = 11
        ];
        assert!(should_compact(&big, &config));
    }

    #[test]
    fn summarize_extracts_tools_and_files() {
        let msgs = vec![
            QueryMessage::assistant("tool-message: Read src/main.rs content"),
            QueryMessage::assistant("tool-message: Grep found matches"),
            QueryMessage::assistant("tool-message: Read another read"),
            QueryMessage::user("please fix src/lib.rs and config.json"),
            QueryMessage::tool("edited src/query.ts successfully"),
        ];
        let summary = summarize_messages(&msgs);

        // Tool names extracted and deduplicated
        assert!(summary.contains("Grep"));
        assert!(summary.contains("Read"));

        // Key files
        assert!(summary.contains("src/main.rs"));
        assert!(summary.contains("src/lib.rs"));
        assert!(summary.contains("config.json"));
        assert!(summary.contains("src/query.ts"));
    }

    #[test]
    fn compact_preserves_recent_messages() {
        let config = CompactionConfig {
            preserve_recent_messages: 2,
            max_estimated_tokens: 5,
        };
        let msgs = vec![
            QueryMessage::user("first request ".repeat(10)),
            QueryMessage::assistant("first response ".repeat(10)),
            QueryMessage::user("second request"),
            QueryMessage::assistant("second response"),
        ];
        let result = compact_session(&msgs, &config);

        assert_eq!(result.removed_message_count, 2);
        // Last two messages preserved verbatim
        assert_eq!(result.compacted_messages.len(), 3); // 1 continuation + 2 recent
        assert_eq!(
            result.compacted_messages[1],
            QueryMessage::user("second request")
        );
        assert_eq!(
            result.compacted_messages[2],
            QueryMessage::assistant("second response")
        );
    }

    #[test]
    fn compact_inserts_continuation_message() {
        let config = CompactionConfig {
            preserve_recent_messages: 1,
            max_estimated_tokens: 3,
        };
        let msgs = vec![
            QueryMessage::user("old message ".repeat(10)),
            QueryMessage::assistant("old reply ".repeat(10)),
            QueryMessage::user("latest"),
        ];
        let result = compact_session(&msgs, &config);

        let continuation = &result.compacted_messages[0];
        assert_eq!(continuation.role, QueryMessageRole::System);
        assert!(continuation.content.contains("[Continuation]"));
        assert!(continuation.content.contains("Session Compaction Summary"));
    }

    #[test]
    fn empty_messages_no_compact() {
        let config = CompactionConfig::default();
        let result = compact_session(&[], &config);
        assert_eq!(result.removed_message_count, 0);
        assert!(result.compacted_messages.is_empty());
        assert!(result.summary.is_empty());
    }

    #[test]
    fn summarize_infers_todos() {
        let msgs = vec![
            QueryMessage::user("TODO: fix the parser"),
            QueryMessage::assistant("next step is to refactor the loop"),
        ];
        let summary = summarize_messages(&msgs);
        assert!(summary.contains("Inferred TODOs"));
        assert!(summary.contains("TODO: fix the parser"));
    }

    #[test]
    fn rich_compactor_trait_impl() {
        let compactor = RichCompactor::new(CompactionConfig {
            preserve_recent_messages: 1,
            max_estimated_tokens: 3,
        });
        let msgs = vec![
            QueryMessage::user("old ".repeat(20)),
            QueryMessage::user("recent"),
        ];
        let result = compactor.compact(&msgs);
        // Should have continuation + 1 recent
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].role, QueryMessageRole::System);
        assert_eq!(result[1], QueryMessage::user("recent"));
    }
}
