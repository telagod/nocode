//! SQL store — rusqlite with date-based volume partitioning.

use std::fs;
use std::sync::{Arc, Mutex, OnceLock};

use chrono::Local;
use rusqlite::{Connection, params};

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
    pub id: i64,
    pub name: String,
    pub memory_type: String,
    pub description: Option<String>,
    pub content: String,
    pub file_path: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct TelemetryRow {
    pub id: i64,
    pub event_type: String,
    pub data: Option<String>,
    pub session_id: Option<String>,
    pub created_at: String,
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

CREATE TABLE IF NOT EXISTS command_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    command TEXT NOT NULL,
    session_id TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS memories (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    memory_type TEXT NOT NULL,
    description TEXT,
    content TEXT NOT NULL,
    file_path TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS telemetry_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    event_type TEXT NOT NULL,
    data TEXT,
    session_id TEXT,
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
        fs::create_dir_all(base_dir).map_err(|e| format!("failed to create data dir: {e}"))?;
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
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .map_err(|e| e.to_string())?;
        // Schema versioning: check user_version and apply migrations
        let version: u32 = conn
            .query_row("PRAGMA user_version;", [], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        if version == 0 {
            conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
            conn.execute_batch("PRAGMA user_version = 1;")
                .map_err(|e| e.to_string())?;
        }
        // Future migrations: if version < 2 { ... PRAGMA user_version = 2; }
        Ok(conn)
    }

    pub fn list_volumes(&self) -> Result<Vec<String>, String> {
        if !std::path::Path::new(&self.base_dir).exists() {
            return Ok(Vec::new());
        }
        let dir =
            fs::read_dir(&self.base_dir).map_err(|e| format!("failed to read data dir: {e}"))?;
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

    // -- Session CRUD --

    pub fn create_session(&self, id: &str, model: &str) -> Result<(), String> {
        let conn = self.connection()?;
        let now = Local::now().to_rfc3339();
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at, model, status) VALUES (?1, ?2, ?3, ?4, 'active')",
            params![id, now, now, model],
        ).map_err(|e| e.to_string())?;
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
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    // -- Message CRUD --

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
            params![
                session_id,
                role,
                content,
                turn as i64,
                now,
                token_count as i64
            ],
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
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
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

    // -- Command history --

    pub fn insert_command(&self, command: &str, session_id: Option<&str>) -> Result<i64, String> {
        let conn = self.connection()?;
        let now = Local::now().to_rfc3339();
        conn.execute(
            "INSERT INTO command_history (command, session_id, created_at) VALUES (?1, ?2, ?3)",
            params![command, session_id, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_commands(&self, limit: usize) -> Result<Vec<CommandRow>, String> {
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
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    // -- Memories CRUD --

    pub fn insert_memory(
        &self,
        name: &str,
        memory_type: &str,
        description: Option<&str>,
        content: &str,
        file_path: Option<&str>,
    ) -> Result<i64, String> {
        let conn = self.connection()?;
        let now = Local::now().to_rfc3339();
        conn.execute(
            "INSERT INTO memories (name, memory_type, description, content, file_path, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![name, memory_type, description, content, file_path, now, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_memories(&self, limit: usize) -> Result<Vec<MemoryRow>, String> {
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, name, memory_type, description, content, file_path, created_at, updated_at \
                 FROM memories ORDER BY updated_at DESC LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![limit as i64], |row| {
                Ok(MemoryRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    memory_type: row.get(2)?,
                    description: row.get(3)?,
                    content: row.get(4)?,
                    file_path: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    pub fn search_memories(&self, query: &str) -> Result<Vec<MemoryRow>, String> {
        let conn = self.connection()?;
        let pattern = format!("%{query}%");
        let mut stmt = conn
            .prepare(
                "SELECT id, name, memory_type, description, content, file_path, created_at, updated_at \
                 FROM memories WHERE name LIKE ?1 OR content LIKE ?1 OR description LIKE ?1 \
                 ORDER BY updated_at DESC LIMIT 50",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![pattern], |row| {
                Ok(MemoryRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    memory_type: row.get(2)?,
                    description: row.get(3)?,
                    content: row.get(4)?,
                    file_path: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    pub fn delete_memory(&self, id: i64) -> Result<bool, String> {
        let conn = self.connection()?;
        let affected = conn
            .execute("DELETE FROM memories WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(affected > 0)
    }

    // -- Telemetry --

    pub fn insert_telemetry(
        &self,
        event_type: &str,
        data: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<i64, String> {
        let conn = self.connection()?;
        let now = Local::now().to_rfc3339();
        conn.execute(
            "INSERT INTO telemetry_events (event_type, data, session_id, created_at) \
             VALUES (?1, ?2, ?3, ?4)",
            params![event_type, data, session_id, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_telemetry(&self, limit: usize) -> Result<Vec<TelemetryRow>, String> {
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, event_type, data, session_id, created_at \
                 FROM telemetry_events ORDER BY id DESC LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![limit as i64], |row| {
                Ok(TelemetryRow {
                    id: row.get(0)?,
                    event_type: row.get(1)?,
                    data: row.get(2)?,
                    session_id: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }
}

/// Global singleton SQL store.
static GLOBAL_SQL_STORE: OnceLock<Arc<Mutex<SqlStore>>> = OnceLock::new();

pub fn global_sql_store(base_dir: &str) -> &'static Arc<Mutex<SqlStore>> {
    GLOBAL_SQL_STORE
        .get_or_init(|| Arc::new(Mutex::new(SqlStore::new(base_dir).expect("sql store init"))))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicU64, Ordering};
    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_store() -> SqlStore {
        let n = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = std::env::temp_dir().join(format!("nocode_sql_test_{}_{n}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        SqlStore::new(tmp.to_str().unwrap()).unwrap()
    }

    #[test]
    fn create_and_list_sessions() {
        let store = test_store();
        store.create_session("sess-1", "claude-sonnet").unwrap();
        store.create_session("sess-2", "claude-opus").unwrap();
        let sessions = store.list_sessions(10).unwrap();
        assert_eq!(sessions.len(), 2);
        let _ = fs::remove_dir_all(&store.base_dir);
    }

    #[test]
    fn insert_and_get_messages() {
        let store = test_store();
        store.create_session("sess-msg", "sonnet").unwrap();
        store
            .insert_message("sess-msg", "user", "hello", 1, 10)
            .unwrap();
        store
            .insert_message("sess-msg", "assistant", "hi there", 1, 20)
            .unwrap();
        let msgs = store.get_messages("sess-msg").unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(store.count_messages("sess-msg").unwrap(), 2);
        let _ = fs::remove_dir_all(&store.base_dir);
    }

    #[test]
    fn update_session_status() {
        let store = test_store();
        store.create_session("sess-upd", "sonnet").unwrap();
        store.update_session("sess-upd", "completed").unwrap();
        let sessions = store.list_sessions(10).unwrap();
        assert_eq!(sessions[0].status, "completed");
        let _ = fs::remove_dir_all(&store.base_dir);
    }

    #[test]
    fn insert_command_history() {
        let store = test_store();
        let id = store.insert_command("cargo build", Some("sess-1")).unwrap();
        assert!(id > 0);
        let cmds = store.list_commands(10).unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].command, "cargo build");
        let _ = fs::remove_dir_all(&store.base_dir);
    }

    #[test]
    fn memory_crud() {
        let store = test_store();
        let id = store
            .insert_memory(
                "user_role",
                "user",
                Some("User is a dev"),
                "Senior Rust dev",
                None,
            )
            .unwrap();
        assert!(id > 0);

        let mems = store.list_memories(10).unwrap();
        assert_eq!(mems.len(), 1);
        assert_eq!(mems[0].name, "user_role");
        assert_eq!(mems[0].memory_type, "user");

        let found = store.search_memories("Rust").unwrap();
        assert_eq!(found.len(), 1);

        let empty = store.search_memories("nonexistent_xyz").unwrap();
        assert!(empty.is_empty());

        assert!(store.delete_memory(id).unwrap());
        assert!(store.list_memories(10).unwrap().is_empty());
        let _ = fs::remove_dir_all(&store.base_dir);
    }

    #[test]
    fn telemetry_crud() {
        let store = test_store();
        let id = store
            .insert_telemetry("tool_call", Some(r#"{"tool":"Bash"}"#), Some("sess-1"))
            .unwrap();
        assert!(id > 0);

        let events = store.list_telemetry(10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "tool_call");
        assert!(events[0].data.as_ref().unwrap().contains("Bash"));
        let _ = fs::remove_dir_all(&store.base_dir);
    }

    #[test]
    fn list_volumes_empty() {
        let store = test_store();
        let vols = store.list_volumes().unwrap();
        assert!(vols.is_empty());
        // After creating a session, there should be a volume
        store.create_session("sess-vol", "sonnet").unwrap();
        let vols = store.list_volumes().unwrap();
        assert_eq!(vols.len(), 1);
        let _ = fs::remove_dir_all(&store.base_dir);
    }
}
