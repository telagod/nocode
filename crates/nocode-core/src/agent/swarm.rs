//! Agent swarm — parallel multi-agent task execution with file ownership.
//!
//! Coordinates multiple workers executing subtasks in parallel,
//! enforcing file ownership (one writer per file at a time),
//! and converging results.

use std::collections::HashMap;

/// File ownership matrix — tracks which agent owns which files.
#[derive(Debug, Default)]
pub struct FileOwnership {
    /// file_path → agent_id
    owners: HashMap<String, String>,
}

impl FileOwnership {
    pub fn new() -> Self {
        Self::default()
    }

    /// Claim ownership of files for an agent. Returns Err if any file is already owned.
    pub fn claim(&mut self, agent_id: &str, files: &[String]) -> Result<(), String> {
        for file in files {
            if let Some(owner) = self.owners.get(file)
                && owner != agent_id
            {
                return Err(format!("File '{file}' already owned by agent '{owner}'"));
            }
        }
        for file in files {
            self.owners.insert(file.clone(), agent_id.to_string());
        }
        Ok(())
    }

    /// Release all files owned by an agent.
    pub fn release(&mut self, agent_id: &str) {
        self.owners.retain(|_, v| v != agent_id);
    }

    /// Check if a file is owned by a specific agent.
    pub fn is_owned_by(&self, file: &str, agent_id: &str) -> bool {
        self.owners.get(file).is_some_and(|o| o == agent_id)
    }

    /// Get the owner of a file.
    pub fn owner_of(&self, file: &str) -> Option<&str> {
        self.owners.get(file).map(String::as_str)
    }

    /// List all files owned by an agent.
    pub fn files_for(&self, agent_id: &str) -> Vec<&str> {
        self.owners
            .iter()
            .filter(|(_, v)| v.as_str() == agent_id)
            .map(|(k, _)| k.as_str())
            .collect()
    }
}

/// A subtask assigned to an agent in the swarm.
#[derive(Debug, Clone)]
pub struct SwarmSubtask {
    pub id: String,
    pub agent_id: String,
    pub description: String,
    pub files: Vec<String>,
    pub status: SwarmSubtaskStatus,
    pub result: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwarmSubtaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

/// Swarm coordinator — manages parallel agent execution.
pub struct SwarmCoordinator {
    pub name: String,
    pub subtasks: Vec<SwarmSubtask>,
    pub file_ownership: FileOwnership,
    next_id: u32,
}

impl SwarmCoordinator {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            subtasks: Vec::new(),
            file_ownership: FileOwnership::new(),
            next_id: 1,
        }
    }

    /// Add a subtask with file assignments.
    pub fn add_subtask(
        &mut self,
        agent_id: &str,
        description: &str,
        files: Vec<String>,
    ) -> Result<String, String> {
        self.file_ownership.claim(agent_id, &files)?;
        let id = format!("swarm-{}-{}", self.name, self.next_id);
        self.next_id += 1;
        self.subtasks.push(SwarmSubtask {
            id: id.clone(),
            agent_id: agent_id.to_string(),
            description: description.to_string(),
            files,
            status: SwarmSubtaskStatus::Pending,
            result: None,
        });
        Ok(id)
    }

    /// Mark a subtask as running.
    pub fn start(&mut self, id: &str) -> Result<(), String> {
        let task = self.find_mut(id)?;
        if task.status != SwarmSubtaskStatus::Pending {
            return Err(format!("Subtask '{id}' not pending"));
        }
        task.status = SwarmSubtaskStatus::Running;
        Ok(())
    }

    /// Mark a subtask as completed with result.
    pub fn complete(&mut self, id: &str, result: String) -> Result<(), String> {
        let agent_id = {
            let task = self.find_mut(id)?;
            if task.status != SwarmSubtaskStatus::Running {
                return Err(format!("Subtask '{id}' not running"));
            }
            task.status = SwarmSubtaskStatus::Completed;
            task.result = Some(result);
            task.agent_id.clone()
        };
        self.file_ownership.release(&agent_id);
        Ok(())
    }

    /// Mark a subtask as failed.
    pub fn fail(&mut self, id: &str, error: String) -> Result<(), String> {
        let agent_id = {
            let task = self.find_mut(id)?;
            task.status = SwarmSubtaskStatus::Failed;
            task.result = Some(error);
            task.agent_id.clone()
        };
        self.file_ownership.release(&agent_id);
        Ok(())
    }

    /// Check if all subtasks are done (completed or failed).
    pub fn is_converged(&self) -> bool {
        self.subtasks.iter().all(|t| {
            matches!(
                t.status,
                SwarmSubtaskStatus::Completed | SwarmSubtaskStatus::Failed
            )
        })
    }

    /// Get summary of swarm progress.
    pub fn progress(&self) -> (usize, usize, usize, usize) {
        let pending = self
            .subtasks
            .iter()
            .filter(|t| t.status == SwarmSubtaskStatus::Pending)
            .count();
        let running = self
            .subtasks
            .iter()
            .filter(|t| t.status == SwarmSubtaskStatus::Running)
            .count();
        let completed = self
            .subtasks
            .iter()
            .filter(|t| t.status == SwarmSubtaskStatus::Completed)
            .count();
        let failed = self
            .subtasks
            .iter()
            .filter(|t| t.status == SwarmSubtaskStatus::Failed)
            .count();
        (pending, running, completed, failed)
    }

    /// Collect results from all completed subtasks.
    pub fn collect_results(&self) -> Vec<(&str, &str)> {
        self.subtasks
            .iter()
            .filter(|t| t.status == SwarmSubtaskStatus::Completed)
            .filter_map(|t| t.result.as_deref().map(|r| (t.agent_id.as_str(), r)))
            .collect()
    }

    fn find_mut(&mut self, id: &str) -> Result<&mut SwarmSubtask, String> {
        self.subtasks
            .iter_mut()
            .find(|t| t.id == id)
            .ok_or_else(|| format!("Subtask '{id}' not found"))
    }
}

