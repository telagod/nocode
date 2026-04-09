//! Memory store — file-system CRUD with Markdown + YAML frontmatter.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemoryType {
    User,
    Feedback,
    Project,
    Reference,
}

impl MemoryType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Feedback => "feedback",
            Self::Project => "project",
            Self::Reference => "reference",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "user" => Some(Self::User),
            "feedback" => Some(Self::Feedback),
            "project" => Some(Self::Project),
            "reference" => Some(Self::Reference),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MemoryEntry {
    pub name: String,
    pub description: String,
    pub memory_type: MemoryType,
    pub content: String,
    pub file_name: String,
}

impl MemoryEntry {
    pub fn to_markdown(&self) -> String {
        format!(
            "---\nname: {}\ndescription: {}\ntype: {}\n---\n\n{}",
            self.name,
            self.description,
            self.memory_type.as_str(),
            self.content
        )
    }

    pub fn from_markdown(file_name: &str, raw: &str) -> Option<Self> {
        let trimmed = raw.trim_start();
        if !trimmed.starts_with("---") {
            return None;
        }
        let after_open = &trimmed[3..];
        let close_pos = after_open.find("\n---")?;
        let frontmatter = &after_open[..close_pos];
        let body_start = close_pos + 4;
        let body = after_open[body_start..].trim_start_matches('\n');

        let mut fields: HashMap<&str, &str> = HashMap::new();
        for line in frontmatter.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(colon) = line.find(':') {
                let key = line[..colon].trim();
                let val = line[colon + 1..].trim();
                fields.insert(key, val);
            }
        }

        let name = fields.get("name")?.to_string();
        let description = fields.get("description")?.to_string();
        let memory_type = MemoryType::parse(fields.get("type")?)?;

