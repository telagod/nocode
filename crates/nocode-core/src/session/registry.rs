//! Session registry — in-memory index of all sessions with metadata.

use crate::session::control::{Session, SessionMeta, SessionState};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// Registry tracking all known sessions.
pub struct SessionRegistry {
    sessions: HashMap<String, SessionMeta>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    /// Register a session from its metadata.
    pub fn register(&mut self, meta: SessionMeta) {
        self.sessions.insert(meta.id.clone(), meta);
    }

    /// Register from a live Session object.
    pub fn register_session(&mut self, session: &Session) {
        self.register(session.to_meta());
    }

    /// Get session metadata by ID.
    pub fn get(&self, id: &str) -> Option<&SessionMeta> {
        self.sessions.get(id)
    }

    /// Update session state.
    pub fn update_state(&mut self, id: &str, state: SessionState) {
        if let Some(meta) = self.sessions.get_mut(id) {
            meta.state = state;
            meta.updated_at = chrono::Local::now().to_rfc3339();
        }
    }

    /// Update message count.
    pub fn update_message_count(&mut self, id: &str, count: usize) {
        if let Some(meta) = self.sessions.get_mut(id) {
            meta.message_count = count;
            meta.updated_at = chrono::Local::now().to_rfc3339();
        }
    }

    /// Remove a session from the registry.
    pub fn remove(&mut self, id: &str) -> Option<SessionMeta> {
        self.sessions.remove(id)
    }

    /// List all sessions, sorted by updated_at descending.
    pub fn list(&self) -> Vec<&SessionMeta> {
        let mut v: Vec<&SessionMeta> = self.sessions.values().collect();
        v.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        v
    }

    /// List sessions filtered by state.
    pub fn list_by_state(&self, state: SessionState) -> Vec<&SessionMeta> {
        let mut v: Vec<&SessionMeta> = self
            .sessions
            .values()
            .filter(|m| m.state == state)
            .collect();
        v.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        v
    }

    /// List sessions filtered by model.
    pub fn list_by_model(&self, model: &str) -> Vec<&SessionMeta> {
        let mut v: Vec<&SessionMeta> = self
            .sessions
            .values()
            .filter(|m| m.model == model)
            .collect();
        v.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        v
    }

    /// Load all session metadata from disk.
    pub fn load_from_disk(&mut self, project_root: &str) -> usize {
        let metas = Session::list_meta(project_root);
        let count = metas.len();
        for meta in metas {
            self.sessions.insert(meta.id.clone(), meta);
        }
        count
    }

    /// Number of registered sessions.
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// Check if registry is empty.
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }
}

impl Default for SessionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Global singleton session registry.
static GLOBAL_SESSION_REGISTRY: OnceLock<Arc<Mutex<SessionRegistry>>> = OnceLock::new();

pub fn global_session_registry() -> &'static Arc<Mutex<SessionRegistry>> {
    GLOBAL_SESSION_REGISTRY.get_or_init(|| Arc::new(Mutex::new(SessionRegistry::new())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_get() {
        let mut reg = SessionRegistry::new();
        let s = Session::new("reg-1", "opus");
        reg.register_session(&s);
        assert_eq!(reg.get("reg-1").unwrap().model, "opus");
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn list_sorted_by_updated() {
        let mut reg = SessionRegistry::new();
        let s1 = Session::new("old", "sonnet");
        let s2 = Session::new("new", "opus");
        reg.register_session(&s1);
        reg.register_session(&s2);
        let list = reg.list();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn filter_by_state() {
        let mut reg = SessionRegistry::new();
        let s1 = Session::new("active-1", "sonnet");
        let mut s2 = Session::new("suspended-1", "opus");
        s2.suspend();
        reg.register_session(&s1);
        reg.register_session(&s2);

        let active = reg.list_by_state(SessionState::Active);
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "active-1");

        let suspended = reg.list_by_state(SessionState::Suspended);
        assert_eq!(suspended.len(), 1);
        assert_eq!(suspended[0].id, "suspended-1");
    }

    #[test]
    fn update_state() {
        let mut reg = SessionRegistry::new();
        let s = Session::new("upd-1", "sonnet");
        reg.register_session(&s);
        reg.update_state("upd-1", SessionState::Completed);
        assert_eq!(reg.get("upd-1").unwrap().state, SessionState::Completed);
    }

    #[test]
    fn remove_session() {
        let mut reg = SessionRegistry::new();
        let s = Session::new("rm-1", "sonnet");
        reg.register_session(&s);
        assert_eq!(reg.len(), 1);
        reg.remove("rm-1");
        assert_eq!(reg.len(), 0);
    }
}
