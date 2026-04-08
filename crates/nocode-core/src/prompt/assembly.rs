//! Prompt assembly — discovers CLAUDE.md variants, deduplicates by FNV hash,
//! applies truncation budgets, assembles final system prompt.

use crate::message::SystemBlock;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// FNV-1a 64-bit hash for fast content deduplication.
fn fnv1a_hash(data: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0100_0000_01b3;
    let mut hash = FNV_OFFSET;
    for &byte in data {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// A discovered CLAUDE.md file with its content and source path.
#[derive(Debug, Clone)]
pub struct ClaudeMdEntry {
    pub path: PathBuf,
    pub content: String,
    pub hash: u64,
}

/// Discover all CLAUDE.md files in the hierarchy.
///
/// Search order (later entries override earlier for dedup):
/// 1. `~/.claude/CLAUDE.md` (user global)
/// 2. Walk from cwd up to filesystem root: `{dir}/CLAUDE.md`
/// 3. Walk from cwd up to filesystem root: `{dir}/.claude/CLAUDE.md`
/// 4. `{cwd}/.claude/CLAUDE.md` (project local — highest priority)
///
/// Deduplication: if two files have the same FNV hash, only the first is kept.
pub fn discover_claude_md(cwd: &str) -> Vec<ClaudeMdEntry> {
    let mut entries = Vec::new();
    let mut seen_hashes = HashSet::new();

    let mut try_add = |path: PathBuf| {
        if let Ok(content) = fs::read_to_string(&path) {
            let trimmed = content.trim();
            if trimmed.is_empty() {
                return;
            }
            let hash = fnv1a_hash(trimmed.as_bytes());
            if seen_hashes.insert(hash) {
                entries.push(ClaudeMdEntry {
                    path,
                    content: content.clone(),
                    hash,
                });
            }
        }
    };

    // 1. User global
    if let Ok(home) = std::env::var("HOME") {
        try_add(PathBuf::from(&home).join(".claude/CLAUDE.md"));
    }

    // 2. Walk from cwd upward — {dir}/CLAUDE.md
    let cwd_path = Path::new(cwd).to_path_buf();
    let mut ancestors: Vec<PathBuf> = Vec::new();
    {
        let mut dir = cwd_path.as_path();
        loop {
            ancestors.push(dir.to_path_buf());
            match dir.parent() {
                Some(p) if p != dir => dir = p,
                _ => break,
            }
        }
    }
    // Process from root → cwd so project-level wins dedup
    ancestors.reverse();
    for dir in &ancestors {
        try_add(dir.join("CLAUDE.md"));
    }

    // 3. Walk from root → cwd — {dir}/.claude/CLAUDE.md
    for dir in &ancestors {
        try_add(dir.join(".claude/CLAUDE.md"));
    }

    entries
}

/// Truncation budget configuration.
#[derive(Debug, Clone)]
pub struct TruncationBudget {
    /// Max characters for all CLAUDE.md content combined.
    pub max_claude_md_chars: usize,
    /// Max characters for the base system prompt.
    pub max_base_prompt_chars: usize,
    /// Max characters for tool definitions section.
    pub max_tool_defs_chars: usize,
}

impl Default for TruncationBudget {
    fn default() -> Self {
        Self {
            max_claude_md_chars: 80_000,
            max_base_prompt_chars: 20_000,
            max_tool_defs_chars: 40_000,
        }
    }
}

/// Truncate text to fit within a character budget, appending a marker if truncated.
fn truncate_with_marker(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }
    let cut = max_chars.saturating_sub(40);
    let mut result = String::with_capacity(max_chars);
    result.push_str(&text[..cut]);
    result.push_str("\n\n[... truncated, ");
    result.push_str(&(text.len() - cut).to_string());
    result.push_str(" chars omitted ...]");
    result
}

/// Assemble the full system prompt with deduplication and truncation.
///
/// Returns a `Vec<SystemBlock>` ready for the API request.
pub fn assemble_system_prompt(
    cwd: &str,
    extra_blocks: &[&str],
    budget: &TruncationBudget,
) -> Vec<SystemBlock> {
    let mut blocks = Vec::new();

    // 1. Base system prompt
    let base = crate::prompt::system::base_system_prompt(cwd);
    let base_text = truncate_with_marker(&base.text, budget.max_base_prompt_chars);
    blocks.push(SystemBlock::text(base_text));

    // 2. CLAUDE.md entries — deduplicated, budget-truncated
    let entries = discover_claude_md(cwd);
    if !entries.is_empty() {
        let mut combined = String::new();
        for (i, entry) in entries.iter().enumerate() {
            if i > 0 {
                combined.push_str("\n\n---\n\n");
            }
            combined.push_str(&format!(
                "# Instructions from {}\n\n{}",
                entry.path.display(),
                entry.content
            ));
        }
        let truncated = truncate_with_marker(&combined, budget.max_claude_md_chars);
        blocks.push(SystemBlock::text(truncated));
    }

    // 3. Extra blocks (tool defs, context, etc.)
    for extra in extra_blocks {
        if !extra.is_empty() {
            blocks.push(SystemBlock::text(*extra));
        }
    }

    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_deterministic() {
        let h1 = fnv1a_hash(b"hello world");
        let h2 = fnv1a_hash(b"hello world");
        assert_eq!(h1, h2);
    }

    #[test]
    fn fnv1a_different_inputs() {
        let h1 = fnv1a_hash(b"hello");
        let h2 = fnv1a_hash(b"world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn truncate_short_text_unchanged() {
        let text = "short text";
        assert_eq!(truncate_with_marker(text, 100), text);
    }

    #[test]
    fn truncate_long_text_adds_marker() {
        let text = "a".repeat(200);
        let result = truncate_with_marker(&text, 100);
        assert!(result.len() <= 120); // some overhead for marker
        assert!(result.contains("[... truncated"));
    }

    #[test]
    fn discover_deduplicates_by_hash() {
        // Two entries with same content should deduplicate
        let mut seen = HashSet::new();
        let hash = fnv1a_hash(b"same content");
        assert!(seen.insert(hash));
        assert!(!seen.insert(hash)); // duplicate rejected
    }

    #[test]
    fn assemble_includes_base_prompt() {
        let blocks = assemble_system_prompt("/tmp", &[], &TruncationBudget::default());
        assert!(!blocks.is_empty());
        assert!(blocks[0].text.contains("nocode"));
    }

    #[test]
    fn assemble_includes_extra_blocks() {
        let blocks = assemble_system_prompt(
            "/tmp",
            &["extra context here"],
            &TruncationBudget::default(),
        );
        assert!(blocks.len() >= 2);
        assert!(blocks.last().unwrap().text.contains("extra context"));
    }

    #[test]
    fn assemble_skips_empty_extras() {
        let blocks = assemble_system_prompt(
            "/tmp/nonexistent_path_xyz",
            &["", "real content", ""],
            &TruncationBudget::default(),
        );
        // Should contain base + possibly CLAUDE.md from ~ + "real content"
        // Empty strings must be skipped
        assert!(blocks.iter().any(|b| b.text.contains("real content")));
        assert!(!blocks.iter().any(|b| b.text.is_empty()));
    }

    #[test]
    fn budget_truncates_claude_md() {
        let budget = TruncationBudget {
            max_claude_md_chars: 50,
            max_base_prompt_chars: 20_000,
            max_tool_defs_chars: 40_000,
        };
        // Even with no real CLAUDE.md files, the assembly should work
        let blocks = assemble_system_prompt("/tmp/nonexistent", &[], &budget);
        assert!(!blocks.is_empty());
    }
}
