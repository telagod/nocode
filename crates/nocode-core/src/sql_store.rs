use std::fs;
use std::sync::{Arc, Mutex, OnceLock};

use chrono::Local;
use rusqlite::{params, Connection};

// ---------------------------------------------------------------------------
// Row types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SessionRow {
    pub id: String,
    pub created_at: String,
    pub updated_at: String,
    pub model: Option<String>,
    pub status: String,
    pub message_count: usize,
}

#[derive(Debug, Clone)]
pub struct MessageRow {
    pub id: i64,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub turn: u32,
    pub created_at: String,
    pub token_count: u64,
}

#[derive(Debug, Clone)]
pub struct MemoryRow {
    pub name: String,
    pub description: String,
    pub memory_type: String,
    pub content: String,
    pub file_name: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct CommandRow {
    pub id: i64,
    pub command: String,
    pub session_id: Option<String>,
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    model TEXT,
    status TEXT DEFAULT 'active',
    metadata TEXT
);

CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    turn INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    token_count INTEGER DEFAULT 0,
    metadata TEXT
);

CREATE TABLE IF NOT EXISTS memories (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    memory_type TEXT NOT NULL,
    content TEXT NOT NULL,
    file_name TEXT UNIQUE NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS command_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    command TEXT NOT NULL,
    session_id TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS telemetry_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT,
    event_type TEXT NOT NULL,
    event_data TEXT,
    created_at TEXT NOT NULL
);
";

// ---------------------------------------------------------------------------
// SqlStore
// ---------------------------------------------------------------------------

pub struct SqlStore {
    base_dir: String,
}

impl SqlStore {
    pub fn new(base_dir: &str) -> Result<Self, String> {
        fs::create_dir_all(base_dir)
            .map_err(|e| format!("failed to create data dir: {e}"))?;
        Ok(Self {
            base_dir: base_dir.to_string(),
        })
    }

    fn db_path_for_date(&self, date: &str) -> String {
        format!("{}/nocode_{date}.db", self.base_dir)
    }

    fn connection(&self) -> Result<Connection, String> {
        let date = Local::now().format("%Y-%m-%d").to_string();
        self.connection_for_date(&date)
    }

    fn connection_for_date(&self, date: &str) -> Result<Connection, String> {
        let path = self.db_path_for_date(date);
        let conn = Connection::open(&path).map_err(|e| e.to_string())?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .map_err(|e| e.to_string())?;
        self.ensure_schema(&conn)?;
        Ok(conn)
    }

    fn ensure_schema(&self, conn: &Connection) -> Result<(), String> {
        conn.execute_batch(SCHEMA).map_err(|e| e.to_string())
    }

    pub fn list_volumes(&self) -> Result<Vec<String>, String> {
        let dir = fs::read_dir(&self.base_dir)
            .map_err(|e| format!("failed to read data dir: {e}"))?;
        let mut dates: Vec<String> = Vec::new();
        for entry in dir {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(date) = name
                .strip_prefix("nocode_")
                .and_then(|s| s.strip_suffix(".db"))
            {
                dates.push(date.to_string());
            }
        }
        dates.sort();
        Ok(dates)
    }

    // -----------------------------------------------------------------------
    // Session CRUD
    // -----------------------------------------------------------------------

    pub fn create_session(&self, id: &str, model: &str) -> Result<(), String> {
        let conn = self.connection()?;
        let now = Local::now().to_rfc3339();
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at, model, status) VALUES (?1, ?2, ?3, ?4, 'active')",
            params![id, now, now, model],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn update_session(&self, id: &str, status: &str) -> Result<(), String> {
        let conn = self.connection()?;
        let now = Local::now().to_rfc3339();
        conn.execute(
            "UPDATE sessions SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![status, now, id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn get_session(&self, id: &str) -> Result<Option<SessionRow>, String> {
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.created_at, s.updated_at, s.model, s.status, \
                 (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id) \
                 FROM sessions s WHERE s.id = ?1",
            )
            .map_err(|e| e.to_string())?;
        let mut rows = stmt
            .query_map(params![id], |row| {
                Ok(SessionRow {
                    id: row.get(0)?,
                    created_at: row.get(1)?,
                    updated_at: row.get(2)?,
                    model: row.get(3)?,
                    status: row.get(4)?,
                    message_count: row.get::<_, i64>(5)? as usize,
                })
            })
            .map_err(|e| e.to_string())?;
        match rows.next() {
            Some(r) => Ok(Some(r.map_err(|e| e.to_string())?)),
            None => Ok(None),
        }
    }

