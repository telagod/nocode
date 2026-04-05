use super::{QueryEngineConfig, QueryEngineState, ResumeSnapshot};
use crate::budget_state::BudgetState;
use crate::file_history::{FileHistoryConfig, FileHistoryState};
use crate::history_store::{HistoryStore, HistoryStoreConfig};
use crate::persistence_backend::PersistenceReader;
use crate::session_persistence::{
    ReadFileCacheState, SessionIdentity, SessionPersistenceConfig, SessionPersistenceState,
};
use crate::usage_tracker::{UsageTotals, UsageTracker};
use std::io;

pub(super) fn build_query_engine_state(
    config: &QueryEngineConfig,
    resume_snapshot: ResumeSnapshot,
) -> QueryEngineState {
    let session_persistence = SessionPersistenceState::new(
        SessionPersistenceConfig::new(
            SessionIdentity::new(config.session_id.clone(), config.cwd.clone()),
            config.persist_session,
            ReadFileCacheState::with_entries(config.read_file_cache_entries),
        ),
        config.initial_messages.len(),
    );
    let mut file_history = FileHistoryState::new(if config.file_history_enabled {
        FileHistoryConfig::enabled()
    } else {
        FileHistoryConfig::disabled()
    });
    if let Some(snapshot) = &resume_snapshot.file_history {
        file_history.requested_snapshots = snapshot.total_requests;
        file_history.committed_snapshots = snapshot.total_committed;
    }

    let mut session_persistence = session_persistence;
    session_persistence.restore_resume_counters(
        resume_snapshot.transcript.len(),
        resume_snapshot.history.len(),
    );

    QueryEngineState {
        mutable_messages: config.initial_messages.clone(),
        completed_turns: Vec::new(),
        completed_responses: Vec::new(),
        permission_denials: Vec::new(),
        total_usage: UsageTotals::default(),
        usage_tracker: UsageTracker::default(),
        budget_state: BudgetState::new(config.task_budget),
        history_store: HistoryStore::new(HistoryStoreConfig::new(
            config.persist_history,
            config.cwd.clone(),
        )),
        file_history,
        session_persistence,
        resume_snapshot,
        has_handled_orphaned_permission: false,
        read_file_cache_entries: config.read_file_cache_entries,
    }
}

pub(super) fn load_resume_snapshot(
    config: &QueryEngineConfig,
    reader: &impl PersistenceReader,
) -> io::Result<ResumeSnapshot> {
    let state = SessionPersistenceState::new(
        SessionPersistenceConfig::new(
            SessionIdentity::new(config.session_id.clone(), config.cwd.clone()),
            config.persist_session,
            ReadFileCacheState::with_entries(config.read_file_cache_entries),
        ),
        config.initial_messages.len(),
    );
    let resume_plan = state.build_resume_plan();
    Ok(ResumeSnapshot {
        transcript: reader.read_transcript(&resume_plan)?,
        history: reader.read_history(&resume_plan)?,
        file_history: reader.read_file_history(&resume_plan)?,
    })
}
