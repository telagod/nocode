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
}
