use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub display: String,
    pub timestamp: u64,
    pub session_id: String,
    pub project: String,
}

impl HistoryEntry {
    pub fn new(
        session_id: impl Into<String>,
        project: impl Into<String>,
        display: impl Into<String>,
    ) -> Self {
        Self {
            display: display.into(),
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_millis() as u64)
                .unwrap_or_default(),
            session_id: session_id.into(),
            project: project.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryStoreConfig {
    pub persist_history: bool,
    pub project: String,
}

impl HistoryStoreConfig {
    pub fn new(persist_history: bool, project: impl Into<String>) -> Self {
        Self {
            persist_history,
            project: project.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryStorePlan {
    pub session_id: String,
    pub persist_history: bool,
    pub pending_entries: usize,
    pub flush_count: u32,
    pub entries: Vec<HistoryEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryStore {
    pub config: HistoryStoreConfig,
    pub pending_entries: Vec<HistoryEntry>,
    pub flush_count: u32,
}

impl HistoryStore {
    pub fn new(config: HistoryStoreConfig) -> Self {
        Self {
            config,
            pending_entries: Vec::new(),
            flush_count: 0,
        }
    }

    pub fn record_entry(&mut self, entry: HistoryEntry) {
        if self.config.persist_history && entry.project == self.config.project {
            self.pending_entries.push(entry);
        }
    }

    pub fn pending_count(&self) -> usize {
        self.pending_entries.len()
    }

    pub fn record_submit(&mut self, session_id: impl Into<String>) -> HistoryStorePlan {
        let session_id = session_id.into();
        if self.config.persist_history {
            self.flush_count += 1;
        }
        let plan = HistoryStorePlan {
            session_id: session_id.clone(),
            persist_history: self.config.persist_history,
            pending_entries: self.pending_entries.len(),
            flush_count: self.flush_count,
            entries: self.pending_entries.clone(),
        };
        self.pending_entries.clear();
        plan
    }
}

#[cfg(test)]
mod tests {
    use super::{HistoryEntry, HistoryStore, HistoryStoreConfig};

    #[test]
    fn pending_entries_respect_project_and_persistence() {
        let mut store = HistoryStore::new(HistoryStoreConfig::new(true, "/tmp/project"));
        store.record_entry(HistoryEntry::new("session-1", "/tmp/project", "first"));
        store.record_entry(HistoryEntry::new("session-2", "/tmp/other", "should skip"));

        assert_eq!(store.pending_count(), 1);
    }

    #[test]
    fn record_submit_clears_pending_and_increments_flush() {
        let mut store = HistoryStore::new(HistoryStoreConfig::new(true, "/tmp/project"));
        store.record_entry(HistoryEntry::new("session-1", "/tmp/project", "one"));
        let plan = store.record_submit("session-1");

        assert!(plan.persist_history);
        assert_eq!(plan.pending_entries, 1);
        assert_eq!(plan.flush_count, 1);
        assert_eq!(plan.entries.len(), 1);
        assert_eq!(store.pending_count(), 0);

        let plan2 = store.record_submit("session-1");
        assert_eq!(plan2.flush_count, 2);
    }

    #[test]
    fn disabled_persistence_never_accumulates_entries() {
        let mut store = HistoryStore::new(HistoryStoreConfig::new(false, "/tmp/project"));
        store.record_entry(HistoryEntry::new("session-1", "/tmp/project", "one"));
        assert_eq!(store.pending_count(), 0);
        let plan = store.record_submit("session-1");
        assert!(!plan.persist_history);
        assert_eq!(plan.pending_entries, 0);
        assert_eq!(plan.flush_count, 0);
        assert!(plan.entries.is_empty());
    }
}
