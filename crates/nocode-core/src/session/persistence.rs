use crate::message::Message;
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

const SESSION_DIR: &str = ".nocode/sessions";

/// Metadata about a saved session.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub id: String,
    pub message_count: usize,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub modified_at: Option<chrono::DateTime<chrono::Utc>>,
    pub first_user_message: Option<String>,
}

/// Manages transcript persistence for a session.
pub struct SessionPersistence {
    transcript_path: PathBuf,
    flushed_count: usize,
}

impl SessionPersistence {
    pub fn new(project_root: &str, session_id: &str) -> Self {
        let dir = Path::new(project_root).join(SESSION_DIR);
        let _ = fs::create_dir_all(&dir);
        Self {
            transcript_path: dir.join(format!("{session_id}.jsonl")),
            flushed_count: 0,
        }
    }

    /// Append only the new (unflushed) messages to the transcript file.
    pub fn flush_incremental(&mut self, messages: &[Message]) {
        if messages.len() <= self.flushed_count {
            return;
        }
        let new_messages = &messages[self.flushed_count..];
        if new_messages.is_empty() {
            return;
        }

        let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.transcript_path)
        else {
            return;
        };

        let mut written = 0usize;
        for msg in new_messages {
            if let Ok(json) = serde_json::to_string(msg) {
                if writeln!(file, "{json}").is_ok() {
                    written += 1;
                } else {
                    break;
                }
            }
        }

        self.flushed_count += written;
    }

    /// Overwrite the transcript with the full message list (final seal).
    pub fn persist_full(&mut self, messages: &[Message]) {
        let Ok(mut file) = fs::File::create(&self.transcript_path) else {
            return;
        };

        for msg in messages {
            if let Ok(json) = serde_json::to_string(msg) {
                let _ = writeln!(file, "{json}");
            }
        }

        self.flushed_count = messages.len();
    }

    /// Load a transcript from disk.
    pub fn load_transcript(path: &Path) -> io::Result<Vec<Message>> {
        let file = fs::File::open(path)?;
        let reader = io::BufReader::new(file);
        let mut messages = Vec::new();
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(msg) = serde_json::from_str::<Message>(&line) {
                messages.push(msg);
            }
        }
        Ok(messages)
    }

    /// List all session IDs in the project.
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

    pub fn transcript_path(&self) -> &Path {
        &self.transcript_path
    }

    /// List all sessions with metadata, sorted by most recently modified.
    pub fn list_sessions_with_info(project_root: &str) -> Vec<SessionInfo> {
        let dir = Path::new(project_root).join(SESSION_DIR);
        let Ok(entries) = fs::read_dir(&dir) else {
            return Vec::new();
        };

        let mut sessions: Vec<SessionInfo> = entries
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let id = name.strip_suffix(".jsonl")?.to_string();
                let meta = e.metadata().ok();
                let created_at = meta
                    .as_ref()
                    .and_then(|m| m.created().ok())
                    .map(chrono::DateTime::<chrono::Utc>::from);
                let modified_at = meta
                    .as_ref()
                    .and_then(|m| m.modified().ok())
                    .map(chrono::DateTime::<chrono::Utc>::from);

                // Count lines and grab first user message without loading all into memory
                let path = e.path();
                let file = fs::File::open(&path).ok()?;
                let reader = io::BufReader::new(file);
                let mut count = 0usize;
                let mut first_user: Option<String> = None;
                for line in reader.lines() {
                    let Ok(line) = line else { break };
                    if line.trim().is_empty() {
                        continue;
                    }
                    count += 1;
                    if first_user.is_none()
                        && let Ok(msg) = serde_json::from_str::<Message>(&line)
                        && msg.role == crate::message::Role::User
                    {
                        let text = msg.text_content();
                        if !text.is_empty() {
                            let preview = if text.len() > 80 {
                                // Find safe UTF-8 boundary
                                let mut idx = 77;
                                while idx > 0 && !text.is_char_boundary(idx) {
                                    idx -= 1;
                                }
                                format!("{}...", &text[..idx])
                            } else {
                                text
                            };
                            first_user = Some(preview);
                        }
                    }
                }

                Some(SessionInfo {
                    id,
                    message_count: count,
                    created_at,
                    modified_at,
                    first_user_message: first_user,
                })
            })
            .collect();

        // Sort by modified_at descending (most recent first)
        sessions.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));
        sessions
    }

    /// Resume a session by loading its transcript.
    pub fn resume(project_root: &str, session_id: &str) -> io::Result<(Self, Vec<Message>)> {
        let dir = Path::new(project_root).join(SESSION_DIR);
        let path = dir.join(format!("{session_id}.jsonl"));
        let messages = Self::load_transcript(&path)?;
        let flushed_count = messages.len();
        Ok((
            Self {
                transcript_path: path,
                flushed_count,
            },
            messages,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Message;

    #[test]
    fn incremental_flush_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        let mut sp = SessionPersistence::new(root, "test-session");

        let mut messages = vec![Message::user_text("hello")];
        sp.flush_incremental(&messages);

        messages.push(Message::assistant_text("hi there"));
        sp.flush_incremental(&messages);

        let loaded = SessionPersistence::load_transcript(sp.transcript_path()).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].text_content(), "hello");
        assert_eq!(loaded[1].text_content(), "hi there");
    }

    #[test]
    fn full_persist_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        let mut sp = SessionPersistence::new(root, "test-overwrite");

        let messages = vec![
            Message::user_text("first"),
            Message::assistant_text("second"),
        ];
        sp.flush_incremental(&messages);

        // Overwrite with just one message
        let new_messages = vec![Message::user_text("only")];
        sp.persist_full(&new_messages);

        let loaded = SessionPersistence::load_transcript(sp.transcript_path()).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].text_content(), "only");
    }

    #[test]
    fn list_sessions_finds_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        let session_dir = dir.path().join(".nocode/sessions");
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(session_dir.join("abc.jsonl"), "").unwrap();
        fs::write(session_dir.join("def.jsonl"), "").unwrap();

        let sessions = SessionPersistence::list_sessions(root);
        assert_eq!(sessions.len(), 2);
        assert!(sessions.contains(&String::from("abc")));
        assert!(sessions.contains(&String::from("def")));
    }

    #[test]
    fn list_sessions_with_info_returns_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        let mut sp = SessionPersistence::new(root, "info-test");

        let messages = vec![
            Message::user_text("what is rust?"),
            Message::assistant_text("Rust is a systems programming language."),
            Message::user_text("thanks"),
        ];
        sp.persist_full(&messages);

        let infos = SessionPersistence::list_sessions_with_info(root);
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].id, "info-test");
        assert_eq!(infos[0].message_count, 3);
        assert_eq!(
            infos[0].first_user_message.as_deref(),
            Some("what is rust?")
        );
    }

    #[test]
    fn resume_loads_session() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        let mut sp = SessionPersistence::new(root, "resume-test");

        let messages = vec![Message::user_text("hello"), Message::assistant_text("hi")];
        sp.persist_full(&messages);

        let (mut resumed, loaded) = SessionPersistence::resume(root, "resume-test").unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].text_content(), "hello");

        // Can continue appending after resume
        let mut all = loaded;
        all.push(Message::user_text("more"));
        resumed.flush_incremental(&all);

        let reloaded = SessionPersistence::load_transcript(resumed.transcript_path()).unwrap();
        assert_eq!(reloaded.len(), 3);
    }
}
