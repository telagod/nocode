//! Telemetry — opt-in event logging to JSONL file.
//!
//! Events are written to `~/.nocode/telemetry/events_YYYY-MM-DD.jsonl`.
//! Controlled by feature flag `telemetry` or env `NOCODE_FF_TELEMETRY=1`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

/// Telemetry event types.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    SessionStart,
    SessionEnd,
    ModelCall,
    ToolCall,
    Error,
    CommandRun,
    PermissionDecision,
    Custom,
}

/// A single telemetry event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryEvent {
    pub event_type: EventType,
    pub timestamp: DateTime<Utc>,
    pub session_id: Option<String>,
    /// Event-specific key-value data.
    pub data: serde_json::Value,
}

impl TelemetryEvent {
    pub fn new(event_type: EventType, data: serde_json::Value) -> Self {
        Self {
            event_type,
            timestamp: Utc::now(),
            session_id: None,
            data,
        }
    }

    pub fn with_session(mut self, session_id: &str) -> Self {
// APPEND_MORE
        self.session_id = Some(session_id.to_string());
        self
    }
}

/// JSONL event logger — appends events to daily log files.
pub struct EventLogger {
    base_dir: PathBuf,
    enabled: bool,
    event_count: u64,
}

impl EventLogger {
    pub fn new(base_dir: &str, enabled: bool) -> Self {
        Self {
            base_dir: PathBuf::from(base_dir),
            enabled,
            event_count: 0,
        }
    }

    /// Create a disabled logger (no-op).
    pub fn disabled() -> Self {
        Self {
            base_dir: PathBuf::new(),
            enabled: false,
            event_count: 0,
        }
    }

    /// Log an event. No-op if disabled.
    pub fn log(&mut self, event: &TelemetryEvent) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        self.ensure_dir()?;
        let date = event.timestamp.format("%Y-%m-%d");
        let path = self.base_dir.join(format!("events_{date}.jsonl"));
        let line =
            serde_json::to_string(event).map_err(|e| format!("serialize error: {e}"))?;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("open error: {e}"))?;
        writeln!(file, "{line}").map_err(|e| format!("write error: {e}"))?;
        self.event_count += 1;
        Ok(())
    }

    /// Convenience: log a tool call event.
    pub fn log_tool_call(&mut self, tool_name: &str, duration_ms: u64) {
        let _ = self.log(&TelemetryEvent::new(
            EventType::ToolCall,
            serde_json::json!({
                "tool": tool_name,
                "duration_ms": duration_ms,
            }),
        ));
    }

    /// Convenience: log a model call event.
    pub fn log_model_call(
        &mut self,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        duration_ms: u64,
    ) {
        let _ = self.log(&TelemetryEvent::new(
            EventType::ModelCall,
            serde_json::json!({
                "model": model,
                "input_tokens": input_tokens,
                "output_tokens": output_tokens,
                "duration_ms": duration_ms,
            }),
        ));
    }

    /// Convenience: log an error event.
    pub fn log_error(&mut self, error: &str, context: &str) {
        let _ = self.log(&TelemetryEvent::new(
            EventType::Error,
            serde_json::json!({
                "error": error,
                "context": context,
            }),
        ));
    }

    /// Get total events logged this session.
    pub fn event_count(&self) -> u64 {
        self.event_count
    }

    /// Check if logging is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Enable or disable logging at runtime.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Read events from a specific date file.
    pub fn read_events(&self, date: &str) -> Result<Vec<TelemetryEvent>, String> {
        let path = self.base_dir.join(format!("events_{date}.jsonl"));
        let raw = fs::read_to_string(&path)
            .map_err(|e| format!("read error: {e}"))?;
        let mut events = Vec::new();
        for line in raw.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let event: TelemetryEvent =
                serde_json::from_str(line).map_err(|e| format!("parse error: {e}"))?;
            events.push(event);
        }
        Ok(events)
    }

    /// List available event log dates.
    pub fn list_dates(&self) -> Result<Vec<String>, String> {
        let dir = fs::read_dir(&self.base_dir)
            .map_err(|e| format!("read dir error: {e}"))?;
        let mut dates = Vec::new();
        for entry in dir {
            let entry = entry.map_err(|e| format!("dir entry error: {e}"))?;
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(date) = name.strip_prefix("events_")
                .and_then(|s| s.strip_suffix(".jsonl"))
            {
                dates.push(date.to_string());
            }
        }
        dates.sort();
        Ok(dates)
    }

    fn ensure_dir(&self) -> Result<(), String> {
        fs::create_dir_all(&self.base_dir)
            .map_err(|e| format!("mkdir error: {e}"))
    }
}

