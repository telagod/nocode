//! Session control — fork, branch, resume, suspend, complete with parent tracking.

use crate::message::Message;
use chrono::Local;

/// Session state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Active,
    Suspended,
    Completed,
    Forked,
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
}
