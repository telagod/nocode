use crate::session_persistence::SessionIdentity;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Metadata for a session node in the session tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMetadata {
    pub session_id: String,
    pub parent_id: Option<String>,
    pub branch_name: Option<String>,
    pub created_at_ms: u64,
    pub message_count: usize,
    pub status: SessionStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatus {
    Active,
    Suspended,
    Completed,
    Forked,
}

/// A checkpoint from which a session can be forked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCheckpoint {
    pub session_id: String,
    pub message_index: usize,
    pub timestamp_ms: u64,
    pub label: Option<String>,
}

/// Manages session branching, forking, and resumption.
#[derive(Debug)]
pub struct SessionControl {
    sessions: HashMap<String, SessionMetadata>,
    checkpoints: Vec<SessionCheckpoint>,
    next_fork_id: u64,
}

impl SessionControl {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            checkpoints: Vec::new(),
            next_fork_id: 1,
        }
    }

    /// Register a root session.
    pub fn register(&mut self, identity: &SessionIdentity) -> &SessionMetadata {
        let meta = SessionMetadata {
            session_id: identity.session_id.clone(),
            parent_id: None,
            branch_name: None,
            created_at_ms: now_ms(),
            message_count: 0,
            status: SessionStatus::Active,
        };
        self.sessions
            .entry(identity.session_id.clone())
            .or_insert(meta)
    }

    /// Create a checkpoint at the current message index.
    pub fn checkpoint(
        &mut self,
        session_id: &str,
        message_index: usize,
        label: Option<&str>,
    ) -> Result<&SessionCheckpoint, String> {
        if !self.sessions.contains_key(session_id) {
            return Err(format!("session not found: {session_id}"));
        }
        let cp = SessionCheckpoint {
            session_id: session_id.to_string(),
            message_index,
            timestamp_ms: now_ms(),
            label: label.map(ToString::to_string),
        };
        self.checkpoints.push(cp);
        Ok(self.checkpoints.last().expect("just pushed"))
    }

    /// Fork a new child session from a checkpoint.
    /// The parent session is marked as Forked.
    pub fn fork(
        &mut self,
        parent_session_id: &str,
        checkpoint_index: usize,
        branch_name: Option<&str>,
    ) -> Result<String, String> {
        let parent = self
            .sessions
            .get(parent_session_id)
            .ok_or_else(|| format!("parent session not found: {parent_session_id}"))?;
        if parent.status == SessionStatus::Completed {
            return Err("cannot fork a completed session".to_string());
        }

        let _cp = self
            .checkpoints
            .iter()
            .find(|c| c.session_id == parent_session_id && c.message_index == checkpoint_index)
            .ok_or_else(|| {
                format!("checkpoint not found at index {checkpoint_index} for {parent_session_id}")
            })?;

        let fork_id = format!("{parent_session_id}-fork-{}", self.next_fork_id);
        self.next_fork_id += 1;

        let child = SessionMetadata {
            session_id: fork_id.clone(),
            parent_id: Some(parent_session_id.to_string()),
            branch_name: branch_name.map(ToString::to_string),
            created_at_ms: now_ms(),
            message_count: checkpoint_index,
            status: SessionStatus::Active,
        };
        self.sessions.insert(fork_id.clone(), child);

        // Mark parent as forked.
        if let Some(p) = self.sessions.get_mut(parent_session_id) {
            p.status = SessionStatus::Forked;
        }

        Ok(fork_id)
    }

    /// Resume a suspended session.
    pub fn resume(&mut self, session_id: &str) -> Result<&SessionMetadata, String> {
        let meta = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("session not found: {session_id}"))?;
        match meta.status {
            SessionStatus::Completed => Err("cannot resume a completed session".to_string()),
            SessionStatus::Active => Ok(meta),
            SessionStatus::Suspended | SessionStatus::Forked => {
                meta.status = SessionStatus::Active;
                Ok(meta)
            }
        }
    }

    /// Suspend an active session.
    pub fn suspend(&mut self, session_id: &str) -> Result<(), String> {
        let meta = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("session not found: {session_id}"))?;
        if meta.status != SessionStatus::Active {
            return Err(format!("session {session_id} is not active"));
        }
        meta.status = SessionStatus::Suspended;
        Ok(())
    }

    /// Complete a session.
    pub fn complete(&mut self, session_id: &str) -> Result<(), String> {
        let meta = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("session not found: {session_id}"))?;
        meta.status = SessionStatus::Completed;
        Ok(())
    }

    /// Update message count for a session.
    pub fn update_message_count(&mut self, session_id: &str, count: usize) {
        if let Some(meta) = self.sessions.get_mut(session_id) {
            meta.message_count = count;
        }
    }

    pub fn get(&self, session_id: &str) -> Option<&SessionMetadata> {
        self.sessions.get(session_id)
    }

    /// List all branches (children) of a session.
    pub fn list_branches(&self, parent_session_id: &str) -> Vec<&SessionMetadata> {
        self.sessions
            .values()
            .filter(|m| m.parent_id.as_deref() == Some(parent_session_id))
            .collect()
    }

    /// List all checkpoints for a session.
    pub fn list_checkpoints(&self, session_id: &str) -> Vec<&SessionCheckpoint> {
        self.checkpoints
            .iter()
            .filter(|c| c.session_id == session_id)
            .collect()
    }

    /// List all sessions.
    pub fn list_all(&self) -> Vec<&SessionMetadata> {
        self.sessions.values().collect()
    }
}

