use crate::file_history::FileHistoryPlan;
use crate::history_store::{HistoryEntry, HistoryStorePlan};
use crate::session_persistence::{SessionPersistencePlan, SessionResumePlan};
use crate::transcript::TranscriptRole;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PersistenceDispatchResult {
    pub transcript_entries_flushed: usize,
    pub history_persisted: bool,
    pub file_history_persisted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedTranscriptEntry {
    pub turn: u32,
    pub role: TranscriptRole,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHistorySnapshot {
    pub snapshot_requested: bool,
    pub total_requests: u32,
    pub total_committed: u32,
}

fn ensure_parent_dir(path: &Path) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("persistence parent directory should be creatable");
    }
}

fn write_lines(path: &Path, lines: &[String]) -> usize {
    ensure_parent_dir(path);
    let mut file =
        fs::File::create(path).expect("transcript persistence target should be writable");
    for line in lines {
        writeln!(file, "{line}").expect("transcript persistence should write line");
    }
    lines.len()
}

fn append_lines(path: &Path, lines: &[String]) -> bool {
    if lines.is_empty() {
        return false;
    }
    ensure_parent_dir(path);
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("history persistence target should be writable");
    for line in lines {
        writeln!(file, "{line}").expect("history persistence should append line");
    }
    true
}

fn read_lines(path: Option<&str>) -> io::Result<Vec<String>> {
    let Some(path) = path else {
        return Ok(Vec::new());
    };
    let file_path = Path::new(path);
    if !file_path.exists() {
        return Ok(Vec::new());
    }
    fs::read_to_string(file_path).map(|content| content.lines().map(String::from).collect())
}

fn escape_json(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn render_history_entry(entry: &HistoryEntry) -> String {
    format!(
        "{{\"display\":\"{}\",\"timestamp\":{},\"sessionId\":\"{}\",\"project\":\"{}\"}}",
        escape_json(entry.display.as_str()),
        entry.timestamp,
        escape_json(entry.session_id.as_str()),
        escape_json(entry.project.as_str())
    )
}

fn extract_json_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{key}\":");
    let start = line.find(needle.as_str())? + needle.len();
    Some(&line[start..])
}

fn extract_json_string(line: &str, key: &str) -> Option<String> {
    let mut rest = extract_json_value(line, key)?;
    rest = rest.strip_prefix('"')?;
    let mut escaped = false;
    let mut value = String::new();
    for ch in rest.chars() {
        if escaped {
            match ch {
                'n' => value.push('\n'),
                '"' => value.push('"'),
                '\\' => value.push('\\'),
                other => value.push(other),
            }
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '"' => return Some(value),
            other => value.push(other),
        }
    }
    None
}

