//! Worker registry — manages background agent workers.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// Worker lifecycle states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerState {
    Spawning,
    Running,
    Finished,
    Failed,
}

/// A background worker executing an agent task.
#[derive(Debug)]
pub struct Worker {
    pub id: String,
    pub name: String,
    pub state: WorkerState,
    pub prompt: String,
    pub result: Option<String>,
    pub error: Option<String>,
}

impl Worker {
    pub fn new(id: &str, name: &str, prompt: &str) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            state: WorkerState::Spawning,
            prompt: prompt.to_string(),
            result: None,
            error: None,
        }
    }
}

/// Registry tracking all background workers.
pub struct WorkerRegistry {
    workers: HashMap<String, Worker>,
    next_id: u64,
}

impl WorkerRegistry {
    pub fn new() -> Self {
        Self {
            workers: HashMap::new(),
            next_id: 1,
        }
    }

    /// Register a new worker, returning its ID.
    pub fn register(&mut self, name: &str, prompt: &str) -> String {
        let id = format!("worker-{}", self.next_id);
        self.next_id += 1;
        let worker = Worker::new(&id, name, prompt);
        self.workers.insert(id.clone(), worker);
        id
    }

    /// Update worker state.
    pub fn set_state(&mut self, id: &str, state: WorkerState) {
        if let Some(w) = self.workers.get_mut(id) {
            w.state = state;
        }
    }

    /// Set worker result on completion.
    pub fn set_result(&mut self, id: &str, result: String) {
        if let Some(w) = self.workers.get_mut(id) {
            w.result = Some(result);
            w.state = WorkerState::Finished;
        }
    }

    /// Set worker error on failure.
    pub fn set_error(&mut self, id: &str, error: String) {
        if let Some(w) = self.workers.get_mut(id) {
            w.error = Some(error);
            w.state = WorkerState::Failed;
        }
    }

    /// Get a worker by ID.
    pub fn get(&self, id: &str) -> Option<&Worker> {
        self.workers.get(id)
    }

    /// List all workers.
    pub fn list(&self) -> Vec<&Worker> {
        self.workers.values().collect()
    }

    /// Remove a finished/failed worker.
    pub fn remove(&mut self, id: &str) -> Option<Worker> {
        self.workers.remove(id)
    }
}

impl Default for WorkerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Global singleton worker registry.
static GLOBAL_WORKER_REGISTRY: OnceLock<Arc<Mutex<WorkerRegistry>>> = OnceLock::new();

pub fn global_worker_registry() -> &'static Arc<Mutex<WorkerRegistry>> {
    GLOBAL_WORKER_REGISTRY.get_or_init(|| Arc::new(Mutex::new(WorkerRegistry::new())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_list() {
        let mut reg = WorkerRegistry::new();
        let id = reg.register("explorer", "find all rust files");
        assert!(id.starts_with("worker-"));
        assert_eq!(reg.list().len(), 1);
        assert_eq!(reg.get(&id).unwrap().state, WorkerState::Spawning);
    }

    #[test]
    fn lifecycle_transitions() {
        let mut reg = WorkerRegistry::new();
        let id = reg.register("builder", "build project");
        reg.set_state(&id, WorkerState::Running);
        assert_eq!(reg.get(&id).unwrap().state, WorkerState::Running);
        reg.set_result(&id, "build succeeded".to_string());
        assert_eq!(reg.get(&id).unwrap().state, WorkerState::Finished);
        assert_eq!(
            reg.get(&id).unwrap().result.as_deref(),
            Some("build succeeded")
        );
    }

    #[test]
    fn error_state() {
        let mut reg = WorkerRegistry::new();
        let id = reg.register("tester", "run tests");
        reg.set_error(&id, "compilation failed".to_string());
        assert_eq!(reg.get(&id).unwrap().state, WorkerState::Failed);
        assert!(
            reg.get(&id)
                .unwrap()
                .error
                .as_ref()
                .unwrap()
                .contains("compilation")
        );
    }

    #[test]
    fn remove_worker() {
        let mut reg = WorkerRegistry::new();
        let id = reg.register("temp", "temporary task");
        assert_eq!(reg.list().len(), 1);
        let removed = reg.remove(&id);
        assert!(removed.is_some());
        assert_eq!(reg.list().len(), 0);
    }
}