impl Default for SessionControl {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_identity(id: &str) -> SessionIdentity {
        SessionIdentity::new(id, "/tmp/test-project")
    }

    #[test]
    fn register_and_get() {
        let mut sc = SessionControl::new();
        sc.register(&make_identity("s1"));
        let meta = sc.get("s1").unwrap();
        assert_eq!(meta.session_id, "s1");
        assert_eq!(meta.status, SessionStatus::Active);
        assert!(meta.parent_id.is_none());
    }

    #[test]
    fn checkpoint_and_fork() {
        let mut sc = SessionControl::new();
        sc.register(&make_identity("s1"));
        sc.update_message_count("s1", 5);
        sc.checkpoint("s1", 3, Some("before-refactor")).unwrap();

        let fork_id = sc.fork("s1", 3, Some("experiment")).unwrap();
        assert!(fork_id.contains("fork"));

        let child = sc.get(&fork_id).unwrap();
        assert_eq!(child.parent_id.as_deref(), Some("s1"));
        assert_eq!(child.branch_name.as_deref(), Some("experiment"));
        assert_eq!(child.message_count, 3);
        assert_eq!(child.status, SessionStatus::Active);

        // Parent marked as forked.
        assert_eq!(sc.get("s1").unwrap().status, SessionStatus::Forked);
    }

    #[test]
    fn fork_nonexistent_checkpoint_fails() {
        let mut sc = SessionControl::new();
        sc.register(&make_identity("s1"));
        let err = sc.fork("s1", 99, None).unwrap_err();
        assert!(err.contains("checkpoint not found"));
    }

    #[test]
    fn fork_completed_session_fails() {
        let mut sc = SessionControl::new();
        sc.register(&make_identity("s1"));
        sc.checkpoint("s1", 0, None).unwrap();
        sc.complete("s1").unwrap();
        let err = sc.fork("s1", 0, None).unwrap_err();
        assert!(err.contains("cannot fork a completed"));
    }

    #[test]
    fn suspend_and_resume() {
        let mut sc = SessionControl::new();
        sc.register(&make_identity("s1"));
        sc.suspend("s1").unwrap();
        assert_eq!(sc.get("s1").unwrap().status, SessionStatus::Suspended);

        sc.resume("s1").unwrap();
        assert_eq!(sc.get("s1").unwrap().status, SessionStatus::Active);
    }

    #[test]
    fn resume_completed_fails() {
        let mut sc = SessionControl::new();
        sc.register(&make_identity("s1"));
        sc.complete("s1").unwrap();
        let err = sc.resume("s1").unwrap_err();
        assert!(err.contains("cannot resume a completed"));
    }

    #[test]
    fn list_branches() {
        let mut sc = SessionControl::new();
        sc.register(&make_identity("s1"));
        sc.checkpoint("s1", 2, None).unwrap();
        sc.fork("s1", 2, Some("branch-a")).unwrap();

        // Re-activate parent to fork again.
        sc.resume("s1").unwrap();
        sc.checkpoint("s1", 4, None).unwrap();
        sc.fork("s1", 4, Some("branch-b")).unwrap();

        let branches = sc.list_branches("s1");
        assert_eq!(branches.len(), 2);
    }

    #[test]
    fn list_checkpoints() {
        let mut sc = SessionControl::new();
        sc.register(&make_identity("s1"));
        sc.checkpoint("s1", 1, Some("cp1")).unwrap();
        sc.checkpoint("s1", 5, Some("cp2")).unwrap();
        let cps = sc.list_checkpoints("s1");
        assert_eq!(cps.len(), 2);
        assert_eq!(cps[0].label.as_deref(), Some("cp1"));
        assert_eq!(cps[1].message_index, 5);
    }

    #[test]
    fn suspend_non_active_fails() {
        let mut sc = SessionControl::new();
        sc.register(&make_identity("s1"));
        sc.suspend("s1").unwrap();
        let err = sc.suspend("s1").unwrap_err();
        assert!(err.contains("not active"));
    }

    #[test]
    fn checkpoint_nonexistent_session_fails() {
        let mut sc = SessionControl::new();
        let err = sc.checkpoint("ghost", 0, None).unwrap_err();
        assert!(err.contains("not found"));
    }
}