// ---------------------------------------------------------------------------
// TeamMemory — shared memory across agents in a swarm
// ---------------------------------------------------------------------------

/// A single shared memory entry visible to all agents in a team.
#[derive(Debug, Clone)]
pub struct TeamMemoryEntry {
    pub key: String,
    pub value: String,
    pub author: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Thread-safe shared memory for swarm agents.
/// All agents can read/write; keyed by string, last-write-wins.
#[derive(Debug, Default)]
pub struct TeamMemory {
    entries: HashMap<String, TeamMemoryEntry>,
}

impl TeamMemory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Write a key-value pair. Overwrites if key exists.
    pub fn set(&mut self, key: &str, value: &str, author: &str) {
        self.entries.insert(
            key.to_string(),
            TeamMemoryEntry {
                key: key.to_string(),
                value: value.to_string(),
                author: author.to_string(),
                timestamp: chrono::Utc::now(),
            },
        );
    }

    /// Read a value by key.
    pub fn get(&self, key: &str) -> Option<&TeamMemoryEntry> {
        self.entries.get(key)
    }

    /// Remove a key.
    pub fn remove(&mut self, key: &str) -> bool {
        self.entries.remove(key).is_some()
    }

    /// List all keys.
    pub fn keys(&self) -> Vec<&str> {
        self.entries.keys().map(String::as_str).collect()
    }

    /// List all entries.
    pub fn entries(&self) -> Vec<&TeamMemoryEntry> {
        self.entries.values().collect()
    }

    /// Search entries by keyword in key or value.
    pub fn search(&self, query: &str) -> Vec<&TeamMemoryEntry> {
        let q = query.to_lowercase();
        self.entries
            .values()
            .filter(|e| e.key.to_lowercase().contains(&q) || e.value.to_lowercase().contains(&q))
            .collect()
    }

