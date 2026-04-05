use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// MemoryType
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// MemoryEntry
// ---------------------------------------------------------------------------

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
        // Split on the closing --- delimiter
        let after_open = &trimmed[3..];
        let close_pos = after_open.find("\n---")?;
        let frontmatter = &after_open[..close_pos];
        let body_start = close_pos + 4; // skip \n---
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
        let type_str = *fields.get("type")?;
        let memory_type = MemoryType::parse(type_str)?;

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
            // Format: - [Title](file.md) — one-line hook
            if !line.starts_with("- [") {
                continue;
            }
            let after_bracket = &line[3..];
            let close_bracket = match after_bracket.find("](") {
                Some(p) => p,
                None => continue,
            };
            let title = after_bracket[..close_bracket].to_string();
            let after_paren = &after_bracket[close_bracket + 2..];
            let close_paren = match after_paren.find(')') {
                Some(p) => p,
                None => continue,
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
    base_dir: PathBuf,
}

impl MemoryStore {
    pub fn new(base_dir: &str) -> Self {
        Self {
            base_dir: PathBuf::from(base_dir),
        }
    }

    pub fn ensure_dir(&self) -> Result<(), String> {
        fs::create_dir_all(&self.base_dir)
            .map_err(|e| format!("failed to create memory dir: {e}"))
    }

    pub fn save(&self, entry: &MemoryEntry) -> Result<(), String> {
        self.ensure_dir()?;
        let path = self.base_dir.join(&entry.file_name);
        fs::write(&path, entry.to_markdown())
            .map_err(|e| format!("failed to write {}: {e}", entry.file_name))
    }

    pub fn load(&self, file_name: &str) -> Result<MemoryEntry, String> {
        let path = self.base_dir.join(file_name);
        let raw = fs::read_to_string(&path)
            .map_err(|e| format!("failed to read {file_name}: {e}"))?;
        MemoryEntry::from_markdown(file_name, &raw)
            .ok_or_else(|| format!("failed to parse {file_name}"))
    }

    pub fn delete(&self, file_name: &str) -> Result<(), String> {
        let path = self.base_dir.join(file_name);
        fs::remove_file(&path)
            .map_err(|e| format!("failed to delete {file_name}: {e}"))
    }

    pub fn list(&self) -> Result<Vec<MemoryEntry>, String> {
        let dir = fs::read_dir(&self.base_dir)
            .map_err(|e| format!("failed to read memory dir: {e}"))?;
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

    // -----------------------------------------------------------------------
    // MEMORY.md index management
    // -----------------------------------------------------------------------

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
        let raw = fs::read_to_string(&path)
            .map_err(|e| format!("failed to read MEMORY.md: {e}"))?;
        Ok(MemoryIndex::parse(&raw))
    }

    pub fn save_index(&self, index: &MemoryIndex) -> Result<(), String> {
        self.ensure_dir()?;
        let path = self.index_path();
        fs::write(&path, index.render())
            .map_err(|e| format!("failed to write MEMORY.md: {e}"))
    }

