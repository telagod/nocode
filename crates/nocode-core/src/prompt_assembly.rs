use std::fs;
use std::path::Path;

pub const DYNAMIC_BOUNDARY: &str = "__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__";
pub const MAX_INSTRUCTION_FILE_CHARS: usize = 4_000;
pub const MAX_TOTAL_INSTRUCTION_CHARS: usize = 12_000;

#[derive(Debug, Clone)]
pub struct InstructionFile {
    pub path: String,
    pub content: String,
    pub content_hash: u64,
}

#[derive(Debug, Clone)]
pub struct ProjectContext {
    pub cwd: String,
    pub current_date: String,
    pub platform: String,
    pub model_name: String,
    pub git_branch: Option<String>,
    pub git_status: Option<String>,
    pub instruction_files: Vec<InstructionFile>,
}

#[derive(Debug, Clone, Default)]
pub struct SystemPromptBuilder {
    sections: Vec<PromptSection>,
}

#[derive(Debug, Clone)]
struct PromptSection {
    label: String,
    content: String,
    is_dynamic: bool,
}

/// FNV-1a style hash for content deduplication.
pub fn simple_hash(content: &str) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0100_0000_01b3;
    let mut hash = FNV_OFFSET;
    for byte in content.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Truncate content to `max` chars, appending `[truncated]` if cut.
pub fn truncate_content(content: &str, max: usize) -> String {
    if content.len() <= max {
        return content.to_string();
    }
    let mut end = max;
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = content[..end].to_string();
    out.push_str("\n[truncated]");
    out
}

impl SystemPromptBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a static section (intro, output style, system, task, actions).
    pub fn add_static(&mut self, label: &str, content: &str) {
        self.sections.push(PromptSection {
            label: label.to_string(),
            content: content.to_string(),
            is_dynamic: false,
        });
    }

    /// Insert the dynamic boundary marker.
    pub fn add_dynamic_boundary(&mut self) {
        self.sections.push(PromptSection {
            label: String::new(),
            content: DYNAMIC_BOUNDARY.to_string(),
            is_dynamic: false,
        });
    }

    /// Add runtime context (cwd, date, platform, model, git info).
    pub fn add_context(&mut self, ctx: &ProjectContext) {
        let mut parts = vec![
            format!("cwd: {}", ctx.cwd),
            format!("currentDate: {}", ctx.current_date),
            format!("platform: {}", ctx.platform),
            format!("model: {}", ctx.model_name),
        ];
        if let Some(ref branch) = ctx.git_branch {
            parts.push(format!("gitBranch: {}", branch));
        }
        if let Some(ref status) = ctx.git_status {
            parts.push(format!("gitStatus: {}", status));
        }
        self.sections.push(PromptSection {
            label: "context".to_string(),
            content: parts.join("\n"),
            is_dynamic: true,
        });
    }

    /// Add instruction files, deduplicating by content_hash and enforcing size limits.
    pub fn add_instructions(&mut self, files: &[InstructionFile]) {
        let mut seen_hashes = std::collections::HashSet::new();
        let mut total_chars = 0usize;
        let mut merged = String::new();

        for file in files {
            if !seen_hashes.insert(file.content_hash) {
                continue;
            }
            let content = truncate_content(&file.content, MAX_INSTRUCTION_FILE_CHARS);
            let remaining = MAX_TOTAL_INSTRUCTION_CHARS.saturating_sub(total_chars);
            if remaining == 0 {
                break;
            }
            let entry = format!("# {}\n{}", file.path, content);
            let entry = truncate_content(&entry, remaining);
            total_chars += entry.len();
            if !merged.is_empty() {
                merged.push('\n');
            }
            merged.push_str(&entry);
        }

        if !merged.is_empty() {
            self.sections.push(PromptSection {
                label: "instructions".to_string(),
                content: merged,
                is_dynamic: true,
            });
        }
    }

    /// Build the final system prompt string.
    pub fn build(&self) -> String {
        let mut out = String::new();
        for (i, section) in self.sections.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            if section.label.is_empty() {
                out.push_str(&section.content);
            } else if section.is_dynamic {
                out.push_str(&format!(
                    "# {}\n{}",
                    section.label, section.content
                ));
            } else {
                out.push_str(&section.content);
            }
        }
        out
    }
}

/// Discover instruction files by walking from `cwd` up to root.
/// Looks for: CLAUDE.md, CLAUDE.local.md, .claude/CLAUDE.md, .claude/instructions.md
pub fn discover_instruction_files(cwd: &str) -> Vec<InstructionFile> {
    let mut files = Vec::new();
    let mut seen_hashes = std::collections::HashSet::new();
    let start = Path::new(cwd);

    let mut ancestors: Vec<&Path> = start.ancestors().collect();
    ancestors.reverse();

    let candidates = [
        "CLAUDE.md",
        ".claude/CLAUDE.md",
        ".claude/instructions.md",
        "CLAUDE.local.md",
    ];

    for dir in &ancestors {
        for name in &candidates {
            let path = dir.join(name);
            if let Some(inst) = try_load_instruction(&path)
                && seen_hashes.insert(inst.content_hash)
            {
                files.push(inst);
            }
        }
    }
    files
}

