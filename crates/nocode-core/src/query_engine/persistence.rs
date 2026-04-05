use super::QueryEngine;
use crate::file_history::FileHistoryPlan;
use crate::history_store::HistoryStorePlan;
use crate::persistence_backend::{
    LocalPersistenceBackend, PersistenceBackend, PersistenceDispatchResult,
};
use crate::session_persistence::SessionPersistencePlan;
use crate::transcript::QueryTranscript;

pub(super) fn build_local_persistence_backend(engine: &QueryEngine) -> LocalPersistenceBackend {
    LocalPersistenceBackend::new(
        engine
            .state()
            .session_persistence
            .config
            .identity
            .transcript_path(),
        engine
            .state()
            .session_persistence
            .config
            .identity
            .history_path(),
        engine
            .state()
            .session_persistence
            .config
            .identity
            .file_history_path(),
    )
}

pub(super) fn persist_submission(
    backend: &mut impl PersistenceBackend,
    transcript: &QueryTranscript,
    history_store: &HistoryStorePlan,
    file_history: &FileHistoryPlan,
    session_persistence: &SessionPersistencePlan,
) -> PersistenceDispatchResult {
    let transcript_entries = transcript
        .entries
        .iter()
        .map(|entry| format!("{}\t{}\t{}", entry.turn, entry.role.as_str(), entry.content))
        .collect::<Vec<_>>();
    let transcript_entries_flushed = backend.persist_transcript(&transcript_entries);
    let history_persisted = backend.persist_history(history_store);
    let file_history_persisted = backend.persist_file_history(file_history);
    backend.finalize(session_persistence);

    PersistenceDispatchResult {
        transcript_entries_flushed,
        history_persisted,
        file_history_persisted,
    }
}
