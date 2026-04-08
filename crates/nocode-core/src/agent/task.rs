//! Task coordinator — manages task lifecycle for agent workflows.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// Task lifecycle event types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskEventKind {
    Created,
    StatusChanged { from: TaskStatus, to: TaskStatus },
    OwnerChanged { owner: String },
    Blocked { by: String },
    Unblocked { by: String },
}

/// A recorded lifecycle event for a task.
#[derive(Debug, Clone)]
pub struct TaskEvent {
    pub task_id: String,
    pub kind: TaskEventKind,
    pub timestamp: std::time::SystemTime,
}

/// Task status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    Deleted,
}

impl TaskStatus {
    /// Check if transitioning to `target` is valid.
    pub fn can_transition_to(self, target: Self) -> bool {
        matches!(
            (self, target),
            // Pending can start, be deleted, or fail
            (Self::Pending, Self::InProgress | Self::Deleted | Self::Failed)
            // InProgress can complete, fail, or be deleted
            | (Self::InProgress, Self::Completed | Self::Failed | Self::Deleted)
            // Failed can retry (back to InProgress) or be deleted
            | (Self::Failed, Self::InProgress | Self::Deleted)
        )
    }
}

/// A task in the coordinator.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Audit trail of task lifecycle events.
    pub events: Vec<TaskEvent>,
}

