//! Telemetry subsystem — event recording, session tracing, and global log.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// Identity of the client application emitting telemetry.
#[derive(Debug, Clone)]
pub struct ClientIdentity {
    pub app_name: String,
    pub app_version: String,
    pub runtime_id: String,
}

/// Discrete events the system can emit.
#[derive(Debug, Clone)]
pub enum TelemetryEvent {
    SessionStarted {
        session_id: String,
        model: String,
    },
    SessionEnded {
        session_id: String,
        total_turns: u32,
        total_tokens: u64,
    },
    TurnStarted {
        session_id: String,
        turn: u32,
    },
    TurnCompleted {
        session_id: String,
        turn: u32,
        input_tokens: u64,
        output_tokens: u64,
        duration_ms: u64,
    },
    ToolExecuted {
        session_id: String,
        tool_name: String,
        success: bool,
        duration_ms: u64,
    },
    ModelError {
        session_id: String,
        error_type: String,
        retryable: bool,
    },
    CompactionTriggered {
        session_id: String,
        removed_messages: usize,
    },
    MemorySaved {
        memory_type: String,
        name: String,
    },
    HookExecuted {
        event: String,
        tool_name: String,
        denied: bool,
    },
}

/// A timestamped, sequenced wrapper around a [`TelemetryEvent`].
#[derive(Debug, Clone)]
pub struct TelemetryRecord {
    pub event: TelemetryEvent,
    pub timestamp_ms: u64,
    pub sequence: u64,
}

// ---------------------------------------------------------------------------
// TelemetrySink trait + MemoryTelemetrySink
// ---------------------------------------------------------------------------

/// Pluggable destination for telemetry records.
pub trait TelemetrySink: Send + Sync + std::fmt::Debug {
    fn record(&self, record: &TelemetryRecord);
}

/// In-memory sink — useful for tests and short-lived processes.
#[derive(Debug, Default)]
pub struct MemoryTelemetrySink {
    records: Mutex<Vec<TelemetryRecord>>,
}

impl MemoryTelemetrySink {
    pub fn new() -> Self {
        Self {
            records: Mutex::new(Vec::new()),
        }
    }

    pub fn records(&self) -> Vec<TelemetryRecord> {
        self.records
            .lock()
            .expect("telemetry lock poisoned")
            .clone()
    }

    pub fn clear(&self) {
        self.records
            .lock()
            .expect("telemetry lock poisoned")
            .clear();
    }

    pub fn count(&self) -> usize {
        self.records.lock().expect("telemetry lock poisoned").len()
    }
}

impl TelemetrySink for MemoryTelemetrySink {
    fn record(&self, record: &TelemetryRecord) {
        self.records
            .lock()
            .expect("telemetry lock poisoned")
            .push(record.clone());
    }
}

// ---------------------------------------------------------------------------
// SessionTracer
// ---------------------------------------------------------------------------

/// Per-session helper that stamps events with identity and sequence numbers,
/// then forwards them to a [`TelemetrySink`].
#[derive(Debug)]
pub struct SessionTracer {
    session_id: String,
    #[allow(dead_code)]
    identity: ClientIdentity,
    sink: Arc<dyn TelemetrySink>,
    sequence: u64,
}

impl SessionTracer {
    pub fn new(session_id: &str, identity: ClientIdentity, sink: Arc<dyn TelemetrySink>) -> Self {
        Self {
            session_id: session_id.to_owned(),
            identity,
            sink,
            sequence: 0,
        }
    }

    pub fn emit(&mut self, event: TelemetryEvent) {
        let record = TelemetryRecord {
            event,
            timestamp_ms: now_ms(),
            sequence: self.sequence,
        };
        self.sink.record(&record);
        self.sequence += 1;
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }
}

// ---------------------------------------------------------------------------
// Global telemetry log (simple shared vec approach)
// ---------------------------------------------------------------------------

static TELEMETRY_SINK: OnceLock<Arc<Mutex<Vec<TelemetryRecord>>>> = OnceLock::new();

/// Returns the process-wide telemetry log (created on first call).
pub fn global_telemetry_log() -> Arc<Mutex<Vec<TelemetryRecord>>> {
    TELEMETRY_SINK
        .get_or_init(|| Arc::new(Mutex::new(Vec::new())))
        .clone()
}