        Some(Self {
            name,
            description,
            memory_type,
            content: body.to_string(),
            file_name: file_name.to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// MemoryIndex
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct MemoryIndexEntry {
    pub title: String,
    pub file_name: String,
    pub hook: String,
}

#[derive(Debug, Clone)]
pub struct MemoryIndex {
    pub entries: Vec<MemoryIndexEntry>,
}

impl MemoryIndex {
    pub fn parse(content: &str) -> Self {
        let mut entries = Vec::new();
        for line in content.lines() {
            let line = line.trim();
            if !line.starts_with("- [") {
                continue;
            }
            let after_bracket = &line[3..];
            let Some(close_bracket) = after_bracket.find("](") else {
                continue;
            };
            let title = after_bracket[..close_bracket].to_string();
            let after_paren = &after_bracket[close_bracket + 2..];
            let Some(close_paren) = after_paren.find(')') else {
                continue;
            };
            let file_name = after_paren[..close_paren].to_string();
            let rest = &after_paren[close_paren + 1..];
            let hook = rest
                .trim_start_matches([' ', '\u{2014}', '-'])
                .trim()
                .to_string();
            entries.push(MemoryIndexEntry {
                title,
                file_name,
                hook,
            });
        }
        Self { entries }
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        for e in &self.entries {
            out.push_str(&format!(
                "- [{}]({}) \u{2014} {}\n",
                e.title, e.file_name, e.hook
            ));
        }
        out
    }
}

// ---------------------------------------------------------------------------
// MemoryStore
// ---------------------------------------------------------------------------

pub struct MemoryStore {
    pub(crate) base_dir: PathBuf,
}

impl MemoryStore {
    pub fn new(base_dir: &str) -> Self {
        Self {
            base_dir: PathBuf::from(base_dir),
        }
    }

    pub fn ensure_dir(&self) -> Result<(), String> {
        fs::create_dir_all(&self.base_dir).map_err(|e| format!("failed to create memory dir: {e}"))
    }

    pub fn save(&self, entry: &MemoryEntry) -> Result<(), String> {
        self.ensure_dir()?;
        let path = self.base_dir.join(&entry.file_name);
        fs::write(&path, entry.to_markdown())
            .map_err(|e| format!("failed to write {}: {e}", entry.file_name))
    }

    pub fn load(&self, file_name: &str) -> Result<MemoryEntry, String> {
        let path = self.base_dir.join(file_name);
        let raw =
            fs::read_to_string(&path).map_err(|e| format!("failed to read {file_name}: {e}"))?;
        MemoryEntry::from_markdown(file_name, &raw)
            .ok_or_else(|| format!("failed to parse {file_name}"))
    }

    pub fn delete(&self, file_name: &str) -> Result<(), String> {
        let path = self.base_dir.join(file_name);
        fs::remove_file(&path).map_err(|e| format!("failed to delete {file_name}: {e}"))
    }

    pub fn list(&self) -> Result<Vec<MemoryEntry>, String> {
        let dir =
            fs::read_dir(&self.base_dir).map_err(|e| format!("failed to read memory dir: {e}"))?;
        let mut entries = Vec::new();
        for item in dir {
            let item = item.map_err(|e| format!("dir entry error: {e}"))?;
            let path = item.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            let fname = item.file_name().to_string_lossy().to_string();
            if fname == "MEMORY.md" {
                continue;
            }
            if let Ok(entry) = self.load(&fname) {
                entries.push(entry);
            }
        }
        Ok(entries)
    }

    pub fn search(&self, query: &str) -> Result<Vec<MemoryEntry>, String> {
        let q = query.to_lowercase();
        let all = self.list()?;
        Ok(all
            .into_iter()
            .filter(|e| {
                e.name.to_lowercase().contains(&q)
                    || e.content.to_lowercase().contains(&q)
                    || e.description.to_lowercase().contains(&q)
            })
            .collect())
    }

    pub fn find_by_name(&self, name: &str) -> Result<Option<MemoryEntry>, String> {
        let all = self.list()?;
        Ok(all.into_iter().find(|e| e.name == name))
    }

    fn index_path(&self) -> PathBuf {
        self.base_dir.join("MEMORY.md")
    }

    pub fn load_index(&self) -> Result<MemoryIndex, String> {
        let path = self.index_path();
        if !path.exists() {
            return Ok(MemoryIndex {
                entries: Vec::new(),
            });
        }
        let raw =
            fs::read_to_string(&path).map_err(|e| format!("failed to read MEMORY.md: {e}"))?;
        Ok(MemoryIndex::parse(&raw))
    }

    pub fn save_index(&self, index: &MemoryIndex) -> Result<(), String> {
        self.ensure_dir()?;
        let path = self.index_path();
        fs::write(&path, index.render()).map_err(|e| format!("failed to write MEMORY.md: {e}"))
    }

    pub fn add_to_index(&self, entry: &MemoryEntry) -> Result<(), String> {
        let mut index = self.load_index()?;
        index.entries.retain(|e| e.file_name != entry.file_name);
        index.entries.push(MemoryIndexEntry {
            title: entry.name.clone(),
            file_name: entry.file_name.clone(),
            hook: entry.description.clone(),
        });
        self.save_index(&index)
    }

    pub fn remove_from_index(&self, file_name: &str) -> Result<(), String> {
        let mut index = self.load_index()?;
        index.entries.retain(|e| e.file_name != file_name);
        self.save_index(&index)
    }
}

// ---------------------------------------------------------------------------
// SessionMemory — cross-session memory extraction and loading
// ---------------------------------------------------------------------------

/// Manages cross-session memory: extracting key info from conversations
/// and loading relevant memories into system prompts.
pub struct SessionMemory {
    store: MemoryStore,
}

impl SessionMemory {
    pub fn new(base_dir: &str) -> Self {
        Self {
            store: MemoryStore::new(base_dir),
        }
    }

    /// Load all memories and format them for inclusion in a system prompt.
    pub fn load_for_prompt(&self) -> String {
        let entries = match self.store.list() {
            Ok(e) => e,
            Err(_) => return String::new(),
        };
        if entries.is_empty() {
            return String::new();
        }

        let mut sections: HashMap<MemoryType, Vec<&MemoryEntry>> = HashMap::new();
        for entry in &entries {
            sections.entry(entry.memory_type).or_default().push(entry);
        }

        let mut out = String::from("# Recalled Memories\n\n");
        let type_order = [
            MemoryType::User,
            MemoryType::Feedback,
            MemoryType::Project,
            MemoryType::Reference,
        ];
        for ty in &type_order {
            if let Some(entries) = sections.get(ty) {
                out.push_str(&format!("## {}\n\n", ty.as_str()));
                for entry in entries {
                    out.push_str(&format!("### {}\n{}\n\n", entry.name, entry.content));
                }
            }
        }
        out
    }

    /// Load memories relevant to a query (by keyword search).
    pub fn load_relevant(&self, query: &str) -> String {
        let entries = match self.store.search(query) {
            Ok(e) => e,
            Err(_) => return String::new(),
        };
        if entries.is_empty() {
            return String::new();
        }

        let mut out = String::from("# Relevant Memories\n\n");
        for entry in &entries {
            out.push_str(&format!(
                "- **{}** ({}): {}\n",
                entry.name,
                entry.memory_type.as_str(),
                entry.description
            ));
        }
        out
    }

    /// Extract and save a memory from explicit user instruction.
    pub fn save_memory(
        &self,
        name: &str,
        description: &str,
        memory_type: MemoryType,
        content: &str,
    ) -> Result<String, String> {
        let file_name = format!(
            "{}_{}.md",
            memory_type.as_str(),
            name.to_lowercase()
                .replace(' ', "_")
                .replace(|c: char| !c.is_alphanumeric() && c != '_', "")
        );

        // Check for existing memory with same name — update instead of duplicate
        if let Ok(Some(existing)) = self.store.find_by_name(name) {
            self.store.delete(&existing.file_name)?;
            self.store.remove_from_index(&existing.file_name)?;
        }

        let entry = MemoryEntry {
            name: name.to_string(),
            description: description.to_string(),
            memory_type,
            content: content.to_string(),
            file_name: file_name.clone(),
        };

        self.store.save(&entry)?;
        self.store.add_to_index(&entry)?;
        Ok(file_name)
    }

    /// Delete a memory by name.
    pub fn delete_memory(&self, name: &str) -> Result<(), String> {
        let entry = self
            .store
            .find_by_name(name)?
            .ok_or_else(|| format!("Memory '{name}' not found"))?;
        self.store.delete(&entry.file_name)?;
        self.store.remove_from_index(&entry.file_name)?;
        Ok(())
    }

    /// List all memories as a summary.
    pub fn list_summary(&self) -> Result<Vec<(String, String, String)>, String> {
        let entries = self.store.list()?;
        Ok(entries
            .iter()
            .map(|e| {
                (
                    e.name.clone(),
                    e.memory_type.as_str().to_string(),
                    e.description.clone(),
                )
            })
            .collect())
    }

    /// Get the underlying store for direct access.
    pub fn store(&self) -> &MemoryStore {
        &self.store
    }
}

// ---------------------------------------------------------------------------
// MemoryExtractor — automatic memory extraction from conversations
// ---------------------------------------------------------------------------

/// A candidate memory extracted from conversation.
#[derive(Debug, Clone)]
pub struct MemoryCandidate {
    pub name: String,
    pub description: String,
    pub memory_type: MemoryType,
    pub content: String,
    /// Confidence score (0.0 to 1.0).
    pub confidence: f64,
}

/// Pattern-based memory extractor — identifies key information from conversation text.
pub struct MemoryExtractor;

/// Patterns that indicate user preferences/feedback.
const FEEDBACK_PATTERNS: &[&str] = &[
    "don't ",
    "do not ",
    "stop ",
    "always ",
    "never ",
    "prefer ",
    "i like ",
    "i hate ",
    "please use ",
    "please avoid ",
    "不要",
    "总是",
    "永远不",
    "请用",
    "请避免",
];

/// Patterns that indicate user identity/role.
const USER_PATTERNS: &[&str] = &[
    "i am a ",
    "i'm a ",
    "my role ",
    "i work ",
    "my team ",
    "我是",
    "我的角色",
    "我负责",
];

/// Patterns that indicate project decisions.
const PROJECT_PATTERNS: &[&str] = &[
    "we decided ",
    "the plan is ",
    "deadline ",
    "milestone ",
    "we're using ",
    "we switched ",
    "blocked by ",
    "我们决定",
    "计划是",
    "截止日期",
    "里程碑",
];

/// Patterns that indicate external references.
const REFERENCE_PATTERNS: &[&str] = &[
    "check the ",
    "documented in ",
    "tracked in ",
    "the wiki ",
    "the dashboard ",
    "jira ",
    "linear ",
    "confluence ",
    "参考",
    "文档在",
    "记录在",
];

impl MemoryExtractor {
    /// Extract memory candidates from a list of user messages.
    pub fn extract(messages: &[crate::message::Message]) -> Vec<MemoryCandidate> {
        let mut candidates = Vec::new();

        for msg in messages {
            if msg.role != crate::message::Role::User {
                continue;
            }
            let text = msg.text_content();
            if text.len() < 10 {
                continue;
            }
            let lower = text.to_lowercase();

            // Check feedback patterns
            for pattern in FEEDBACK_PATTERNS {
                if lower.contains(pattern) {
                    let name = Self::extract_name(&text, "feedback");
                    candidates.push(MemoryCandidate {
                        name,
                        description: Self::truncate(&text, 80),
                        memory_type: MemoryType::Feedback,
                        content: text.clone(),
                        confidence: 0.7,
                    });
                    break;
                }
            }

            // Check user identity patterns
            for pattern in USER_PATTERNS {
                if lower.contains(pattern) {
                    let name = Self::extract_name(&text, "user");
                    candidates.push(MemoryCandidate {
                        name,
                        description: Self::truncate(&text, 80),
                        memory_type: MemoryType::User,
                        content: text.clone(),
                        confidence: 0.8,
                    });
                    break;
                }
            }

            // Check project patterns
            for pattern in PROJECT_PATTERNS {
                if lower.contains(pattern) {
                    let name = Self::extract_name(&text, "project");
                    candidates.push(MemoryCandidate {
                        name,
                        description: Self::truncate(&text, 80),
                        memory_type: MemoryType::Project,
                        content: text.clone(),
                        confidence: 0.6,
                    });
                    break;
                }
            }

            // Check reference patterns
            for pattern in REFERENCE_PATTERNS {
                if lower.contains(pattern) {
                    let name = Self::extract_name(&text, "reference");
                    candidates.push(MemoryCandidate {
                        name,
                        description: Self::truncate(&text, 80),
                        memory_type: MemoryType::Reference,
                        content: text.clone(),
                        confidence: 0.6,
                    });
                    break;
                }
            }
        }

        // Deduplicate by name
        candidates.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut seen = std::collections::HashSet::new();
        candidates.retain(|c| seen.insert(c.name.clone()));
        candidates
    }

    /// Extract a short name from text.
    fn extract_name(text: &str, prefix: &str) -> String {
        let words: Vec<&str> = text.split_whitespace().take(5).collect();
        let slug = words.join("_").to_lowercase();
        let clean: String = slug
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '_')
            .take(30)
            .collect();
        format!("{prefix}_{clean}")
    }

    fn truncate(text: &str, max: usize) -> String {
        if text.len() <= max {
            text.to_string()
        } else {
            format!("{}...", &text[..max.min(text.len())])
        }
    }
}

// ---------------------------------------------------------------------------
// DreamConsolidator — automatic memory cleanup and consolidation
// ---------------------------------------------------------------------------

/// Report from a consolidation run.
#[derive(Debug, Clone, Default)]
pub struct ConsolidationReport {
    /// Number of memories scanned.
    pub scanned: usize,
    /// Number of duplicate memories merged.
    pub merged: usize,
    /// Number of stale project memories removed.
    pub expired: usize,
    /// Number of memories remaining after consolidation.
    pub remaining: usize,
}

/// Consolidates memories: merges duplicates, removes stale project entries.
pub struct DreamConsolidator {
    store: MemoryStore,
    /// Max age in days for project memories before they're considered stale.
    project_max_age_days: u64,
}

impl DreamConsolidator {
    pub fn new(base_dir: &str, project_max_age_days: u64) -> Self {
        Self {
            store: MemoryStore::new(base_dir),
            project_max_age_days,
        }
    }

    /// Run consolidation: merge similar memories, expire old project memories.
    pub fn consolidate(&self) -> Result<ConsolidationReport, String> {
        let entries = self.store.list()?;
        let mut report = ConsolidationReport {
            scanned: entries.len(),
            ..Default::default()
        };

        // 1. Find and merge duplicates (same type + similar name)
        let mut seen: std::collections::HashMap<(String, String), MemoryEntry> =
            std::collections::HashMap::new();
        let mut to_delete: Vec<String> = Vec::new();

        for entry in &entries {
            let key = (entry.memory_type.as_str().to_string(), normalize_name(&entry.name));
            if let Some(existing) = seen.get(&key) {
                // Merge: keep the longer content, delete the shorter
                if entry.content.len() > existing.content.len() {
                    to_delete.push(existing.file_name.clone());
                    seen.insert(key, entry.clone());
                } else {
                    to_delete.push(entry.file_name.clone());
                }
                report.merged += 1;
            } else {
                seen.insert(key, entry.clone());
            }
        }

        // 2. Expire old project memories
        let _now = chrono::Utc::now().timestamp();
        let max_age_secs = (self.project_max_age_days * 86400) as i64;

        for entry in &entries {
            if entry.memory_type != MemoryType::Project {
                continue;
            }
            if to_delete.contains(&entry.file_name) {
                continue;
            }
            // Check file modification time as proxy for age
            let path = std::path::Path::new(self.store.base_dir.as_path()).join(&entry.file_name);
            if let Ok(meta) = std::fs::metadata(&path)
                && let Ok(modified) = meta.modified()
                && let Ok(age) = modified.elapsed()
            {
                let age_secs = age.as_secs() as i64;
                if age_secs > max_age_secs {
                    to_delete.push(entry.file_name.clone());
                    report.expired += 1;
                }
            }
        }

        // 3. Execute deletions
        for file_name in &to_delete {
            let _ = self.store.delete(file_name);
            let _ = self.store.remove_from_index(file_name);
        }

        // Recount remaining
        report.remaining = self.store.list().unwrap_or_default().len();
        Ok(report)
    }

    /// Quick check: how many memories exist.
    pub fn memory_count(&self) -> usize {
        self.store.list().unwrap_or_default().len()
    }

    /// Check if consolidation is recommended (>20 memories or any project memories older than threshold).
    pub fn should_consolidate(&self) -> bool {
        let entries = match self.store.list() {
            Ok(e) => e,
            Err(_) => return false,
        };
        if entries.len() > 20 {
            return true;
        }
        // Check for old project memories
        let max_age_secs = (self.project_max_age_days * 86400) as i64;
        for entry in &entries {
            if entry.memory_type != MemoryType::Project {
                continue;
            }
            let path = std::path::Path::new(self.store.base_dir.as_path()).join(&entry.file_name);
            if let Ok(meta) = std::fs::metadata(&path)
                && let Ok(modified) = meta.modified()
                && let Ok(age) = modified.elapsed()
            {
                let _ = max_age_secs;
                if age.as_secs() as i64 > max_age_secs {
                    return true;
                }
            }
        }
        false
    }
}

/// Normalize a memory name for dedup comparison.
fn normalize_name(name: &str) -> String {
    name.to_lowercase()
        .replace(['-', ' '], "_")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_')
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entry() -> MemoryEntry {
        MemoryEntry {
            name: "user_role".to_string(),
            description: "user is a security researcher".to_string(),
            memory_type: MemoryType::User,
            content: "Senior pentester, 10 years experience.".to_string(),
            file_name: "user_role.md".to_string(),
        }
    }

    #[test]
    fn memory_type_roundtrip() {
        for ty in &[
            MemoryType::User,
            MemoryType::Feedback,
            MemoryType::Project,
            MemoryType::Reference,
        ] {
            assert_eq!(MemoryType::parse(ty.as_str()), Some(*ty));
        }
        assert_eq!(MemoryType::parse("bogus"), None);
    }

    #[test]
    fn entry_markdown_roundtrip() {
        let entry = sample_entry();
        let md = entry.to_markdown();
        let parsed = MemoryEntry::from_markdown("user_role.md", &md).unwrap();
        assert_eq!(parsed.name, entry.name);
        assert_eq!(parsed.description, entry.description);
        assert_eq!(parsed.memory_type, entry.memory_type);
        assert_eq!(parsed.content, entry.content);
    }

    #[test]
    fn entry_from_markdown_parses_frontmatter() {
        let md = "---\nname: test\ndescription: a test\ntype: feedback\n---\n\nBody text.";
        let entry = MemoryEntry::from_markdown("test.md", md).unwrap();
        assert_eq!(entry.name, "test");
        assert_eq!(entry.memory_type, MemoryType::Feedback);
        assert_eq!(entry.content, "Body text.");
    }

    #[test]
    fn entry_from_markdown_rejects_invalid() {
        assert!(MemoryEntry::from_markdown("x.md", "no frontmatter").is_none());
        assert!(MemoryEntry::from_markdown("x.md", "---\nname: x\n---\n\nbody").is_none());
    }

    #[test]
    fn parse_index() {
        let content =
            "- [Role](user_role.md) \u{2014} pentester\n- [FB](fb.md) \u{2014} no mocks\n";
        let idx = MemoryIndex::parse(content);
        assert_eq!(idx.entries.len(), 2);
        assert_eq!(idx.entries[0].title, "Role");
        assert_eq!(idx.entries[0].file_name, "user_role.md");
    }

    #[test]
    fn render_index() {
        let idx = MemoryIndex {
            entries: vec![MemoryIndexEntry {
                title: "Role".to_string(),
                file_name: "user_role.md".to_string(),
                hook: "pentester".to_string(),
            }],
        };
        let rendered = idx.render();
        assert!(rendered.contains("[Role](user_role.md)"));
    }

    #[test]
    fn store_save_load_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("nocode_mem_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let store = MemoryStore::new(tmp.to_str().unwrap());
        let entry = sample_entry();
        store.save(&entry).unwrap();
        let loaded = store.load("user_role.md").unwrap();
        assert_eq!(loaded.name, entry.name);
        assert_eq!(loaded.content, entry.content);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn store_list_and_search() {
        let tmp = std::env::temp_dir().join(format!("nocode_mem_test2_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let store = MemoryStore::new(tmp.to_str().unwrap());
        store.save(&sample_entry()).unwrap();
        store
            .save(&MemoryEntry {
                name: "feedback_testing".to_string(),
                description: "no mocks".to_string(),
                memory_type: MemoryType::Feedback,
                content: "Use real database.".to_string(),
                file_name: "feedback_testing.md".to_string(),
            })
            .unwrap();
        assert_eq!(store.list().unwrap().len(), 2);
        assert_eq!(store.search("pentester").unwrap().len(), 1);
        assert_eq!(store.search("database").unwrap().len(), 1);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn store_delete() {
        let tmp = std::env::temp_dir().join(format!("nocode_mem_test3_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let store = MemoryStore::new(tmp.to_str().unwrap());
        store.save(&sample_entry()).unwrap();
        store.delete("user_role.md").unwrap();
        assert!(store.load("user_role.md").is_err());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn store_index_operations() {
        let tmp = std::env::temp_dir().join(format!("nocode_mem_test4_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let store = MemoryStore::new(tmp.to_str().unwrap());
        let entry = sample_entry();
        store.add_to_index(&entry).unwrap();
        let idx = store.load_index().unwrap();
        assert_eq!(idx.entries.len(), 1);
        // No duplicates
        store.add_to_index(&entry).unwrap();
        assert_eq!(store.load_index().unwrap().entries.len(), 1);
        store.remove_from_index("user_role.md").unwrap();
        assert!(store.load_index().unwrap().entries.is_empty());
        let _ = fs::remove_dir_all(&tmp);
    }

    // --- SessionMemory ---
    #[test]
    fn session_memory_save_and_load() {
        let tmp = std::env::temp_dir().join(format!("nocode_smem1_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let sm = SessionMemory::new(tmp.to_str().unwrap());
        sm.save_memory(
            "user_role",
            "pentester",
            MemoryType::User,
            "Senior pentester.",
        )
        .unwrap();
        let prompt = sm.load_for_prompt();
        assert!(prompt.contains("user_role"));
        assert!(prompt.contains("Senior pentester"));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn session_memory_deduplicates() {
        let tmp = std::env::temp_dir().join(format!("nocode_smem2_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let sm = SessionMemory::new(tmp.to_str().unwrap());
        sm.save_memory("pref", "v1", MemoryType::Feedback, "old content")
            .unwrap();
        sm.save_memory("pref", "v2", MemoryType::Feedback, "new content")
            .unwrap();
        let list = sm.list_summary().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].2, "v2");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn session_memory_delete() {
        let tmp = std::env::temp_dir().join(format!("nocode_smem3_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let sm = SessionMemory::new(tmp.to_str().unwrap());
        sm.save_memory("temp", "temporary", MemoryType::Project, "will delete")
            .unwrap();
        assert_eq!(sm.list_summary().unwrap().len(), 1);
        sm.delete_memory("temp").unwrap();
        assert!(sm.list_summary().unwrap().is_empty());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn session_memory_load_relevant() {
        let tmp = std::env::temp_dir().join(format!("nocode_smem4_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let sm = SessionMemory::new(tmp.to_str().unwrap());
        sm.save_memory(
            "rust_pref",
            "prefers rust",
            MemoryType::User,
            "Uses Rust daily.",
        )
        .unwrap();
        sm.save_memory("py_pref", "uses python", MemoryType::User, "Python for ML.")
            .unwrap();
        let relevant = sm.load_relevant("rust");
        assert!(relevant.contains("rust_pref"));
        assert!(!relevant.contains("py_pref"));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn session_memory_empty_prompt() {
        let tmp = std::env::temp_dir().join(format!("nocode_smem5_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let sm = SessionMemory::new(tmp.to_str().unwrap());
        sm.store().ensure_dir().unwrap();
        assert!(sm.load_for_prompt().is_empty());
        let _ = fs::remove_dir_all(&tmp);
    }

    // --- MemoryExtractor ---
    #[test]
    fn extract_feedback_from_conversation() {
        use crate::message::Message;
        let msgs = vec![
            Message::user_text("don't mock the database in tests"),
            Message::assistant_text("understood"),
        ];
        let candidates = MemoryExtractor::extract(&msgs);
        assert!(!candidates.is_empty());
        assert_eq!(candidates[0].memory_type, MemoryType::Feedback);
    }

    #[test]
    fn extract_user_identity() {
        use crate::message::Message;
        let msgs = vec![Message::user_text(
            "I'm a senior security researcher at Acme Corp",
        )];
        let candidates = MemoryExtractor::extract(&msgs);
        assert!(!candidates.is_empty());
        assert_eq!(candidates[0].memory_type, MemoryType::User);
    }

    #[test]
    fn extract_project_decision() {
        use crate::message::Message;
        let msgs = vec![Message::user_text(
            "we decided to use PostgreSQL instead of MySQL for the new service",
        )];
        let candidates = MemoryExtractor::extract(&msgs);
        assert!(!candidates.is_empty());
        assert_eq!(candidates[0].memory_type, MemoryType::Project);
    }

    #[test]
    fn extract_reference() {
        use crate::message::Message;
        let msgs = vec![Message::user_text(
            "bugs are tracked in Linear project BACKEND",
        )];
        let candidates = MemoryExtractor::extract(&msgs);
        assert!(!candidates.is_empty());
        assert_eq!(candidates[0].memory_type, MemoryType::Reference);
    }

    #[test]
    fn extract_skips_short_messages() {
        use crate::message::Message;
        let msgs = vec![Message::user_text("ok")];
        let candidates = MemoryExtractor::extract(&msgs);
        assert!(candidates.is_empty());
    }

    #[test]
    fn extract_skips_assistant_messages() {
        use crate::message::Message;
        let msgs = vec![Message::assistant_text(
            "I prefer to always use Rust for this",
        )];
        let candidates = MemoryExtractor::extract(&msgs);
        assert!(candidates.is_empty());
    }

    #[test]
    fn extract_deduplicates() {
        use crate::message::Message;
        let msgs = vec![
            Message::user_text("don't use mocks in integration tests"),
            Message::user_text("don't use mocks in integration tests"),
        ];
        let candidates = MemoryExtractor::extract(&msgs);
        // Same text → same name → deduplicated
        assert_eq!(candidates.len(), 1);
    }

    // --- DreamConsolidator ---

    #[test]
    fn normalize_name_works() {
        assert_eq!(super::normalize_name("User-Role"), "user_role");
        assert_eq!(super::normalize_name("my test NAME"), "my_test_name");
        assert_eq!(super::normalize_name("a!b@c#d"), "abcd");
    }

    #[test]
    fn consolidator_empty_dir() {
        let tmp = std::env::temp_dir().join(format!("nocode_dream1_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let c = DreamConsolidator::new(tmp.to_str().unwrap(), 30);
        let report = c.consolidate().unwrap();
        assert_eq!(report.scanned, 0);
        assert_eq!(report.merged, 0);
        assert_eq!(report.expired, 0);
        assert_eq!(report.remaining, 0);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn consolidator_merges_duplicates() {
        let tmp = std::env::temp_dir().join(format!("nocode_dream2_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let store = MemoryStore::new(tmp.to_str().unwrap());
        // Two feedback memories with similar normalized names
        store.save(&MemoryEntry {
            name: "feedback_testing".to_string(),
            description: "no mocks v1".to_string(),
            memory_type: MemoryType::Feedback,
            content: "short".to_string(),
            file_name: "feedback_testing_1.md".to_string(),
        }).unwrap();
        store.save(&MemoryEntry {
            name: "feedback_testing".to_string(),
            description: "no mocks v2".to_string(),
            memory_type: MemoryType::Feedback,
            content: "longer content wins the merge".to_string(),
            file_name: "feedback_testing_2.md".to_string(),
        }).unwrap();

        let c = DreamConsolidator::new(tmp.to_str().unwrap(), 30);
        let report = c.consolidate().unwrap();
        assert_eq!(report.scanned, 2);
        assert_eq!(report.merged, 1);
        assert_eq!(report.remaining, 1);
        // The longer one survives
        let remaining = store.list().unwrap();
        assert!(remaining[0].content.contains("longer"));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn consolidator_no_false_merges() {
        let tmp = std::env::temp_dir().join(format!("nocode_dream3_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let store = MemoryStore::new(tmp.to_str().unwrap());
        store.save(&MemoryEntry {
            name: "user_role".to_string(),
            description: "pentester".to_string(),
            memory_type: MemoryType::User,
            content: "Senior pentester.".to_string(),
            file_name: "user_role.md".to_string(),
        }).unwrap();
        store.save(&MemoryEntry {
            name: "feedback_testing".to_string(),
            description: "no mocks".to_string(),
            memory_type: MemoryType::Feedback,
            content: "Use real database.".to_string(),
            file_name: "feedback_testing.md".to_string(),
        }).unwrap();

        let c = DreamConsolidator::new(tmp.to_str().unwrap(), 30);
        let report = c.consolidate().unwrap();
        assert_eq!(report.scanned, 2);
        assert_eq!(report.merged, 0);
        assert_eq!(report.remaining, 2);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn consolidator_should_consolidate_threshold() {
        let tmp = std::env::temp_dir().join(format!("nocode_dream4_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let store = MemoryStore::new(tmp.to_str().unwrap());
        // Create 21 memories to trigger threshold
        for i in 0..21 {
            store.save(&MemoryEntry {
                name: format!("mem_{i}"),
                description: format!("desc {i}"),
                memory_type: MemoryType::User,
                content: format!("content {i}"),
                file_name: format!("mem_{i}.md"),
            }).unwrap();
        }
        let c = DreamConsolidator::new(tmp.to_str().unwrap(), 30);
        assert!(c.should_consolidate());
        assert_eq!(c.memory_count(), 21);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn consolidator_report_default() {
        let report = ConsolidationReport::default();
        assert_eq!(report.scanned, 0);
        assert_eq!(report.merged, 0);
        assert_eq!(report.expired, 0);
        assert_eq!(report.remaining, 0);
    }
}