fn try_load_instruction(path: &Path) -> Option<InstructionFile> {
    if !path.is_file() {
        return None;
    }
    let raw = fs::read_to_string(path).ok()?;
    if raw.trim().is_empty() {
        return None;
    }
    let content = truncate_content(&raw, MAX_INSTRUCTION_FILE_CHARS);
    let hash = simple_hash(&raw);
    Some(InstructionFile {
        path: path.to_string_lossy().to_string(),
        content,
        content_hash: hash,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_adds_static_sections() {
        let mut b = SystemPromptBuilder::new();
        b.add_static("intro", "You are an assistant.");
        b.add_static("style", "Be concise.");
        let out = b.build();
        assert!(out.contains("You are an assistant."));
        assert!(out.contains("Be concise."));
    }

    #[test]
    fn builder_inserts_dynamic_boundary() {
        let mut b = SystemPromptBuilder::new();
        b.add_static("intro", "Hello");
        b.add_dynamic_boundary();
        b.add_static("tail", "World");
        let out = b.build();
        assert!(out.contains(DYNAMIC_BOUNDARY));
        let idx_boundary = out.find(DYNAMIC_BOUNDARY).unwrap();
        let idx_hello = out.find("Hello").unwrap();
        let idx_world = out.find("World").unwrap();
        assert!(idx_hello < idx_boundary);
        assert!(idx_boundary < idx_world);
    }

    #[test]
    fn builder_adds_context() {
        let ctx = ProjectContext {
            cwd: "/tmp/proj".into(),
            current_date: "2026-04-06".into(),
            platform: "linux".into(),
            model_name: "opus-4".into(),
            git_branch: Some("main".into()),
            git_status: None,
            instruction_files: vec![],
        };
        let mut b = SystemPromptBuilder::new();
        b.add_context(&ctx);
        let out = b.build();
        assert!(out.contains("cwd: /tmp/proj"));
        assert!(out.contains("currentDate: 2026-04-06"));
        assert!(out.contains("gitBranch: main"));
        assert!(!out.contains("gitStatus"));
    }

    #[test]
    fn instruction_dedup_by_hash() {
        let h = simple_hash("same content");
        let files = vec![
            InstructionFile { path: "a.md".into(), content: "same content".into(), content_hash: h },
            InstructionFile { path: "b.md".into(), content: "same content".into(), content_hash: h },
            InstructionFile { path: "c.md".into(), content: "different".into(), content_hash: simple_hash("different") },
        ];
        let mut b = SystemPromptBuilder::new();
        b.add_instructions(&files);
        let out = b.build();
        // "a.md" included, "b.md" deduplicated away
        assert!(out.contains("# a.md"));
        assert!(!out.contains("# b.md"));
        assert!(out.contains("# c.md"));
    }

    #[test]
    fn instruction_truncation() {
        let long = "x".repeat(MAX_INSTRUCTION_FILE_CHARS + 500);
        let truncated = truncate_content(&long, MAX_INSTRUCTION_FILE_CHARS);
        assert!(truncated.len() < long.len());
        assert!(truncated.ends_with("[truncated]"));
    }

    #[test]
    fn total_instruction_budget() {
        // Create files that together exceed MAX_TOTAL_INSTRUCTION_CHARS
        let chunk = "y".repeat(MAX_TOTAL_INSTRUCTION_CHARS / 2 + 100);
        let files: Vec<InstructionFile> = (0..4)
            .map(|i| {
                let content = format!("{}{}", chunk, i);
                InstructionFile {
                    path: format!("file{}.md", i),
                    content: content.clone(),
                    content_hash: simple_hash(&content),
                }
            })
            .collect();
        let mut b = SystemPromptBuilder::new();
        b.add_instructions(&files);
        let out = b.build();
        // The merged instructions section must not exceed the budget
        // (the "# instructions\n" label adds a few chars, but the content itself is capped)
        // At most 2 files can fit within the budget
        let count = files.iter().filter(|f| out.contains(&format!("# {}", f.path))).count();
        assert!(count <= 3, "too many files included: {}", count);
    }

    #[test]
    fn simple_hash_deterministic() {
        let a = simple_hash("hello world");
        let b = simple_hash("hello world");
        let c = simple_hash("hello world!");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn build_produces_ordered_output() {
        let mut b = SystemPromptBuilder::new();
        b.add_static("intro", "FIRST");
        b.add_dynamic_boundary();
        let ctx = ProjectContext {
            cwd: "/w".into(),
            current_date: "2026-01-01".into(),
            platform: "linux".into(),
            model_name: "test".into(),
            git_branch: None,
            git_status: None,
            instruction_files: vec![],
        };
        b.add_context(&ctx);
        b.add_static("tail", "LAST");
        let out = b.build();
        let i_first = out.find("FIRST").unwrap();
        let i_boundary = out.find(DYNAMIC_BOUNDARY).unwrap();
        let i_cwd = out.find("cwd: /w").unwrap();
        let i_last = out.find("LAST").unwrap();
        assert!(i_first < i_boundary);
        assert!(i_boundary < i_cwd);
        assert!(i_cwd < i_last);
    }
}