impl TaskCoordinator {
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
            next_id: 1,
            events: Vec::new(),
        }
    }

    fn record(&mut self, task_id: &str, kind: TaskEventKind) {
        self.events.push(TaskEvent {
            task_id: task_id.to_string(),
            kind,
            timestamp: std::time::SystemTime::now(),
        });
    }

    /// Create a new task, returning its ID.
    pub fn create(&mut self, subject: &str, description: &str) -> String {
        let id = format!("{}", self.next_id);
        self.next_id += 1;
        let task = Task::new(&id, subject, description);
        self.tasks.insert(id.clone(), task);
        self.record(&id, TaskEventKind::Created);
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
        self.tasks
            .values()
            .filter(|t| t.status != TaskStatus::Deleted)
            .collect()
    }

    /// Update task status (validates transition).
    pub fn set_status(&mut self, id: &str, status: TaskStatus) -> Result<(), String> {
        let task = self
            .tasks
            .get_mut(id)
            .ok_or_else(|| format!("task {id} not found"))?;

        if !task.status.can_transition_to(status) {
            return Err(format!(
                "invalid transition: {:?} → {:?} for task {id}",
                task.status, status
            ));
        }

        let from = task.status;
        task.status = status;

        // Record audit event
        self.record(id, TaskEventKind::StatusChanged { from, to: status });

        // If completed/deleted, unblock dependents
        if matches!(status, TaskStatus::Completed | TaskStatus::Deleted) {
            let task_id = id.to_string();
            let unblocked: Vec<String> = self
                .tasks
                .values()
                .filter(|t| t.blocked_by.contains(&task_id))
                .map(|t| t.id.clone())
                .collect();
            for t in self.tasks.values_mut() {
                t.blocked_by.retain(|b| b != &task_id);
            }
            for uid in &unblocked {
                self.record(
                    uid,
                    TaskEventKind::Unblocked {
                        by: task_id.clone(),
                    },
                );
            }
        }
        Ok(())
    }

    /// Set task owner.
    pub fn set_owner(&mut self, id: &str, owner: &str) -> Result<(), String> {
        let task = self
            .tasks
            .get_mut(id)
            .ok_or_else(|| format!("task {id} not found"))?;
        task.owner = Some(owner.to_string());
        self.record(
            id,
            TaskEventKind::OwnerChanged {
                owner: owner.to_string(),
            },
        );
        Ok(())
    }

    /// Add a blocking dependency: `id` is blocked by `blocker_id`.
    pub fn add_blocked_by(&mut self, id: &str, blocker_id: &str) -> Result<(), String> {
        if !self.tasks.contains_key(blocker_id) {
            return Err(format!("blocker task {blocker_id} not found"));
        }
        let task = self
            .tasks
            .get_mut(id)
            .ok_or_else(|| format!("task {id} not found"))?;
        if !task.blocked_by.contains(&blocker_id.to_string()) {
            task.blocked_by.push(blocker_id.to_string());
            self.record(
                id,
                TaskEventKind::Blocked {
                    by: blocker_id.to_string(),
                },
            );
        }
        Ok(())
    }

    /// Add a blocks relationship: `id` blocks `blocked_id`.
    pub fn add_blocks(&mut self, id: &str, blocked_id: &str) -> Result<(), String> {
        if !self.tasks.contains_key(blocked_id) {
            return Err(format!("blocked task {blocked_id} not found"));
        }
        let task = self
            .tasks
            .get_mut(id)
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

    /// Clear all tasks (used by TodoWrite to replace the entire list).
    pub fn clear(&mut self) {
        self.tasks.clear();
        self.events.clear();
    }

    /// Get audit events for a specific task, in chronological order.
    pub fn events_for_task(&self, task_id: &str) -> Vec<&TaskEvent> {
        self.events
            .iter()
            .filter(|e| e.task_id == task_id)
            .collect()
    }

    /// Persist all non-deleted tasks to a JSONL file.
    pub fn persist_to_file(&self, path: &str) -> Result<(), String> {
        use std::io::Write;
        let tasks: Vec<&Task> = self
            .tasks
            .values()
            .filter(|t| t.status != TaskStatus::Deleted)
            .collect();
        let file =
            std::fs::File::create(path).map_err(|e| format!("Failed to create task file: {e}"))?;
        let mut writer = std::io::BufWriter::new(file);
        for task in tasks {
            let json = serde_json::to_string(task)
                .map_err(|e| format!("Failed to serialize task: {e}"))?;
            writeln!(writer, "{json}").map_err(|e| format!("Failed to write task: {e}"))?;
        }
        Ok(())
    }

    /// Load tasks from a JSONL file, replacing current state.
    pub fn load_from_file(&mut self, path: &str) -> Result<usize, String> {
        use std::io::BufRead;
        let file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(format!("Failed to open task file: {e}")),
        };
        let reader = std::io::BufReader::new(file);
        self.tasks.clear();
        self.events.clear();
        let mut max_id: u64 = 0;
        let mut count = 0;
        for line in reader.lines() {
            let line = line.map_err(|e| format!("Failed to read line: {e}"))?;
            if line.trim().is_empty() {
                continue;
            }
            let task: Task =
                serde_json::from_str(&line).map_err(|e| format!("Failed to parse task: {e}"))?;
            if let Ok(n) = task.id.parse::<u64>() {
                max_id = max_id.max(n);
            }
            self.tasks.insert(task.id.clone(), task);
            count += 1;
        }
        self.next_id = max_id + 1;
        Ok(count)
    }

    /// Default persistence path under .nocode/
    pub fn default_path(cwd: &str) -> String {
        format!("{cwd}/.nocode/tasks.jsonl")
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

        // Completing id1 unblocks id2 (must go Pending → InProgress → Completed)
        tc.set_status(&id1, TaskStatus::InProgress).unwrap();
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

    #[test]
    fn valid_transitions() {
        assert!(TaskStatus::Pending.can_transition_to(TaskStatus::InProgress));
        assert!(TaskStatus::Pending.can_transition_to(TaskStatus::Deleted));
        assert!(TaskStatus::Pending.can_transition_to(TaskStatus::Failed));
        assert!(TaskStatus::InProgress.can_transition_to(TaskStatus::Completed));
        assert!(TaskStatus::InProgress.can_transition_to(TaskStatus::Failed));
        assert!(TaskStatus::InProgress.can_transition_to(TaskStatus::Deleted));
        assert!(TaskStatus::Failed.can_transition_to(TaskStatus::InProgress));
        assert!(TaskStatus::Failed.can_transition_to(TaskStatus::Deleted));
    }

    #[test]
    fn invalid_transitions() {
        assert!(!TaskStatus::Pending.can_transition_to(TaskStatus::Completed));
        assert!(!TaskStatus::Completed.can_transition_to(TaskStatus::InProgress));
        assert!(!TaskStatus::Completed.can_transition_to(TaskStatus::Pending));
        assert!(!TaskStatus::Deleted.can_transition_to(TaskStatus::Pending));
    }

    #[test]
    fn set_status_rejects_invalid_transition() {
        let mut tc = TaskCoordinator::new();
        let id = tc.create("Test", "desc");
        // Pending → Completed is invalid (must go through InProgress)
        let result = tc.set_status(&id, TaskStatus::Completed);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid transition"));
    }

    #[test]
    fn persist_and_load_roundtrip() {
        let dir = std::env::temp_dir().join("nocode_task_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("tasks.jsonl");
        let path_str = path.to_string_lossy().to_string();

        // Create tasks and persist
        let mut tc = TaskCoordinator::new();
        let id1 = tc.create("Task A", "First task");
        let id2 = tc.create("Task B", "Second task");
        tc.set_status(&id1, TaskStatus::InProgress).unwrap();
        tc.set_status(&id1, TaskStatus::Completed).unwrap();
        tc.set_owner(&id2, "agent-1").unwrap();
        tc.persist_to_file(&path_str).unwrap();

        // Load into fresh coordinator
        let mut tc2 = TaskCoordinator::new();
        let count = tc2.load_from_file(&path_str).unwrap();
        assert_eq!(count, 2);
        assert_eq!(tc2.get(&id1).unwrap().status, TaskStatus::Completed);
        assert_eq!(tc2.get(&id2).unwrap().owner.as_deref(), Some("agent-1"));
        // next_id should be past loaded IDs
        let id3 = tc2.create("Task C", "Third");
        assert_eq!(id3, "3");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_nonexistent_file_returns_zero() {
        let mut tc = TaskCoordinator::new();
        let count = tc
            .load_from_file("/tmp/nocode_nonexistent_tasks.jsonl")
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn audit_trail_records_events() {
        let mut tc = TaskCoordinator::new();
        let id = tc.create("Audited", "desc");
        tc.set_status(&id, TaskStatus::InProgress).unwrap();
        tc.set_owner(&id, "worker-1").unwrap();

        let events = tc.events_for_task(&id);
        assert_eq!(events.len(), 3); // Created + StatusChanged + OwnerChanged
        assert_eq!(events[0].kind, TaskEventKind::Created);
        assert!(matches!(
            events[1].kind,
            TaskEventKind::StatusChanged { .. }
        ));
        assert!(matches!(events[2].kind, TaskEventKind::OwnerChanged { .. }));
    }
}