    pub fn add_to_index(&self, entry: &MemoryEntry) -> Result<(), String> {
        let mut index = self.load_index()?;
        // Remove existing entry for same file if present
        index
            .entries
            .retain(|e| e.file_name != entry.file_name);
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
// Tests
// ---------------------------------------------------------------------------

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
    fn memory_type_from_str_roundtrip() {
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
    fn memory_entry_to_markdown_roundtrip() {
        let entry = sample_entry();
        let md = entry.to_markdown();
        let parsed = MemoryEntry::from_markdown("user_role.md", &md).unwrap();
        assert_eq!(parsed.name, entry.name);
        assert_eq!(parsed.description, entry.description);
        assert_eq!(parsed.memory_type, entry.memory_type);
        assert_eq!(parsed.content, entry.content);
        assert_eq!(parsed.file_name, "user_role.md");
    }

    #[test]
    fn memory_entry_from_markdown_parses_frontmatter() {
        let md = "---\nname: test\ndescription: a test entry\ntype: feedback\n---\n\nBody text here.";
        let entry = MemoryEntry::from_markdown("test.md", md).unwrap();
        assert_eq!(entry.name, "test");
        assert_eq!(entry.description, "a test entry");
        assert_eq!(entry.memory_type, MemoryType::Feedback);
        assert_eq!(entry.content, "Body text here.");
    }

    #[test]
    fn memory_entry_from_markdown_rejects_invalid() {
        assert!(MemoryEntry::from_markdown("x.md", "no frontmatter").is_none());
        assert!(MemoryEntry::from_markdown("x.md", "---\nname: x\n---\n\nbody").is_none());
    }

    #[test]
    fn parse_index_extracts_entries() {
        let content = "- [Role](user_role.md) \u{2014} user is a pentester\n- [Feedback](feedback_testing.md) \u{2014} no mocks\n";
        let idx = MemoryIndex::parse(content);
        assert_eq!(idx.entries.len(), 2);
        assert_eq!(idx.entries[0].title, "Role");
        assert_eq!(idx.entries[0].file_name, "user_role.md");
        assert_eq!(idx.entries[0].hook, "user is a pentester");
        assert_eq!(idx.entries[1].title, "Feedback");
        assert_eq!(idx.entries[1].file_name, "feedback_testing.md");
    }

    #[test]
    fn render_index_format() {
        let idx = MemoryIndex {
            entries: vec![MemoryIndexEntry {
                title: "Role".to_string(),
                file_name: "user_role.md".to_string(),
                hook: "user is a pentester".to_string(),
            }],
        };
        let rendered = idx.render();
        assert_eq!(rendered, "- [Role](user_role.md) \u{2014} user is a pentester\n");
    }

    #[test]
    fn save_and_load_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(tmp.path().to_str().unwrap());
        let entry = sample_entry();
        store.save(&entry).unwrap();
        let loaded = store.load("user_role.md").unwrap();
        assert_eq!(loaded.name, entry.name);
        assert_eq!(loaded.content, entry.content);
        assert_eq!(loaded.memory_type, entry.memory_type);
    }

    #[test]
    fn list_returns_all_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(tmp.path().to_str().unwrap());
        let e1 = sample_entry();
        let e2 = MemoryEntry {
            name: "proj_goal".to_string(),
            description: "current project goal".to_string(),
            memory_type: MemoryType::Project,
            content: "Ship v1 by Friday.".to_string(),
            file_name: "project_goal.md".to_string(),
        };
        store.save(&e1).unwrap();
        store.save(&e2).unwrap();
        let all = store.list().unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn search_matches_name_and_content() {
        let tmp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(tmp.path().to_str().unwrap());
        let e1 = sample_entry();
        let e2 = MemoryEntry {
            name: "feedback_testing".to_string(),
            description: "no mocks in tests".to_string(),
            memory_type: MemoryType::Feedback,
            content: "Use real database for integration tests.".to_string(),
            file_name: "feedback_testing.md".to_string(),
        };
        store.save(&e1).unwrap();
        store.save(&e2).unwrap();

        let by_name = store.search("pentester").unwrap();
        assert_eq!(by_name.len(), 1);
        assert_eq!(by_name[0].file_name, "user_role.md");

        let by_content = store.search("database").unwrap();
        assert_eq!(by_content.len(), 1);
        assert_eq!(by_content[0].file_name, "feedback_testing.md");
    }

    #[test]
    fn delete_removes_file() {
        let tmp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(tmp.path().to_str().unwrap());
        let entry = sample_entry();
        store.save(&entry).unwrap();
        assert!(store.load("user_role.md").is_ok());
        store.delete("user_role.md").unwrap();
        assert!(store.load("user_role.md").is_err());
    }

    #[test]
    fn add_to_index_updates_memory_md() {
        let tmp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(tmp.path().to_str().unwrap());
        let entry = sample_entry();
        store.add_to_index(&entry).unwrap();

        let idx = store.load_index().unwrap();
        assert_eq!(idx.entries.len(), 1);
        assert_eq!(idx.entries[0].title, "user_role");
        assert_eq!(idx.entries[0].file_name, "user_role.md");

        // Adding again should not duplicate
        store.add_to_index(&entry).unwrap();
        let idx2 = store.load_index().unwrap();
        assert_eq!(idx2.entries.len(), 1);
    }

    #[test]
    fn remove_from_index_cleans_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(tmp.path().to_str().unwrap());
        let entry = sample_entry();
        store.add_to_index(&entry).unwrap();
        store.remove_from_index("user_role.md").unwrap();
        let idx = store.load_index().unwrap();
        assert!(idx.entries.is_empty());
    }

    #[test]
    fn find_by_name_returns_match() {
        let tmp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(tmp.path().to_str().unwrap());
        let entry = sample_entry();
        store.save(&entry).unwrap();
        let found = store.find_by_name("user_role").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().file_name, "user_role.md");
        let missing = store.find_by_name("nonexistent").unwrap();
        assert!(missing.is_none());
    }
}