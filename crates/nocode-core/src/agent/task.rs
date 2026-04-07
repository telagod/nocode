//! Task coordinator — manages task lifecycle for agent workflows.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// Task status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    Deleted,
}

/// A task in the coordinator.
#[derive(Debug, Clone)]
pub struct Task {
    pub id: String,
    pub subject: String,
    pub description: String,
    pub status: TaskStatus,
    pub owner: Option<String>,
    pub blocked_by: Vec<String>,
    pub blocks: Vec<String>,
}

impl Task {
    pub fn new(id: &str, subject: &str, description: &str) -> Self {
        Self {
            id: id.to_string(),
            subject: subject.to_string(),
            description: description.to_string(),
            status: TaskStatus::Pending,
            owner: None,
            blocked_by: Vec::new(),
            blocks: Vec::new(),
        }
    }

    pub fn is_blocked(&self) -> bool {
        !self.blocked_by.is_empty()
    }
}

/// Coordinates tasks across agents.
pub struct TaskCoordinator {
    tasks: HashMap<String, Task>,
    next_id: u64,
}

impl TaskCoordinator {
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
            next_id: 1,
        }
    }

    /// Create a new task, returning its ID.
    pub fn create(&mut self, subject: &str, description: &str) -> String {
        let id = format!("{}", self.next_id);
        self.next_id += 1;
        let task = Task::new(&id, subject, description);
        self.tasks.insert(id.clone(), task);
        id
    }

    /// Get a task by ID.
    pub fn get(&self, id: &str) -> Option<&Task> {
        self.tasks.get(id)
    }

    /// Get a mutable task by ID.
    pub fn get_mut(&mut self, id: &str) -> Option<&mut Task> {
        self.tasks.get_mut(id)
    }

    /// List all non-deleted tasks.
    pub fn list(&self) -> Vec<&Task> {
        self.tasks.values()
            .filter(|t| t.status != TaskStatus::Deleted)
            .collect()
    }

    /// Update task status.
    pub fn set_status(&mut self, id: &str, status: TaskStatus) -> Result<(), String> {
        let task = self.tasks.get_mut(id)
            .ok_or_else(|| format!("task {id} not found"))?;
        task.status = status;

        // If completed/deleted, unblock dependents
        if matches!(status, TaskStatus::Completed | TaskStatus::Deleted) {
            let task_id = id.to_string();
            for t in self.tasks.values_mut() {
                t.blocked_by.retain(|b| b != &task_id);
            }
        }
        Ok(())
    }

    /// Set task owner.
    pub fn set_owner(&mut self, id: &str, owner: &str) -> Result<(), String> {
        let task = self.tasks.get_mut(id)
            .ok_or_else(|| format!("task {id} not found"))?;
        task.owner = Some(owner.to_string());
        Ok(())
    }

    /// Add a blocking dependency: `id` is blocked by `blocker_id`.
    pub fn add_blocked_by(&mut self, id: &str, blocker_id: &str) -> Result<(), String> {
        if !self.tasks.contains_key(blocker_id) {
            return Err(format!("blocker task {blocker_id} not found"));
        }
        let task = self.tasks.get_mut(id)
            .ok_or_else(|| format!("task {id} not found"))?;
        if !task.blocked_by.contains(&blocker_id.to_string()) {
            task.blocked_by.push(blocker_id.to_string());
        }
        Ok(())
    }

    /// Add a blocks relationship: `id` blocks `blocked_id`.
    pub fn add_blocks(&mut self, id: &str, blocked_id: &str) -> Result<(), String> {
        if !self.tasks.contains_key(blocked_id) {
            return Err(format!("blocked task {blocked_id} not found"));
        }
        let task = self.tasks.get_mut(id)
            .ok_or_else(|| format!("task {id} not found"))?;
        if !task.blocks.contains(&blocked_id.to_string()) {
            task.blocks.push(blocked_id.to_string());
        }
        // Also add reverse relationship
        self.add_blocked_by(blocked_id, id)
    }

    /// Delete a task.
    pub fn delete(&mut self, id: &str) -> Result<(), String> {
        self.set_status(id, TaskStatus::Deleted)
    }
}

impl Default for TaskCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

/// Global singleton task coordinator.
static GLOBAL_TASK_COORDINATOR: OnceLock<Arc<Mutex<TaskCoordinator>>> = OnceLock::new();

pub fn global_task_coordinator() -> &'static Arc<Mutex<TaskCoordinator>> {
    GLOBAL_TASK_COORDINATOR.get_or_init(|| Arc::new(Mutex::new(TaskCoordinator::new())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_list() {
        let mut tc = TaskCoordinator::new();
        let id = tc.create("Build feature", "Implement the new feature");
        assert_eq!(tc.list().len(), 1);
        assert_eq!(tc.get(&id).unwrap().subject, "Build feature");
        assert_eq!(tc.get(&id).unwrap().status, TaskStatus::Pending);
    }

    #[test]
    fn status_transitions() {
        let mut tc = TaskCoordinator::new();
        let id = tc.create("Test task", "Run tests");
        tc.set_status(&id, TaskStatus::InProgress).unwrap();
        assert_eq!(tc.get(&id).unwrap().status, TaskStatus::InProgress);
        tc.set_status(&id, TaskStatus::Completed).unwrap();
        assert_eq!(tc.get(&id).unwrap().status, TaskStatus::Completed);
    }

    #[test]
    fn blocking_dependencies() {
        let mut tc = TaskCoordinator::new();
        let id1 = tc.create("First", "Do first");
        let id2 = tc.create("Second", "Do second");
        tc.add_blocked_by(&id2, &id1).unwrap();
        assert!(tc.get(&id2).unwrap().is_blocked());

        // Completing id1 unblocks id2
        tc.set_status(&id1, TaskStatus::Completed).unwrap();
        assert!(!tc.get(&id2).unwrap().is_blocked());
    }

    #[test]
    fn delete_unblocks_dependents() {
        let mut tc = TaskCoordinator::new();
        let id1 = tc.create("Blocker", "Blocks others");
        let id2 = tc.create("Blocked", "Waiting");
        tc.add_blocked_by(&id2, &id1).unwrap();
        assert!(tc.get(&id2).unwrap().is_blocked());
        tc.delete(&id1).unwrap();
        assert!(!tc.get(&id2).unwrap().is_blocked());
    }

    #[test]
    fn set_owner() {
        let mut tc = TaskCoordinator::new();
        let id = tc.create("Owned task", "Has an owner");
        tc.set_owner(&id, "agent-1").unwrap();
        assert_eq!(tc.get(&id).unwrap().owner.as_deref(), Some("agent-1"));
    }

    #[test]
    fn nonexistent_task_errors() {
        let mut tc = TaskCoordinator::new();
        assert!(tc.set_status("999", TaskStatus::Completed).is_err());
        assert!(tc.set_owner("999", "agent").is_err());
    }
}
