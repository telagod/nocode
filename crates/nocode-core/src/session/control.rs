//! Session control — fork, branch, resume, suspend, complete with parent tracking.

use crate::message::Message;
use chrono::Local;
use serde::{Deserialize, Serialize};

/// Session state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionState {
    Active,
    Suspended,
    Completed,
    Forked,
}

/// Serializable session metadata (without messages — those live in JSONL).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub parent_id: Option<String>,
    pub state: SessionState,
    pub created_at: String,
    pub updated_at: String,
    pub model: String,
    pub message_count: usize,
}

/// A session with lifecycle control.
#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub parent_id: Option<String>,
    pub state: SessionState,
    pub created_at: String,
    pub updated_at: String,
    pub model: String,
    pub messages: Vec<Message>,
}

impl Session {
    pub fn new(id: &str, model: &str) -> Self {
        let now = Local::now().to_rfc3339();
        Self {
            id: id.to_string(),
            parent_id: None,
            state: SessionState::Active,
            created_at: now.clone(),
            updated_at: now,
            model: model.to_string(),
            messages: Vec::new(),
        }
    }

    /// Fork this session — create a new session branching from current state.
    pub fn fork(&self, new_id: &str) -> Self {
        let now = Local::now().to_rfc3339();
        Self {
            id: new_id.to_string(),
            parent_id: Some(self.id.clone()),
            state: SessionState::Active,
            created_at: now.clone(),
            updated_at: now,
            model: self.model.clone(),
            messages: self.messages.clone(),
        }
    }

    /// Suspend the session (can be resumed later).
    pub fn suspend(&mut self) {
        self.state = SessionState::Suspended;
        self.updated_at = Local::now().to_rfc3339();
    }

    /// Resume a suspended session.
    pub fn resume(&mut self) -> Result<(), String> {
        if self.state != SessionState::Suspended {
            return Err(format!("Cannot resume session in state {:?}", self.state));
        }
        self.state = SessionState::Active;
        self.updated_at = Local::now().to_rfc3339();
        Ok(())
    }

    /// Mark session as completed.
    pub fn complete(&mut self) {
        self.state = SessionState::Completed;
        self.updated_at = Local::now().to_rfc3339();
    }

    /// Check if session is active.
    pub fn is_active(&self) -> bool {
        self.state == SessionState::Active
    }

    /// Add a message to the session.
    pub fn push_message(&mut self, msg: Message) {
        self.messages.push(msg);
        self.updated_at = Local::now().to_rfc3339();
    }

    /// Message count.
    pub fn message_count(&self) -> usize {
        self.messages.len()
    }

    /// Convert to serializable metadata (without messages).
    pub fn to_meta(&self) -> SessionMeta {
        SessionMeta {
            id: self.id.clone(),
            parent_id: self.parent_id.clone(),
            state: self.state,
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
            model: self.model.clone(),
            message_count: self.messages.len(),
        }
    }

    /// Persist session metadata to a JSON file.
    pub fn persist_meta(&self, project_root: &str) -> Result<(), String> {
        let dir = format!("{project_root}/.nocode/sessions");
        std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create sessions dir: {e}"))?;
        let path = format!("{dir}/{}.meta.json", self.id);
        let json = serde_json::to_string_pretty(&self.to_meta())
            .map_err(|e| format!("Failed to serialize session meta: {e}"))?;
        std::fs::write(&path, json).map_err(|e| format!("Failed to write session meta: {e}"))?;
        Ok(())
    }

    /// Load session metadata from a JSON file (without messages).
    pub fn load_meta(project_root: &str, session_id: &str) -> Result<SessionMeta, String> {
        let path = format!("{project_root}/.nocode/sessions/{session_id}.meta.json");
        let json = std::fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read session meta: {e}"))?;
        serde_json::from_str(&json).map_err(|e| format!("Failed to parse session meta: {e}"))
    }

    /// List all session metadata files in the project.
    pub fn list_meta(project_root: &str) -> Vec<SessionMeta> {
        let dir = format!("{project_root}/.nocode/sessions");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Vec::new();
        };
        let mut metas = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json")
                && path.to_string_lossy().contains(".meta.")
                && let Ok(json) = std::fs::read_to_string(&path)
                && let Ok(meta) = serde_json::from_str::<SessionMeta>(&json)
            {
                metas.push(meta);
            }
        }
        metas.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        metas
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_session_is_active() {
        let s = Session::new("sess-1", "sonnet");
        assert!(s.is_active());
        assert!(s.parent_id.is_none());
        assert_eq!(s.model, "sonnet");
    }

    #[test]
    fn fork_creates_child() {
        let parent = Session::new("parent", "sonnet");
        let child = parent.fork("child");
        assert_eq!(child.parent_id.as_deref(), Some("parent"));
        assert!(child.is_active());
        assert_eq!(child.model, "sonnet");
    }

    #[test]
    fn suspend_and_resume() {
        let mut s = Session::new("sess-2", "opus");
        s.suspend();
        assert_eq!(s.state, SessionState::Suspended);
        assert!(!s.is_active());
        s.resume().unwrap();
        assert!(s.is_active());
    }

    #[test]
    fn resume_fails_if_not_suspended() {
        let mut s = Session::new("sess-3", "haiku");
        assert!(s.resume().is_err());
    }

    #[test]
    fn complete_session() {
        let mut s = Session::new("sess-4", "sonnet");
        s.complete();
        assert_eq!(s.state, SessionState::Completed);
        assert!(!s.is_active());
    }

    #[test]
    fn push_message_updates_count() {
        let mut s = Session::new("sess-5", "sonnet");
        assert_eq!(s.message_count(), 0);
        s.push_message(Message::user_text("hello"));
        assert_eq!(s.message_count(), 1);
    }

    #[test]
    fn meta_roundtrip() {
        let dir = std::env::temp_dir().join("nocode_session_meta_test");
        let _ = std::fs::create_dir_all(dir.join(".nocode/sessions"));
        let root = dir.to_string_lossy().to_string();

        let mut s = Session::new("meta-test-1", "opus");
        s.push_message(Message::user_text("hello"));
        s.suspend();
        s.persist_meta(&root).unwrap();

        let loaded = Session::load_meta(&root, "meta-test-1").unwrap();
        assert_eq!(loaded.id, "meta-test-1");
        assert_eq!(loaded.state, SessionState::Suspended);
        assert_eq!(loaded.model, "opus");
        assert_eq!(loaded.message_count, 1);
        assert!(loaded.parent_id.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_meta_finds_sessions() {
        let dir = std::env::temp_dir().join("nocode_session_list_test");
        let _ = std::fs::create_dir_all(dir.join(".nocode/sessions"));
        let root = dir.to_string_lossy().to_string();

        let s1 = Session::new("list-1", "sonnet");
        s1.persist_meta(&root).unwrap();
        let s2 = Session::new("list-2", "opus");
        s2.persist_meta(&root).unwrap();

        let metas = Session::list_meta(&root);
        assert_eq!(metas.len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn to_meta_captures_fork_parent() {
        let parent = Session::new("fork-parent", "sonnet");
        let child = parent.fork("fork-child");
        let meta = child.to_meta();
        assert_eq!(meta.parent_id.as_deref(), Some("fork-parent"));
        assert_eq!(meta.state, SessionState::Active);
    }
}