    /// Get entries written by a specific agent.
    pub fn by_author(&self, author: &str) -> Vec<&TeamMemoryEntry> {
        self.entries
            .values()
            .filter(|e| e.author == author)
            .collect()
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Format all entries for inclusion in an agent's context.
    pub fn format_for_prompt(&self) -> String {
        if self.entries.is_empty() {
            return String::new();
        }
        let mut out = String::from("# Team Shared Memory\n\n");
        let mut sorted: Vec<_> = self.entries.values().collect();
        sorted.sort_by_key(|e| &e.key);
        for entry in sorted {
            out.push_str(&format!(
                "- **{}** (by {}): {}\n",
                entry.key, entry.author, entry.value
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_ownership_claim_and_release() {
        let mut fo = FileOwnership::new();
        fo.claim("agent-1", &["a.rs".into(), "b.rs".into()])
            .unwrap();
        assert!(fo.is_owned_by("a.rs", "agent-1"));
        assert!(!fo.is_owned_by("a.rs", "agent-2"));
        assert!(fo.claim("agent-2", &["a.rs".into()]).is_err());
        fo.release("agent-1");
        fo.claim("agent-2", &["a.rs".into()]).unwrap();
        assert!(fo.is_owned_by("a.rs", "agent-2"));
    }

    #[test]
    fn file_ownership_files_for() {
        let mut fo = FileOwnership::new();
        fo.claim("a1", &["x.rs".into(), "y.rs".into()]).unwrap();
        let mut files = fo.files_for("a1");
        files.sort();
        assert_eq!(files, vec!["x.rs", "y.rs"]);
    }

    #[test]
    fn swarm_full_lifecycle() {
        let mut swarm = SwarmCoordinator::new("test");
        let id1 = swarm
            .add_subtask("w1", "fix module A", vec!["a.rs".into()])
            .unwrap();
        let id2 = swarm
            .add_subtask("w2", "fix module B", vec!["b.rs".into()])
            .unwrap();

        assert!(!swarm.is_converged());
        let (p, r, c, f) = swarm.progress();
        assert_eq!((p, r, c, f), (2, 0, 0, 0));

        swarm.start(&id1).unwrap();
        swarm.start(&id2).unwrap();
        let (p, r, _, _) = swarm.progress();
        assert_eq!((p, r), (0, 2));

        swarm.complete(&id1, "done A".into()).unwrap();
        swarm.fail(&id2, "error B".into()).unwrap();

        assert!(swarm.is_converged());
        let results = swarm.collect_results();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], ("w1", "done A"));
    }

    #[test]
    fn swarm_file_conflict_rejected() {
        let mut swarm = SwarmCoordinator::new("conflict");
        swarm
            .add_subtask("w1", "task 1", vec!["shared.rs".into()])
            .unwrap();
        assert!(
            swarm
                .add_subtask("w2", "task 2", vec!["shared.rs".into()])
                .is_err()
        );
    }

    #[test]
    fn swarm_files_released_on_complete() {
        let mut swarm = SwarmCoordinator::new("release");
        let id = swarm
            .add_subtask("w1", "task", vec!["f.rs".into()])
            .unwrap();
        swarm.start(&id).unwrap();
        swarm.complete(&id, "ok".into()).unwrap();
        // File should be released — another agent can claim it
        swarm
            .add_subtask("w2", "task 2", vec!["f.rs".into()])
            .unwrap();
    }

    // --- TeamMemory ---

    #[test]
    fn team_memory_set_and_get() {
        let mut tm = TeamMemory::new();
        tm.set("api_url", "https://example.com", "agent-1");
        let entry = tm.get("api_url").unwrap();
        assert_eq!(entry.value, "https://example.com");
        assert_eq!(entry.author, "agent-1");
    }

    #[test]
    fn team_memory_overwrite() {
        let mut tm = TeamMemory::new();
        tm.set("key", "v1", "a1");
        tm.set("key", "v2", "a2");
        assert_eq!(tm.get("key").unwrap().value, "v2");
        assert_eq!(tm.get("key").unwrap().author, "a2");
        assert_eq!(tm.len(), 1);
    }

    #[test]
    fn team_memory_remove() {
        let mut tm = TeamMemory::new();
        tm.set("temp", "data", "a1");
        assert!(tm.remove("temp"));
        assert!(tm.get("temp").is_none());
        assert!(!tm.remove("nonexistent"));
    }

    #[test]
    fn team_memory_search() {
        let mut tm = TeamMemory::new();
        tm.set("db_host", "postgres://localhost", "a1");
        tm.set("api_key", "sk-xxx", "a2");
        let results = tm.search("postgres");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].key, "db_host");
    }

    #[test]
    fn team_memory_by_author() {
        let mut tm = TeamMemory::new();
        tm.set("k1", "v1", "agent-1");
        tm.set("k2", "v2", "agent-2");
        tm.set("k3", "v3", "agent-1");
        let a1 = tm.by_author("agent-1");
        assert_eq!(a1.len(), 2);
    }

    #[test]
    fn team_memory_format_for_prompt() {
        let mut tm = TeamMemory::new();
        assert!(tm.format_for_prompt().is_empty());
        tm.set("finding", "SQLi in /api/login", "scanner");
        let prompt = tm.format_for_prompt();
        assert!(prompt.contains("Team Shared Memory"));
        assert!(prompt.contains("finding"));
        assert!(prompt.contains("SQLi"));
    }

    #[test]
    fn team_memory_keys_and_entries() {
        let mut tm = TeamMemory::new();
        tm.set("a", "1", "x");
        tm.set("b", "2", "y");
        assert_eq!(tm.keys().len(), 2);
        assert_eq!(tm.entries().len(), 2);
        assert!(!tm.is_empty());
    }
}
