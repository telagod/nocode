use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerStatus {
    Spawning,
    TrustRequired,
    ReadyForPrompt,
    Running,
    Finished,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerFailureKind {
    TrustGate,
    PromptDelivery,
    Protocol,
    Provider,
}

#[derive(Debug, Clone)]
pub struct WorkerEvent {
    pub seq: u64,
    pub kind: WorkerEventKind,
    pub status: WorkerStatus,
    pub detail: Option<String>,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerEventKind {
    Spawning,
    TrustRequired,
    TrustResolved,
    ReadyForPrompt,
    PromptMisdelivery,
    Running,
    Finished,
    Failed { kind: WorkerFailureKind },
}

#[derive(Debug, Clone)]
pub struct WorkerFailure {
    pub kind: WorkerFailureKind,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct Worker {
    pub id: String,
    pub status: WorkerStatus,
    pub events: Vec<WorkerEvent>,
    pub failure: Option<WorkerFailure>,
    next_seq: u64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

impl Worker {
    pub fn new(id: &str) -> Self {
        let mut w = Self {
            id: id.to_string(),
            status: WorkerStatus::Spawning,
            events: Vec::new(),
            failure: None,
            next_seq: 0,
        };
        w.emit_event(WorkerEventKind::Spawning);
        w
    }

    pub fn require_trust(&mut self) {
        self.status = WorkerStatus::TrustRequired;
        self.emit_event(WorkerEventKind::TrustRequired);
    }

    pub fn resolve_trust(&mut self) {
        self.status = WorkerStatus::ReadyForPrompt;
        self.emit_event(WorkerEventKind::TrustResolved);
    }

    pub fn mark_ready(&mut self) {
        self.status = WorkerStatus::ReadyForPrompt;
        self.emit_event(WorkerEventKind::ReadyForPrompt);
    }

    pub fn start_running(&mut self) {
        self.status = WorkerStatus::Running;
        self.emit_event(WorkerEventKind::Running);
    }

    pub fn finish(&mut self) {
        self.status = WorkerStatus::Finished;
        self.emit_event(WorkerEventKind::Finished);
    }

    pub fn fail(&mut self, kind: WorkerFailureKind, message: &str) {
        self.status = WorkerStatus::Failed;
        self.failure = Some(WorkerFailure {
            kind,
            message: message.to_string(),
        });
        self.emit_event(WorkerEventKind::Failed { kind });
    }

    pub fn emit_event(&mut self, kind: WorkerEventKind) {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.events.push(WorkerEvent {
            seq,
            kind,
            status: self.status,
            detail: None,
            timestamp_ms: now_ms(),
        });
    }
}

pub struct WorkerRegistry {
    workers: HashMap<String, Worker>,
}

impl WorkerRegistry {
    pub fn new() -> Self {
        Self {
            workers: HashMap::new(),
        }
    }

    pub fn create(&mut self, id: &str) -> &Worker {
        self.workers
            .entry(id.to_string())
            .or_insert_with(|| Worker::new(id))
    }

    pub fn get(&self, id: &str) -> Option<&Worker> {
        self.workers.get(id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Worker> {
        self.workers.get_mut(id)
    }

    pub fn list(&self) -> Vec<&Worker> {
        self.workers.values().collect()
    }

    pub fn remove(&mut self, id: &str) -> Option<Worker> {
        self.workers.remove(id)
    }
}

impl Default for WorkerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

static WORKER_REGISTRY: OnceLock<Arc<Mutex<WorkerRegistry>>> = OnceLock::new();

pub fn global_worker_registry() -> Arc<Mutex<WorkerRegistry>> {
    WORKER_REGISTRY
        .get_or_init(|| Arc::new(Mutex::new(WorkerRegistry::new())))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_lifecycle_spawning_to_finished() {
        let mut w = Worker::new("w1");
        assert_eq!(w.status, WorkerStatus::Spawning);

        w.mark_ready();
        assert_eq!(w.status, WorkerStatus::ReadyForPrompt);

        w.start_running();
        assert_eq!(w.status, WorkerStatus::Running);

        w.finish();
        assert_eq!(w.status, WorkerStatus::Finished);
        assert!(w.failure.is_none());
    }

    #[test]
    fn worker_failure_records_event() {
        let mut w = Worker::new("w2");
        w.fail(WorkerFailureKind::Provider, "timeout");

        assert_eq!(w.status, WorkerStatus::Failed);
        let f = w.failure.as_ref().unwrap();
        assert_eq!(f.kind, WorkerFailureKind::Provider);
        assert_eq!(f.message, "timeout");

        let last = w.events.last().unwrap();
        assert_eq!(last.kind, WorkerEventKind::Failed { kind: WorkerFailureKind::Provider });
        assert_eq!(last.status, WorkerStatus::Failed);
    }

    #[test]
    fn trust_gate_flow() {
        let mut w = Worker::new("w3");
        w.require_trust();
        assert_eq!(w.status, WorkerStatus::TrustRequired);

        w.resolve_trust();
        assert_eq!(w.status, WorkerStatus::ReadyForPrompt);

        let kinds: Vec<_> = w.events.iter().map(|e| e.kind.clone()).collect();
        assert_eq!(kinds, vec![
            WorkerEventKind::Spawning,
            WorkerEventKind::TrustRequired,
            WorkerEventKind::TrustResolved,
        ]);
    }

    #[test]
    fn registry_create_and_get() {
        let mut reg = WorkerRegistry::new();
        reg.create("a");
        reg.create("b");

        assert!(reg.get("a").is_some());
        assert!(reg.get("b").is_some());
        assert!(reg.get("c").is_none());
        assert_eq!(reg.list().len(), 2);
    }

    #[test]
    fn registry_remove() {
        let mut reg = WorkerRegistry::new();
        reg.create("x");
        assert!(reg.get("x").is_some());

        let removed = reg.remove("x");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().id, "x");
        assert!(reg.get("x").is_none());
    }

    #[test]
    fn events_have_sequential_ids() {
        let mut w = Worker::new("seq");
        w.mark_ready();
        w.start_running();
        w.finish();

        let seqs: Vec<u64> = w.events.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, vec![0, 1, 2, 3]);
    }
}