    pub fn list_sessions(&self, limit: usize) -> Result<Vec<SessionRow>, String> {
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.created_at, s.updated_at, s.model, s.status, \
                 (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id) \
                 FROM sessions s ORDER BY s.created_at DESC LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![limit as i64], |row| {
                Ok(SessionRow {
                    id: row.get(0)?,
                    created_at: row.get(1)?,
                    updated_at: row.get(2)?,
                    model: row.get(3)?,
                    status: row.get(4)?,
                    message_count: row.get::<_, i64>(5)? as usize,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    pub fn list_sessions_for_date(&self, date: &str) -> Result<Vec<SessionRow>, String> {
        let conn = self.connection_for_date(date)?;
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.created_at, s.updated_at, s.model, s.status, \
                 (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id) \
                 FROM sessions s ORDER BY s.created_at DESC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok(SessionRow {
                    id: row.get(0)?,
                    created_at: row.get(1)?,
                    updated_at: row.get(2)?,
                    model: row.get(3)?,
                    status: row.get(4)?,
                    message_count: row.get::<_, i64>(5)? as usize,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    // -----------------------------------------------------------------------
    // Message CRUD
    // -----------------------------------------------------------------------

    pub fn insert_message(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        turn: u32,
        token_count: u64,
    ) -> Result<i64, String> {
        let conn = self.connection()?;
        let now = Local::now().to_rfc3339();
        conn.execute(
            "INSERT INTO messages (session_id, role, content, turn, created_at, token_count) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![session_id, role, content, turn as i64, now, token_count as i64],
        )
        .map_err(|e| e.to_string())?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_messages(&self, session_id: &str) -> Result<Vec<MessageRow>, String> {
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, session_id, role, content, turn, created_at, token_count \
                 FROM messages WHERE session_id = ?1 ORDER BY id ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![session_id], |row| {
                Ok(MessageRow {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    role: row.get(2)?,
                    content: row.get(3)?,
                    turn: row.get::<_, i64>(4)? as u32,
                    created_at: row.get(5)?,
                    token_count: row.get::<_, i64>(6)? as u64,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    pub fn get_messages_since(
        &self,
        session_id: &str,
        after_id: i64,
    ) -> Result<Vec<MessageRow>, String> {
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, session_id, role, content, turn, created_at, token_count \
                 FROM messages WHERE session_id = ?1 AND id > ?2 ORDER BY id ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![session_id, after_id], |row| {
                Ok(MessageRow {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    role: row.get(2)?,
                    content: row.get(3)?,
                    turn: row.get::<_, i64>(4)? as u32,
                    created_at: row.get(5)?,
                    token_count: row.get::<_, i64>(6)? as u64,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    pub fn count_messages(&self, session_id: &str) -> Result<usize, String> {
        let conn = self.connection()?;
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok(count as usize)
    }

    // -----------------------------------------------------------------------
    // Memory CRUD
    // -----------------------------------------------------------------------

    pub fn save_memory(&self, entry: &MemoryRow) -> Result<(), String> {
        let conn = self.connection()?;
        conn.execute(
            "INSERT INTO memories (name, description, memory_type, content, file_name, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
             ON CONFLICT(file_name) DO UPDATE SET \
             name = excluded.name, description = excluded.description, \
             memory_type = excluded.memory_type, content = excluded.content, \
             updated_at = excluded.updated_at",
            params![
                entry.name,
                entry.description,
                entry.memory_type,
                entry.content,
                entry.file_name,
                entry.created_at,
                entry.updated_at,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn get_memory(&self, file_name: &str) -> Result<Option<MemoryRow>, String> {
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT name, description, memory_type, content, file_name, created_at, updated_at \
                 FROM memories WHERE file_name = ?1",
            )
            .map_err(|e| e.to_string())?;
        let mut rows = stmt
            .query_map(params![file_name], |row| {
                Ok(MemoryRow {
                    name: row.get(0)?,
                    description: row.get(1)?,
                    memory_type: row.get(2)?,
                    content: row.get(3)?,
                    file_name: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })
            .map_err(|e| e.to_string())?;
        match rows.next() {
            Some(r) => Ok(Some(r.map_err(|e| e.to_string())?)),
            None => Ok(None),
        }
    }

    pub fn delete_memory(&self, file_name: &str) -> Result<bool, String> {
        let conn = self.connection()?;
        let affected = conn
            .execute(
                "DELETE FROM memories WHERE file_name = ?1",
                params![file_name],
            )
            .map_err(|e| e.to_string())?;
        Ok(affected > 0)
    }

    pub fn list_memories(&self, memory_type: Option<&str>) -> Result<Vec<MemoryRow>, String> {
        let conn = self.connection()?;
        let rows = if let Some(mt) = memory_type {
            let mut stmt = conn
                .prepare(
                    "SELECT name, description, memory_type, content, file_name, created_at, updated_at \
                     FROM memories WHERE memory_type = ?1 ORDER BY name ASC",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![mt], |row| {
                Ok(MemoryRow {
                    name: row.get(0)?,
                    description: row.get(1)?,
                    memory_type: row.get(2)?,
                    content: row.get(3)?,
                    file_name: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
        } else {
            let mut stmt = conn
                .prepare(
                    "SELECT name, description, memory_type, content, file_name, created_at, updated_at \
                     FROM memories ORDER BY name ASC",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map([], |row| {
                Ok(MemoryRow {
                    name: row.get(0)?,
                    description: row.get(1)?,
                    memory_type: row.get(2)?,
                    content: row.get(3)?,
                    file_name: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
        };
        Ok(rows)
    }

    pub fn search_memories(&self, query: &str) -> Result<Vec<MemoryRow>, String> {
        let conn = self.connection()?;
        let pattern = format!("%{query}%");
        let mut stmt = conn
            .prepare(
                "SELECT name, description, memory_type, content, file_name, created_at, updated_at \
                 FROM memories \
                 WHERE name LIKE ?1 OR description LIKE ?1 OR content LIKE ?1 \
                 ORDER BY name ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![pattern], |row| {
                Ok(MemoryRow {
                    name: row.get(0)?,
                    description: row.get(1)?,
                    memory_type: row.get(2)?,
                    content: row.get(3)?,
                    file_name: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    // -----------------------------------------------------------------------
    // Command History + Telemetry
    // -----------------------------------------------------------------------

    pub fn insert_command(&self, command: &str, session_id: Option<&str>) -> Result<(), String> {
        let conn = self.connection()?;
        let now = Local::now().to_rfc3339();
        conn.execute(
            "INSERT INTO command_history (command, session_id, created_at) VALUES (?1, ?2, ?3)",
            params![command, session_id, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn recent_commands(&self, limit: usize) -> Result<Vec<CommandRow>, String> {
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, command, session_id, created_at \
                 FROM command_history ORDER BY id DESC LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![limit as i64], |row| {
                Ok(CommandRow {
                    id: row.get(0)?,
                    command: row.get(1)?,
                    session_id: row.get(2)?,
                    created_at: row.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    pub fn insert_telemetry(
        &self,
        session_id: Option<&str>,
        event_type: &str,
        event_data: &str,
    ) -> Result<(), String> {
        let conn = self.connection()?;
        let now = Local::now().to_rfc3339();
        conn.execute(
            "INSERT INTO telemetry_events (session_id, event_type, event_data, created_at) \
             VALUES (?1, ?2, ?3, ?4)",
            params![session_id, event_type, event_data, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Global singleton
// ---------------------------------------------------------------------------

static SQL_STORE: OnceLock<Arc<Mutex<SqlStore>>> = OnceLock::new();

pub fn global_sql_store() -> Arc<Mutex<SqlStore>> {
    SQL_STORE
        .get_or_init(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            let base = format!("{home}/.nocode/data");
            let store = SqlStore::new(&base).expect("failed to init SqlStore");
            Arc::new(Mutex::new(store))
        })
        .clone()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_store() -> (tempfile::TempDir, SqlStore) {
        let tmp = tempfile::tempdir().unwrap();
        let store = SqlStore::new(tmp.path().to_str().unwrap()).unwrap();
        (tmp, store)
    }

    #[test]
    fn create_and_get_session() {
        let (_tmp, store) = make_store();
        store.create_session("s1", "claude-opus-4-6").unwrap();
        let row = store.get_session("s1").unwrap().unwrap();
        assert_eq!(row.id, "s1");
        assert_eq!(row.model.as_deref(), Some("claude-opus-4-6"));
        assert_eq!(row.status, "active");
        assert_eq!(row.message_count, 0);

        store.update_session("s1", "completed").unwrap();
        let row2 = store.get_session("s1").unwrap().unwrap();
        assert_eq!(row2.status, "completed");

        assert!(store.get_session("nonexistent").unwrap().is_none());
    }

    #[test]
    fn insert_and_get_messages() {
        let (_tmp, store) = make_store();
        store.create_session("s1", "test-model").unwrap();

        let id1 = store.insert_message("s1", "user", "hello", 0, 5).unwrap();
        let id2 = store.insert_message("s1", "assistant", "hi there", 1, 10).unwrap();
        assert!(id2 > id1);

        let msgs = store.get_messages("s1").unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content, "hello");
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[1].token_count, 10);

        let since = store.get_messages_since("s1", id1).unwrap();
        assert_eq!(since.len(), 1);
        assert_eq!(since[0].id, id2);

        let count = store.count_messages("s1").unwrap();
        assert_eq!(count, 2);

        // session message_count should reflect
        let sess = store.get_session("s1").unwrap().unwrap();
        assert_eq!(sess.message_count, 2);
    }

    #[test]
    fn save_and_search_memories() {
        let (_tmp, store) = make_store();
        let now = Local::now().to_rfc3339();
        let m1 = MemoryRow {
            name: "user_role".to_string(),
            description: "user is a pentester".to_string(),
            memory_type: "user".to_string(),
            content: "Senior security researcher.".to_string(),
            file_name: "user_role.md".to_string(),
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        let m2 = MemoryRow {
            name: "feedback_testing".to_string(),
            description: "no mocks in tests".to_string(),
            memory_type: "feedback".to_string(),
            content: "Use real database for integration tests.".to_string(),
            file_name: "feedback_testing.md".to_string(),
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        store.save_memory(&m1).unwrap();
        store.save_memory(&m2).unwrap();

        // get by file_name
        let got = store.get_memory("user_role.md").unwrap().unwrap();
        assert_eq!(got.name, "user_role");

        // list all
        let all = store.list_memories(None).unwrap();
        assert_eq!(all.len(), 2);

        // list by type
        let feedback = store.list_memories(Some("feedback")).unwrap();
        assert_eq!(feedback.len(), 1);
        assert_eq!(feedback[0].file_name, "feedback_testing.md");

        // search
        let found = store.search_memories("pentester").unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].file_name, "user_role.md");

        let found2 = store.search_memories("database").unwrap();
        assert_eq!(found2.len(), 1);
        assert_eq!(found2[0].file_name, "feedback_testing.md");

        // upsert: save_memory with same file_name updates
        let m1_updated = MemoryRow {
            name: "user_role_v2".to_string(),
            ..m1.clone()
        };
        store.save_memory(&m1_updated).unwrap();
        let got2 = store.get_memory("user_role.md").unwrap().unwrap();
        assert_eq!(got2.name, "user_role_v2");
        assert_eq!(store.list_memories(None).unwrap().len(), 2);

        // delete
        assert!(store.delete_memory("user_role.md").unwrap());
        assert!(store.get_memory("user_role.md").unwrap().is_none());
        assert!(!store.delete_memory("nonexistent.md").unwrap());
    }

    #[test]
    fn list_volumes() {
        let (_tmp, store) = make_store();
        // Creating a session writes to today's volume
        store.create_session("s1", "m").unwrap();
        let vols = store.list_volumes().unwrap();
        assert_eq!(vols.len(), 1);
        let today = Local::now().format("%Y-%m-%d").to_string();
        assert_eq!(vols[0], today);
    }

    #[test]
    fn date_based_db_path() {
        let (_tmp, store) = make_store();
        let path = store.db_path_for_date("2026-04-06");
        assert!(path.ends_with("/nocode_2026-04-06.db"));
    }

    #[test]
    fn insert_and_recent_commands() {
        let (_tmp, store) = make_store();
        store.insert_command("ls -la", None).unwrap();
        store.insert_command("cargo test", Some("s1")).unwrap();
        store.insert_command("git status", None).unwrap();

        let cmds = store.recent_commands(2).unwrap();
        assert_eq!(cmds.len(), 2);
        // Most recent first
        assert_eq!(cmds[0].command, "git status");
        assert_eq!(cmds[1].command, "cargo test");
        assert_eq!(cmds[1].session_id.as_deref(), Some("s1"));

        let all = store.recent_commands(100).unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn schema_creation_idempotent() {
        let (_tmp, store) = make_store();
        // First connection creates schema
        store.create_session("s1", "m").unwrap();
        // Second connection on same date re-runs ensure_schema — must not fail
        store.create_session("s2", "m").unwrap();
        let sessions = store.list_sessions(10).unwrap();
        assert_eq!(sessions.len(), 2);
    }

    #[test]
    fn telemetry_insert() {
        let (_tmp, store) = make_store();
        store
            .insert_telemetry(Some("s1"), "model_call", r#"{"tokens": 100}"#)
            .unwrap();
        store
            .insert_telemetry(None, "startup", "{}")
            .unwrap();
        // No read API required by spec, just verify no errors
    }

    #[test]
    fn list_sessions_ordering() {
        let (_tmp, store) = make_store();
        store.create_session("a", "m1").unwrap();
        store.create_session("b", "m2").unwrap();
        store.create_session("c", "m3").unwrap();

        let limited = store.list_sessions(2).unwrap();
        assert_eq!(limited.len(), 2);
        // Most recent first
        assert_eq!(limited[0].id, "c");
        assert_eq!(limited[1].id, "b");
    }
}
