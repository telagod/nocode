use crate::file_history::FileHistoryPlan;
use crate::history_store::HistoryStorePlan;
use crate::transcript::TranscriptEntry;
use std::fs;
use std::path::Path;

const HISTORY_FILE: &str = ".nocode/history.jsonl";
const FILE_HISTORY_DIR: &str = ".nocode/file-history";
const SESSION_DIR: &str = ".nocode/sessions";
const TASK_DIR: &str = ".nocode/tasks";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionIdentity {
    pub session_id: String,
    pub project_root: String,
}

impl SessionIdentity {
    pub fn new(session_id: impl Into<String>, project_root: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            project_root: project_root.into(),
        }
    }

    pub fn transcript_path(&self) -> String {
        Path::new(self.project_root.as_str())
            .join(SESSION_DIR)
            .join(format!("{}.jsonl", self.session_id))
            .to_string_lossy()
            .into_owned()
    }

    pub fn history_path(&self) -> String {
        Path::new(self.project_root.as_str())
            .join(HISTORY_FILE)
            .to_string_lossy()
            .into_owned()
    }

    pub fn file_history_path(&self) -> String {
        Path::new(self.project_root.as_str())
            .join(FILE_HISTORY_DIR)
            .join(format!("{}.jsonl", self.session_id))
            .to_string_lossy()
            .into_owned()
    }

    pub fn task_path(&self) -> String {
        Path::new(self.project_root.as_str())
            .join(TASK_DIR)
            .join(format!("{}.jsonl", self.session_id))
            .to_string_lossy()
            .into_owned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReadFileCacheState {
    pub entries: usize,
}

impl ReadFileCacheState {
    pub const fn with_entries(entries: usize) -> Self {
        Self { entries }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPersistenceConfig {
    pub identity: SessionIdentity,
    pub persist_session: bool,
    pub read_file_cache: ReadFileCacheState,
}

impl SessionPersistenceConfig {
    pub fn new(
        identity: SessionIdentity,
        persist_session: bool,
        read_file_cache: ReadFileCacheState,
    ) -> Self {
        Self {
            identity,
            persist_session,
            read_file_cache,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPersistencePlan {
    pub session_id: String,
    pub persist_session: bool,
    pub transcript_path: Option<String>,
    pub history_path: Option<String>,
    pub file_history_path: Option<String>,
    pub transcript_flushes: u32,
    pub history_entries: u32,
    pub transcript_messages: usize,
    pub transcript_entries: usize,
    pub history_flushes: u32,
    pub history_pending_entries: usize,
    pub file_history_requested: bool,
    pub file_history_requests: u32,
    pub file_history_committed: u32,
    pub read_file_cache: ReadFileCacheState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionResumePlan {
    pub session_id: String,
    pub transcript_path: Option<String>,
    pub history_path: Option<String>,
    pub file_history_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPersistenceState {
    pub config: SessionPersistenceConfig,
    pub transcript_flushes: u32,
    pub history_entries: u32,
    pub transcript_messages: usize,
    pub transcript_entries: usize,
}

impl SessionPersistenceState {
    pub fn new(config: SessionPersistenceConfig, transcript_messages: usize) -> Self {
        Self {
            config,
            transcript_flushes: 0,
            history_entries: 0,
            transcript_messages,
            transcript_entries: 0,
        }
    }

    pub fn record_submit(
        &mut self,
        transcript_messages: usize,
        transcript_entries: usize,
        history_store: &HistoryStorePlan,
        file_history: &FileHistoryPlan,
    ) -> SessionPersistencePlan {
        self.transcript_messages = transcript_messages;
        self.transcript_entries = transcript_entries;
        if self.config.persist_session {
            self.transcript_flushes += 1;
            self.history_entries += history_store.pending_entries as u32;
        }

        SessionPersistencePlan {
            session_id: self.config.identity.session_id.clone(),
            persist_session: self.config.persist_session,
            transcript_path: self
                .config
                .persist_session
                .then(|| self.config.identity.transcript_path()),
            history_path: self
                .config
                .persist_session
                .then(|| self.config.identity.history_path()),
            file_history_path: self
                .config
                .persist_session
                .then(|| self.config.identity.file_history_path()),
            transcript_flushes: self.transcript_flushes,
            history_entries: self.history_entries,
            transcript_messages: self.transcript_messages,
            transcript_entries: self.transcript_entries,
            history_flushes: history_store.flush_count,
            history_pending_entries: history_store.pending_entries,
            file_history_requested: file_history.snapshot_requested,
            file_history_requests: file_history.total_requests,
            file_history_committed: file_history.total_committed,
            read_file_cache: self.config.read_file_cache.clone(),
        }
    }

    pub fn build_resume_plan(&self) -> SessionResumePlan {
        SessionResumePlan {
            session_id: self.config.identity.session_id.clone(),
            transcript_path: self
                .config
                .persist_session
                .then(|| self.config.identity.transcript_path()),
            history_path: self
                .config
                .persist_session
                .then(|| self.config.identity.history_path()),
            file_history_path: self
                .config
                .persist_session
                .then(|| self.config.identity.file_history_path()),
        }
    }

    pub fn restore_resume_counters(&mut self, transcript_entries: usize, history_entries: usize) {
        if !self.config.persist_session {
            return;
        }
        self.transcript_flushes = u32::from(transcript_entries > 0);
        self.history_entries = history_entries as u32;
        self.transcript_messages = transcript_entries;
        self.transcript_entries = transcript_entries;
    }
}

// ---------------------------------------------------------------------------
// Task persistence
// ---------------------------------------------------------------------------

/// A serializable snapshot of a task record for JSONL persistence.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PersistedTaskRecord {
    pub task_id: String,
    pub task_type: String,
    pub status: String,
    pub summary: String,
    pub timestamp_ms: u64,
}

/// Append a task record to the session's task JSONL file.
pub fn persist_task_record(identity: &SessionIdentity, record: &PersistedTaskRecord) {
    let path = identity.task_path();
    let dir = Path::new(&path).parent();
    if let Some(dir) = dir {
        let _ = fs::create_dir_all(dir);
    }
    if let Ok(line) = serde_json::to_string(record) {
        let _ = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut f| {
                use std::io::Write;
                writeln!(f, "{line}")
            });
    }
}

/// Load all persisted task records from the session's task JSONL file.
pub fn load_persisted_tasks(identity: &SessionIdentity) -> Vec<PersistedTaskRecord> {
    let path = identity.task_path();
    let Ok(content) = fs::read_to_string(&path) else {
        return Vec::new();
    };
    content
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

// ---------------------------------------------------------------------------
// Transcript persistence
// ---------------------------------------------------------------------------

/// Append transcript entries to the session's transcript JSONL file.
pub fn persist_transcript(identity: &SessionIdentity, entries: &[TranscriptEntry]) {
    let path = identity.transcript_path();
    let dir = Path::new(&path).parent();
    if let Some(dir) = dir {
        let _ = fs::create_dir_all(dir);
    }
    let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    use std::io::Write;
    for entry in entries {
        if let Ok(line) = serde_json::to_string(entry) {
            let _ = writeln!(f, "{line}");
        }
    }
}

/// Load all transcript entries from the session's transcript JSONL file.
pub fn load_transcript(identity: &SessionIdentity) -> Vec<TranscriptEntry> {
    let path = identity.transcript_path();
    let Ok(content) = fs::read_to_string(&path) else {
        return Vec::new();
    };
    content
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// List all session IDs that have transcript files in the project.
pub fn list_sessions(project_root: &str) -> Vec<String> {
    let dir = Path::new(project_root).join(SESSION_DIR);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".jsonl").map(String::from)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        ReadFileCacheState, SessionIdentity, SessionPersistenceConfig, SessionPersistenceState,
    };
    use crate::file_history::FileHistoryPlan;
    use crate::history_store::HistoryStorePlan;

    #[test]
    fn session_paths_are_stable() {
        let identity = SessionIdentity::new("session-1", "/tmp/redcode");
        assert_eq!(
            identity.transcript_path(),
            "/tmp/redcode/.nocode/sessions/session-1.jsonl"
        );
        assert_eq!(
            identity.history_path(),
            "/tmp/redcode/.nocode/history.jsonl"
        );
        assert_eq!(
            identity.file_history_path(),
            "/tmp/redcode/.nocode/file-history/session-1.jsonl"
        );
    }

    #[test]
    fn record_submit_tracks_flushes() {
        let config = SessionPersistenceConfig::new(
            SessionIdentity::new("session-1", "/tmp/redcode"),
            true,
            ReadFileCacheState::with_entries(3),
        );
        let mut state = SessionPersistenceState::new(config, 1);
        let plan = state.record_submit(
            8,
            11,
            &HistoryStorePlan {
                session_id: String::from("session-1"),
                persist_history: true,
                pending_entries: 2,
                flush_count: 1,
                entries: Vec::new(),
            },
            &FileHistoryPlan {
                snapshot_requested: true,
                total_requests: 1,
                total_committed: 1,
            },
        );

        assert_eq!(plan.transcript_flushes, 1);
        assert_eq!(plan.history_entries, 2);
        assert_eq!(plan.transcript_messages, 8);
        assert_eq!(plan.transcript_entries, 11);
        assert_eq!(plan.history_flushes, 1);
        assert_eq!(plan.history_pending_entries, 2);
        assert!(plan.file_history_requested);
        assert_eq!(plan.file_history_requests, 1);
        assert_eq!(plan.file_history_committed, 1);
        assert_eq!(plan.read_file_cache.entries, 3);
        assert_eq!(
            plan.transcript_path.as_deref(),
            Some("/tmp/redcode/.nocode/sessions/session-1.jsonl")
        );
        assert_eq!(
            plan.file_history_path.as_deref(),
            Some("/tmp/redcode/.nocode/file-history/session-1.jsonl")
        );
    }

    #[test]
    fn resume_plan_uses_same_paths() {
        let state = SessionPersistenceState::new(
            SessionPersistenceConfig::new(
                SessionIdentity::new("session-1", "/tmp/redcode"),
                true,
                ReadFileCacheState::default(),
            ),
            0,
        );
        let plan = state.build_resume_plan();

        assert_eq!(plan.session_id, "session-1");
        assert_eq!(
            plan.transcript_path.as_deref(),
            Some("/tmp/redcode/.nocode/sessions/session-1.jsonl")
        );
        assert_eq!(
            plan.history_path.as_deref(),
            Some("/tmp/redcode/.nocode/history.jsonl")
        );
        assert_eq!(
            plan.file_history_path.as_deref(),
            Some("/tmp/redcode/.nocode/file-history/session-1.jsonl")
        );
    }

    #[test]
    fn disabled_persistence_keeps_paths_empty() {
        let config = SessionPersistenceConfig::new(
            SessionIdentity::new("session-2", "/tmp/redcode"),
            false,
            ReadFileCacheState::default(),
        );
        let mut state = SessionPersistenceState::new(config, 2);
        let plan = state.record_submit(
            5,
            7,
            &HistoryStorePlan {
                session_id: String::from("session-2"),
                persist_history: false,
                pending_entries: 0,
                flush_count: 0,
                entries: Vec::new(),
            },
            &FileHistoryPlan {
                snapshot_requested: false,
                total_requests: 0,
                total_committed: 0,
            },
        );

        assert!(!plan.persist_session);
        assert!(plan.transcript_path.is_none());
        assert!(plan.history_path.is_none());
        assert!(plan.file_history_path.is_none());
        assert_eq!(plan.transcript_flushes, 0);
        assert_eq!(plan.history_entries, 0);
        assert_eq!(plan.transcript_entries, 7);
    }

    #[test]
    fn restore_resume_counters_rehydrates_snapshot_counts() {
        let mut state = SessionPersistenceState::new(
            SessionPersistenceConfig::new(
                SessionIdentity::new("session-3", "/tmp/redcode"),
                true,
                ReadFileCacheState::default(),
            ),
            0,
        );

        state.restore_resume_counters(9, 2);

        assert_eq!(state.transcript_flushes, 1);
        assert_eq!(state.history_entries, 2);
        assert_eq!(state.transcript_messages, 9);
        assert_eq!(state.transcript_entries, 9);
    }

    #[test]
    fn transcript_roundtrip_persist_and_load() {
        use super::{load_transcript, persist_transcript};
        use crate::transcript::{TranscriptEntry, TranscriptRole};

        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().to_string_lossy().into_owned();
        let identity = SessionIdentity::new("test-session", &root);

        let entries = vec![
            TranscriptEntry::new(1, TranscriptRole::Conversation, "user: hello"),
            TranscriptEntry::new(1, TranscriptRole::Conversation, "assistant: hi there"),
            TranscriptEntry::new(2, TranscriptRole::ToolRequest, "Read file.rs"),
            TranscriptEntry::new(2, TranscriptRole::ToolResult, "contents of file.rs"),
        ];

        persist_transcript(&identity, &entries);

        let loaded = load_transcript(&identity);
        assert_eq!(loaded.len(), 4);
        assert_eq!(loaded[0].turn, 1);
        assert_eq!(loaded[0].role, TranscriptRole::Conversation);
        assert_eq!(loaded[0].content, "user: hello");
        assert_eq!(loaded[2].role, TranscriptRole::ToolRequest);
        assert_eq!(loaded[3].content, "contents of file.rs");
    }

    #[test]
    fn transcript_append_adds_to_existing() {
        use super::{load_transcript, persist_transcript};
        use crate::transcript::{TranscriptEntry, TranscriptRole};

        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().to_string_lossy().into_owned();
        let identity = SessionIdentity::new("append-session", &root);

        let batch1 = vec![TranscriptEntry::new(
            1,
            TranscriptRole::Conversation,
            "first",
        )];
        persist_transcript(&identity, &batch1);

        let batch2 = vec![TranscriptEntry::new(
            2,
            TranscriptRole::Conversation,
            "second",
        )];
        persist_transcript(&identity, &batch2);

        let loaded = load_transcript(&identity);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].content, "first");
        assert_eq!(loaded[1].content, "second");
    }

    #[test]
    fn load_transcript_returns_empty_for_missing_file() {
        use super::load_transcript;

        let identity = SessionIdentity::new("nonexistent", "/tmp/nocode-test-missing");
        let loaded = load_transcript(&identity);
        assert!(loaded.is_empty());
    }

    #[test]
    fn list_sessions_finds_transcript_files() {
        use super::{list_sessions, persist_transcript};
        use crate::transcript::{TranscriptEntry, TranscriptRole};

        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().to_string_lossy().into_owned();

        // Create two sessions
        let id1 = SessionIdentity::new("sess-aaa", &root);
        let id2 = SessionIdentity::new("sess-bbb", &root);
        persist_transcript(
            &id1,
            &[TranscriptEntry::new(1, TranscriptRole::Conversation, "a")],
        );
        persist_transcript(
            &id2,
            &[TranscriptEntry::new(1, TranscriptRole::Conversation, "b")],
        );

        let mut sessions = list_sessions(&root);
        sessions.sort();
        assert_eq!(sessions, vec!["sess-aaa", "sess-bbb"]);
    }
}
