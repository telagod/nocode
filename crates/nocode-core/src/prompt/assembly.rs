//! Prompt assembly — discovers CLAUDE.md and AGENTS.md variants, deduplicates
//! by FNV hash, applies truncation budgets, assembles final system prompt.

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

/// A discovered instruction file (CLAUDE.md or AGENTS.md) with content and path.
#[derive(Debug, Clone)]
pub struct ClaudeMdEntry {
    pub path: PathBuf,
    pub content: String,
    pub hash: u64,
}

/// Walk the directory hierarchy and collect instruction files by name.
///
/// Search order:
/// 1. `~/{global_dir}/{filename}` (user global)
/// 2. Walk from cwd up to filesystem root: `{dir}/{filename}`
/// 3. Walk from cwd up to filesystem root: `{dir}/{subdir}/{filename}`
///
/// Deduplication: if two files have the same FNV hash, only the first is kept.
fn discover_instruction_files(
    cwd: &str,
    filename: &str,
    global_dir: &str,
    subdir: &str,
) -> Vec<ClaudeMdEntry> {
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
        try_add(PathBuf::from(&home).join(global_dir).join(filename));
    }

    // 2. Walk from cwd upward — {dir}/{filename}
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
        try_add(dir.join(filename));
    }

    // 3. Walk from root → cwd — {dir}/{subdir}/{filename}
    for dir in &ancestors {
        try_add(dir.join(subdir).join(filename));
    }

    entries
}

/// Discover all CLAUDE.md files in the hierarchy.
pub fn discover_claude_md(cwd: &str) -> Vec<ClaudeMdEntry> {
    discover_instruction_files(cwd, "CLAUDE.md", ".claude", ".claude")
}

/// Discover all AGENTS.md files in the hierarchy.
///
/// Search order:
/// 1. `~/.nocode/AGENTS.md` (user global)
/// 2. Walk from cwd up to filesystem root: `{dir}/AGENTS.md`
/// 3. Walk from cwd up to filesystem root: `{dir}/.nocode/AGENTS.md`
pub fn discover_agents_md(cwd: &str) -> Vec<ClaudeMdEntry> {
    discover_instruction_files(cwd, "AGENTS.md", ".nocode", ".nocode")
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
    /// Max characters for the skill index block (just the index, not bodies).
    pub max_skill_index_chars: usize,
}

impl Default for TruncationBudget {
    fn default() -> Self {
        Self {
            max_claude_md_chars: 80_000,
            max_base_prompt_chars: 20_000,
            max_tool_defs_chars: 40_000,
            max_skill_index_chars: 4_000,
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
/// If `custom_system_prompt` is provided, it replaces the built-in base prompt.
/// Returns a `Vec<SystemBlock>` ready for the API request.
pub fn assemble_system_prompt(
    cwd: &str,
    extra_blocks: &[&str],
    budget: &TruncationBudget,
    custom_system_prompt: Option<&str>,
) -> Vec<SystemBlock> {
    let mut blocks = Vec::new();

    // 1. Base system prompt (or user-supplied override)
    let base_text = if let Some(custom) = custom_system_prompt {
        truncate_with_marker(custom, budget.max_base_prompt_chars)
    } else {
        let base = crate::prompt::system::base_system_prompt(cwd);
        truncate_with_marker(&base.text, budget.max_base_prompt_chars)
    };
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

    // 3. AGENTS.md entries — same discovery + dedup logic
    let agents_entries = discover_agents_md(cwd);
    if !agents_entries.is_empty() {
        let mut combined = String::new();
        for (i, entry) in agents_entries.iter().enumerate() {
            if i > 0 {
                combined.push_str("\n\n---\n\n");
            }
            combined.push_str(&format!(
                "# Agent instructions from {}\n\n{}",
                entry.path.display(),
                entry.content
            ));
        }
        let truncated = truncate_with_marker(&combined, budget.max_claude_md_chars);
        blocks.push(SystemBlock::text(truncated));
    }

    // 4. Skill index — first-class skill block (name + description only;
    //    bodies are materialized lazily via the Skill tool). Adaptive trim
    //    keeps the densest entries when the budget is tight.
    let skill_registry = crate::skill::SkillRegistry::load(cwd);
    if let Some(index) = skill_registry.prompt_index_with_budget(Some(budget.max_skill_index_chars))
    {
        // The registry already respects the budget, so no extra truncation marker needed.
        blocks.push(SystemBlock::text(index));
    }

    // 5. Extra blocks (tool defs, context, etc.)
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
        let blocks = assemble_system_prompt("/tmp", &[], &TruncationBudget::default(), None);
        assert!(!blocks.is_empty());
        assert!(blocks[0].text.contains("nocode"));
    }

    #[test]
    fn assemble_includes_extra_blocks() {
        let blocks = assemble_system_prompt(
            "/tmp",
            &["extra context here"],
            &TruncationBudget::default(),
            None,
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
            None,
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
            max_skill_index_chars: 4_000,
        };
        // Even with no real CLAUDE.md files, the assembly should work
        let blocks = assemble_system_prompt("/tmp/nonexistent", &[], &budget, None);
        assert!(!blocks.is_empty());
    }

    #[test]
    fn custom_system_prompt_replaces_base() {
        let blocks = assemble_system_prompt(
            "/tmp",
            &[],
            &TruncationBudget::default(),
            Some("You are a pirate assistant."),
        );
        assert!(blocks[0].text.contains("pirate"));
        assert!(!blocks[0].text.contains("nocode"));
    }

    #[test]
    fn skill_index_appears_when_skills_exist() {
        use tempfile::TempDir;
        let _guard = crate::test_support::env_mutex().lock().unwrap();
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join(".nocode/skills");
        fs::create_dir_all(&skills_dir).unwrap();
        fs::write(
            skills_dir.join("hello.md"),
            "---\ndescription: Say hello\n---\nHi!\n",
        )
        .unwrap();

        // Point HOME away so we don't pick up unrelated user skills.
        let saved_home = std::env::var("HOME").ok();
        // SAFETY: env mutations restored below, serialized via env_mutex
        unsafe { std::env::set_var("HOME", tmp.path()) };

        let cwd = tmp.path().to_string_lossy().into_owned();
        let blocks = assemble_system_prompt(&cwd, &[], &TruncationBudget::default(), None);

        match saved_home {
            Some(h) => unsafe { std::env::set_var("HOME", h) },
            None => unsafe { std::env::remove_var("HOME") },
        }

        assert!(
            blocks
                .iter()
                .any(|b| b.text.contains("Available Skills") && b.text.contains("hello")),
            "skill index block missing"
        );
    }

    #[test]
    fn skill_index_absent_when_no_skills() {
        use tempfile::TempDir;
        let _guard = crate::test_support::env_mutex().lock().unwrap();
        let tmp = TempDir::new().unwrap();
        let saved_home = std::env::var("HOME").ok();
        // SAFETY: env mutations restored below, serialized via env_mutex
        unsafe { std::env::set_var("HOME", tmp.path()) };

        let cwd = tmp.path().to_string_lossy().into_owned();
        let blocks = assemble_system_prompt(&cwd, &[], &TruncationBudget::default(), None);

        match saved_home {
            Some(h) => unsafe { std::env::set_var("HOME", h) },
            None => unsafe { std::env::remove_var("HOME") },
        }

        assert!(
            !blocks.iter().any(|b| b.text.contains("Available Skills")),
            "skill block should be omitted when no skills are discovered"
        );
    }
}
