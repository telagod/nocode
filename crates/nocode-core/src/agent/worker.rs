//! Worker registry — manages background agent workers.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Worker lifecycle states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerState {
    Spawning,
    TrustRequired,
    ReadyForPrompt,
    Running,
    Finished,
    Failed,
}

impl WorkerState {
    /// Check if transitioning to `target` is valid.
    pub fn can_transition_to(self, target: Self) -> bool {
        matches!(
            (self, target),
            (
                Self::Spawning,
                Self::TrustRequired | Self::ReadyForPrompt | Self::Failed
            ) | (Self::TrustRequired, Self::ReadyForPrompt | Self::Failed)
                | (Self::ReadyForPrompt, Self::Running | Self::Failed)
                | (Self::Running, Self::Finished | Self::Failed)
        )
    }
}

/// A background worker executing an agent task.
pub struct Worker {
    pub id: String,
    pub name: String,
    pub state: WorkerState,
    pub prompt: String,
    pub result: Option<String>,
    pub error: Option<String>,
    /// Inbox for inter-agent messages.
    pub inbox: Vec<AgentMessage>,
    /// Cancel token — set to true to request cancellation.
    pub cancel_token: Arc<AtomicBool>,
    /// Timeout in seconds (0 = no timeout).
    pub timeout_secs: u64,
    /// When the worker started running.
    pub started_at: Option<std::time::Instant>,
}

/// A message sent between agents.
#[derive(Debug, Clone)]
pub struct AgentMessage {
    pub from: String,
    pub content: serde_json::Value,
    pub timestamp: chrono::DateTime<chrono::Utc>,
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
            inbox: Vec::new(),
            cancel_token: Arc::new(AtomicBool::new(false)),
            timeout_secs: 0,
            started_at: None,
        }
    }

    /// Request cancellation of this worker.
    pub fn cancel(&self) {
        self.cancel_token.store(true, Ordering::Relaxed);
    }

    /// Check if cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancel_token.load(Ordering::Relaxed)
    }

    /// Check if the worker has timed out.
    pub fn is_timed_out(&self) -> bool {
        if self.timeout_secs == 0 {
            return false;
        }
        self.started_at
            .is_some_and(|t| t.elapsed().as_secs() >= self.timeout_secs)
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

    /// Update worker state (validates transition).
    pub fn set_state(&mut self, id: &str, state: WorkerState) {
        if let Some(w) = self.workers.get_mut(id)
            && w.state.can_transition_to(state)
        {
            w.state = state;
        }
    }

    /// Set worker result on completion (only valid from Running state).
    pub fn set_result(&mut self, id: &str, result: String) {
        if let Some(w) = self.workers.get_mut(id)
            && w.state == WorkerState::Running
        {
            w.result = Some(result);
            w.state = WorkerState::Finished;
        }
    }

    /// Set worker error on failure (valid from any non-terminal state).
    pub fn set_error(&mut self, id: &str, error: String) {
        if let Some(w) = self.workers.get_mut(id)
            && !matches!(w.state, WorkerState::Finished | WorkerState::Failed)
        {
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

    /// Find a worker by name (returns first match).
    pub fn find_by_name(&self, name: &str) -> Option<&Worker> {
        self.workers.values().find(|w| w.name == name)
    }

    /// Send a message to a worker by ID or name.
    pub fn send_message(
        &mut self,
        to: &str,
        from: &str,
        content: serde_json::Value,
    ) -> Result<(), String> {
        // Resolve target: try by ID first, then by name
        let key = if self.workers.contains_key(to) {
            to.to_string()
        } else {
            self.workers
                .values()
                .find(|w| w.name == to)
                .map(|w| w.id.clone())
                .ok_or_else(|| format!("Worker '{to}' not found"))?
        };

        let worker = self.workers.get_mut(&key).unwrap();
        worker.inbox.push(AgentMessage {
            from: from.to_string(),
            content,
            timestamp: chrono::Utc::now(),
        });
        Ok(())
    }

    /// Drain all messages from a worker's inbox.
    pub fn drain_inbox(&mut self, id: &str) -> Vec<AgentMessage> {
        self.workers
            .get_mut(id)
            .map(|w| std::mem::take(&mut w.inbox))
            .unwrap_or_default()
    }

    /// Get a clone of the worker's cancel token (for passing to background thread).
    pub fn get_cancel_token(&self, id: &str) -> Option<Arc<AtomicBool>> {
        self.workers.get(id).map(|w| Arc::clone(&w.cancel_token))
    }

    /// Request cancellation of a worker.
    pub fn cancel_worker(&mut self, id: &str) {
        if let Some(w) = self.workers.get(id) {
            w.cancel();
        }
    }

    /// Set timeout for a worker (in seconds).
    pub fn set_timeout(&mut self, id: &str, timeout_secs: u64) {
        if let Some(w) = self.workers.get_mut(id) {
            w.timeout_secs = timeout_secs;
        }
    }

    /// Mark a worker as started (records start time).
    pub fn mark_started(&mut self, id: &str) {
        if let Some(w) = self.workers.get_mut(id) {
            w.started_at = Some(std::time::Instant::now());
        }
    }

    /// Check all running workers for timeouts, cancel any that exceeded their limit.
    pub fn check_timeouts(&mut self) -> Vec<String> {
        let timed_out: Vec<String> = self
            .workers
            .values()
            .filter(|w| w.state == WorkerState::Running && w.is_timed_out())
            .map(|w| w.id.clone())
            .collect();

        for id in &timed_out {
            if let Some(w) = self.workers.get(id) {
                w.cancel();
            }
            self.set_error(id, "worker timed out".to_string());
        }

        timed_out
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
        reg.set_state(&id, WorkerState::ReadyForPrompt);
        assert_eq!(reg.get(&id).unwrap().state, WorkerState::ReadyForPrompt);
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