/// Convenience: push an event into the global log.
pub fn emit_telemetry(event: TelemetryEvent) {
    let log = global_telemetry_log();
    if let Ok(mut records) = log.lock() {
        let seq = records.len() as u64;
        records.push(TelemetryRecord {
            event,
            timestamp_ms: now_ms(),
            sequence: seq,
        });
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_identity() -> ClientIdentity {
        ClientIdentity {
            app_name: "nocode-test".into(),
            app_version: "0.1.0".into(),
            runtime_id: "test-run-1".into(),
        }
    }

    #[test]
    fn emit_records_event() {
        let sink = Arc::new(MemoryTelemetrySink::new());
        let mut tracer = SessionTracer::new("s1", sample_identity(), sink.clone());

        tracer.emit(TelemetryEvent::SessionStarted {
            session_id: "s1".into(),
            model: "test-model".into(),
        });

        assert_eq!(sink.count(), 1);
        let recs = sink.records();
        assert!(matches!(
            &recs[0].event,
            TelemetryEvent::SessionStarted { session_id, model }
                if session_id == "s1" && model == "test-model"
        ));
    }

    #[test]
    fn sequence_increments() {
        let sink = Arc::new(MemoryTelemetrySink::new());
        let mut tracer = SessionTracer::new("s2", sample_identity(), sink.clone());

        for i in 0..5 {
            tracer.emit(TelemetryEvent::TurnStarted {
                session_id: "s2".into(),
                turn: i,
            });
        }

        let recs = sink.records();
        let seqs: Vec<u64> = recs.iter().map(|r| r.sequence).collect();
        assert_eq!(seqs, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn multiple_events_tracked() {
        let sink = Arc::new(MemoryTelemetrySink::new());
        let mut tracer = SessionTracer::new("s3", sample_identity(), sink.clone());

        tracer.emit(TelemetryEvent::SessionStarted {
            session_id: "s3".into(),
            model: "m".into(),
        });
        tracer.emit(TelemetryEvent::ToolExecuted {
            session_id: "s3".into(),
            tool_name: "bash".into(),
            success: true,
            duration_ms: 42,
        });
        tracer.emit(TelemetryEvent::SessionEnded {
            session_id: "s3".into(),
            total_turns: 1,
            total_tokens: 100,
        });

        assert_eq!(sink.count(), 3);
        sink.clear();
        assert_eq!(sink.count(), 0);
    }

    #[test]
    fn session_tracer_emits_to_sink() {
        let sink = Arc::new(MemoryTelemetrySink::new());
        let mut tracer = SessionTracer::new("sess-abc", sample_identity(), sink.clone());

        assert_eq!(tracer.session_id(), "sess-abc");

        tracer.emit(TelemetryEvent::ModelError {
            session_id: "sess-abc".into(),
            error_type: "rate_limit".into(),
            retryable: true,
        });

        let recs = sink.records();
        assert_eq!(recs.len(), 1);
        assert!(matches!(
            &recs[0].event,
            TelemetryEvent::ModelError { retryable, .. } if *retryable
        ));
    }

    #[test]
    fn global_telemetry_log_singleton() {
        let log_a = global_telemetry_log();
        let log_b = global_telemetry_log();
        // Both point to the same allocation.
        assert!(Arc::ptr_eq(&log_a, &log_b));
    }

    #[test]
    fn telemetry_event_variants() {
        // Ensure every variant can be constructed and debug-printed.
        let events: Vec<TelemetryEvent> = vec![
            TelemetryEvent::SessionStarted {
                session_id: "s".into(),
                model: "m".into(),
            },
            TelemetryEvent::SessionEnded {
                session_id: "s".into(),
                total_turns: 0,
                total_tokens: 0,
            },
            TelemetryEvent::TurnStarted {
                session_id: "s".into(),
                turn: 0,
            },
            TelemetryEvent::TurnCompleted {
                session_id: "s".into(),
                turn: 0,
                input_tokens: 0,
                output_tokens: 0,
                duration_ms: 0,
            },
            TelemetryEvent::ToolExecuted {
                session_id: "s".into(),
                tool_name: "t".into(),
                success: true,
                duration_ms: 0,
            },
            TelemetryEvent::ModelError {
                session_id: "s".into(),
                error_type: "e".into(),
                retryable: false,
            },
            TelemetryEvent::CompactionTriggered {
                session_id: "s".into(),
                removed_messages: 0,
            },
            TelemetryEvent::MemorySaved {
                memory_type: "user".into(),
                name: "n".into(),
            },
            TelemetryEvent::HookExecuted {
                event: "tool_call".into(),
                tool_name: "bash".into(),
                denied: false,
            },
        ];

        assert_eq!(events.len(), 9);
        for e in &events {
            // Debug must not panic.
            let _ = format!("{e:?}");
        }
    }
}