fn extract_json_u64(line: &str, key: &str) -> Option<u64> {
    let rest = extract_json_value(line, key)?;
    let end = rest.find([',', '}']).unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn extract_json_bool(line: &str, key: &str) -> Option<bool> {
    let rest = extract_json_value(line, key)?;
    if rest.starts_with("true") {
        Some(true)
    } else if rest.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

fn parse_transcript_entry(line: &str) -> Option<PersistedTranscriptEntry> {
    let mut fields = line.splitn(3, '\t');
    let turn = fields.next()?.parse().ok()?;
    let role = TranscriptRole::parse_kind(fields.next()?)?;
    let content = fields.next()?.to_string();
    Some(PersistedTranscriptEntry {
        turn,
        role,
        content,
    })
}

fn parse_history_entry(line: &str) -> Option<HistoryEntry> {
    Some(HistoryEntry {
        display: extract_json_string(line, "display")?,
        timestamp: extract_json_u64(line, "timestamp")?,
        session_id: extract_json_string(line, "sessionId")?,
        project: extract_json_string(line, "project")?,
    })
}

fn parse_file_history_snapshot(line: &str) -> Option<FileHistorySnapshot> {
    Some(FileHistorySnapshot {
        snapshot_requested: extract_json_bool(line, "snapshotRequested")?,
        total_requests: extract_json_u64(line, "totalRequests")? as u32,
        total_committed: extract_json_u64(line, "totalCommitted")? as u32,
    })
}

/// Trait defining persistence hooks for transcripts, history, and file checkpoints.
pub trait PersistenceBackend: Send + Sync {
    /// Persist transcript entries. Returns the number of entries flushed.
    fn persist_transcript(&mut self, entries: &[String]) -> usize;

    /// Persist prompt history plan (e.g., history.jsonl append).
    fn persist_history(&mut self, plan: &HistoryStorePlan) -> bool;

    /// Persist file history checkpoint plan.
    fn persist_file_history(&mut self, plan: &FileHistoryPlan) -> bool;

    /// Finalize persistence for this turn (optional additional metadata).
    fn finalize(&mut self, plan: &SessionPersistencePlan);
}

pub trait PersistenceReader: Send + Sync {
    fn read_transcript(
        &self,
        plan: &SessionResumePlan,
    ) -> io::Result<Vec<PersistedTranscriptEntry>>;

    fn read_history(&self, plan: &SessionResumePlan) -> io::Result<Vec<HistoryEntry>>;

    fn read_file_history(
        &self,
        plan: &SessionResumePlan,
    ) -> io::Result<Option<FileHistorySnapshot>>;
}

/// No-op backend for testing or disabled persistence.
#[derive(Default)]
pub struct NoopPersistenceBackend;

#[derive(Default)]
pub struct NoopPersistenceReader;

impl PersistenceBackend for NoopPersistenceBackend {
    fn persist_transcript(&mut self, _entries: &[String]) -> usize {
        0
    }

    fn persist_history(&mut self, _: &HistoryStorePlan) -> bool {
        false
    }

    fn persist_file_history(&mut self, _: &FileHistoryPlan) -> bool {
        false
    }

    fn finalize(&mut self, _: &SessionPersistencePlan) {}
}

impl PersistenceReader for NoopPersistenceReader {
    fn read_transcript(&self, _: &SessionResumePlan) -> io::Result<Vec<PersistedTranscriptEntry>> {
        Ok(Vec::new())
    }

    fn read_history(&self, _: &SessionResumePlan) -> io::Result<Vec<HistoryEntry>> {
        Ok(Vec::new())
    }

    fn read_file_history(&self, _: &SessionResumePlan) -> io::Result<Option<FileHistorySnapshot>> {
        Ok(None)
    }
}

#[derive(Debug, Clone)]
pub struct LocalPersistenceBackend {
    transcript_path: PathBuf,
    history_path: PathBuf,
    file_history_path: PathBuf,
}

impl LocalPersistenceBackend {
    pub fn new(
        transcript_path: impl Into<PathBuf>,
        history_path: impl Into<PathBuf>,
        file_history_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            transcript_path: transcript_path.into(),
            history_path: history_path.into(),
            file_history_path: file_history_path.into(),
        }
    }
}

impl PersistenceBackend for LocalPersistenceBackend {
    fn persist_transcript(&mut self, entries: &[String]) -> usize {
        write_lines(self.transcript_path.as_path(), entries)
    }

    fn persist_history(&mut self, plan: &HistoryStorePlan) -> bool {
        let lines = plan
            .entries
            .iter()
            .map(render_history_entry)
            .collect::<Vec<_>>();
        append_lines(self.history_path.as_path(), lines.as_slice())
    }

    fn persist_file_history(&mut self, plan: &FileHistoryPlan) -> bool {
        let line = format!(
            "{{\"snapshotRequested\":{},\"totalRequests\":{},\"totalCommitted\":{}}}",
            plan.snapshot_requested, plan.total_requests, plan.total_committed
        );
        append_lines(self.file_history_path.as_path(), &[line])
    }

    fn finalize(&mut self, _: &SessionPersistencePlan) {}
}

impl PersistenceReader for LocalPersistenceBackend {
    fn read_transcript(
        &self,
        plan: &SessionResumePlan,
    ) -> io::Result<Vec<PersistedTranscriptEntry>> {
        read_lines(plan.transcript_path.as_deref()).map(|lines| {
            lines
                .iter()
                .filter_map(|line| parse_transcript_entry(line))
                .collect()
        })
    }

    fn read_history(&self, plan: &SessionResumePlan) -> io::Result<Vec<HistoryEntry>> {
        read_lines(plan.history_path.as_deref()).map(|lines| {
            lines
                .iter()
                .filter_map(|line| parse_history_entry(line))
                .filter(|entry| entry.session_id == plan.session_id)
                .collect()
        })
    }

    fn read_file_history(
        &self,
        plan: &SessionResumePlan,
    ) -> io::Result<Option<FileHistorySnapshot>> {
        let lines = read_lines(plan.file_history_path.as_deref())?;
        Ok(lines
            .last()
            .and_then(|line| parse_file_history_snapshot(line)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_history::FileHistoryPlan;
    use crate::history_store::HistoryStorePlan;
    use crate::session_persistence::{
        ReadFileCacheState, SessionPersistencePlan, SessionResumePlan,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn noop_backend_returns_defaults() {
        let mut backend = NoopPersistenceBackend;
        assert_eq!(backend.persist_transcript(&["a".into(), "b".into()]), 0);
        assert!(!backend.persist_history(&HistoryStorePlan {
            session_id: "id".into(),
            persist_history: false,
            pending_entries: 0,
            flush_count: 0,
            entries: Vec::new(),
        }));
        assert!(!backend.persist_file_history(&FileHistoryPlan {
            snapshot_requested: false,
            total_requests: 0,
            total_committed: 0
        }));
        backend.finalize(&SessionPersistencePlan {
            session_id: "id".into(),
            persist_session: false,
            transcript_path: None,
            history_path: None,
            file_history_path: None,
            transcript_flushes: 0,
            history_entries: 0,
            transcript_messages: 0,
            transcript_entries: 0,
            history_flushes: 0,
            history_pending_entries: 0,
            file_history_requested: false,
            file_history_requests: 0,
            file_history_committed: 0,
            read_file_cache: ReadFileCacheState::default(),
        });
    }

    #[test]
    fn local_backend_writes_transcript_history_and_file_history() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("nocode-persistence-{unique}"));
        let transcript_path = root.join(".nocode/sessions/session-1.jsonl");
        let history_path = root.join(".nocode/history.jsonl");
        let file_history_path = root.join(".nocode/file-history/session-1.jsonl");
        let mut backend = LocalPersistenceBackend::new(
            transcript_path.clone(),
            history_path.clone(),
            file_history_path.clone(),
        );

        assert_eq!(
            backend.persist_transcript(&[String::from("1\tconversation\tseed")]),
            1
        );
        assert!(backend.persist_history(&HistoryStorePlan {
            session_id: String::from("session-1"),
            persist_history: true,
            pending_entries: 1,
            flush_count: 1,
            entries: vec![HistoryEntry {
                display: String::from("prompt"),
                timestamp: 1,
                session_id: String::from("session-1"),
                project: String::from("/tmp/project"),
            }],
        }));
        assert!(backend.persist_file_history(&FileHistoryPlan {
            snapshot_requested: true,
            total_requests: 1,
            total_committed: 1,
        }));

        let transcript =
            fs::read_to_string(transcript_path.clone()).expect("transcript should exist");
        let history = fs::read_to_string(history_path.clone()).expect("history should exist");
        let file_history =
            fs::read_to_string(file_history_path.clone()).expect("file history should exist");
        assert!(transcript.contains("conversation"));
        assert!(history.contains("\"display\":\"prompt\""));
        assert!(file_history.contains("\"snapshotRequested\":true"));

        let resume_plan = SessionResumePlan {
            session_id: String::from("session-1"),
            transcript_path: Some(transcript_path.to_string_lossy().into_owned()),
            history_path: Some(history_path.to_string_lossy().into_owned()),
            file_history_path: Some(file_history_path.to_string_lossy().into_owned()),
        };
        assert_eq!(
            backend
                .read_transcript(&resume_plan)
                .expect("transcript read should work"),
            vec![PersistedTranscriptEntry {
                turn: 1,
                role: TranscriptRole::Conversation,
                content: String::from("seed"),
            }]
        );
        assert_eq!(
            backend
                .read_history(&resume_plan)
                .expect("history read should work"),
            vec![HistoryEntry {
                display: String::from("prompt"),
                timestamp: 1,
                session_id: String::from("session-1"),
                project: String::from("/tmp/project"),
            }]
        );
        assert_eq!(
            backend
                .read_file_history(&resume_plan)
                .expect("file history read should work"),
            Some(FileHistorySnapshot {
                snapshot_requested: true,
                total_requests: 1,
                total_committed: 1,
            })
        );

        let _ = fs::remove_dir_all(root);
    }
}