/// Global singleton event logger.
static GLOBAL_EVENT_LOGGER: OnceLock<Arc<Mutex<EventLogger>>> = OnceLock::new();

pub fn global_event_logger() -> &'static Arc<Mutex<EventLogger>> {
    GLOBAL_EVENT_LOGGER.get_or_init(|| Arc::new(Mutex::new(EventLogger::disabled())))
}

pub fn init_global_event_logger(logger: EventLogger) {
    let global = global_event_logger();
    let mut guard = global.lock().unwrap_or_else(|e| e.into_inner());
    *guard = logger;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_serialization_roundtrip() {
        let event = TelemetryEvent::new(
            EventType::ToolCall,
            serde_json::json!({"tool": "Bash"}),
        );
        let json = serde_json::to_string(&event).unwrap();
        let parsed: TelemetryEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.event_type, EventType::ToolCall);
    }

    #[test]
    fn event_with_session() {
        let event = TelemetryEvent::new(EventType::SessionStart, serde_json::json!({}))
            .with_session("sess-123");
        assert_eq!(event.session_id.as_deref(), Some("sess-123"));
    }

    #[test]
    fn disabled_logger_is_noop() {
        let mut logger = EventLogger::disabled();
        let event = TelemetryEvent::new(EventType::Error, serde_json::json!({}));
        assert!(logger.log(&event).is_ok());
        assert_eq!(logger.event_count(), 0);
        assert!(!logger.is_enabled());
    }

    #[test]
    fn logger_writes_and_reads() {
        let tmp = format!("/tmp/nocode_telem_test_{}", std::process::id());
        let _ = fs::remove_dir_all(&tmp);
        let mut logger = EventLogger::new(&tmp, true);
        logger.log_tool_call("Bash", 42);
        logger.log_model_call("claude", 100, 50, 1000);
        logger.log_error("timeout", "model_call");
        assert_eq!(logger.event_count(), 3);

        let today = Utc::now().format("%Y-%m-%d").to_string();
        let events = logger.read_events(&today).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].event_type, EventType::ToolCall);
        assert_eq!(events[1].event_type, EventType::ModelCall);
        assert_eq!(events[2].event_type, EventType::Error);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn logger_list_dates() {
        let tmp = format!("/tmp/nocode_telem_test2_{}", std::process::id());
        let _ = fs::remove_dir_all(&tmp);
        let mut logger = EventLogger::new(&tmp, true);
        logger.log_tool_call("Grep", 10);
        let dates = logger.list_dates().unwrap();
        assert!(!dates.is_empty());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn logger_toggle_enabled() {
        let mut logger = EventLogger::disabled();
        assert!(!logger.is_enabled());
        logger.set_enabled(true);
        assert!(logger.is_enabled());
    }

    #[test]
    fn event_types_all_variants() {
        let types = [
            EventType::SessionStart,
            EventType::SessionEnd,
            EventType::ModelCall,
            EventType::ToolCall,
            EventType::Error,
            EventType::CommandRun,
            EventType::PermissionDecision,
            EventType::Custom,
        ];
        for t in &types {
            let json = serde_json::to_string(t).unwrap();
            let parsed: EventType = serde_json::from_str(&json).unwrap();
            assert_eq!(&parsed, t);
        }
    }
}
