use super::support::*;
use crate::query_loop::QueryLoopOutcome;

#[test]
fn engine_state_is_seeded_from_config() {
    let engine = QueryEngine::new(sample_config());
    assert_eq!(
        engine.state().mutable_messages,
        vec![SharedQueryMessage::system("seed")]
    );
    assert!(engine.state().completed_turns.is_empty());
    assert!(engine.state().completed_responses.is_empty());
    assert_eq!(engine.state().total_usage.input_tokens, 0);
    assert_eq!(engine.state().usage_tracker.completed_turns(), 0);
    assert_eq!(
        engine.state().budget_state.current_turn_budget,
        Some(20_000)
    );
    assert_eq!(
        engine
            .state()
            .session_persistence
            .config
            .identity
            .session_id,
        "session-1"
    );
    assert_eq!(engine.state().history_store.pending_count(), 0);
    assert_eq!(engine.state().file_history.committed_snapshots, 0);
    assert!(engine.state().resume_snapshot.transcript.is_empty());
    assert_eq!(engine.state().read_file_cache_entries, 2);
    assert_eq!(engine.build_query_config().selected_model(), Some("sonnet"));
    assert_eq!(
        engine.build_query_config().model_selection.provider,
        SharedModelProvider::Mock
    );
}

#[test]
fn resume_with_reader_loads_persisted_snapshot() {
    let reader = RecordingPersistenceReader {
        transcript: vec![SharedPersistedTranscriptEntry {
            turn: 3,
            role: SharedTranscriptRole::ToolResult,
            content: String::from("Read completed"),
        }],
        history: vec![SharedHistoryEntry {
            display: String::from("rewrite query loop"),
            timestamp: 7,
            session_id: String::from("session-1"),
            project: repo_root(),
        }],
        file_history: Some(SharedFileHistorySnapshot {
            snapshot_requested: true,
            total_requests: 4,
            total_committed: 3,
        }),
    };

    let engine = QueryEngine::resume_with_reader(sample_config(), &reader)
        .expect("resume should build engine");

    assert_eq!(engine.state().resume_snapshot.transcript.len(), 1);
    assert_eq!(engine.state().resume_snapshot.history.len(), 1);
    assert_eq!(
        engine.state().resume_snapshot.file_history,
        Some(SharedFileHistorySnapshot {
            snapshot_requested: true,
            total_requests: 4,
            total_committed: 3,
        })
    );
    assert_eq!(engine.state().file_history.requested_snapshots, 4);
    assert_eq!(engine.state().file_history.committed_snapshots, 3);
    assert_eq!(engine.state().session_persistence.transcript_flushes, 1);
    assert_eq!(engine.state().session_persistence.history_entries, 1);
    assert_eq!(engine.state().session_persistence.transcript_entries, 1);
}

#[test]
fn submit_message_uses_custom_deps_for_ids_and_stop_hooks() {
    let deps = SharedQueryDeps::builder()
        .with_clock(FixedClock)
        .with_id_gen(FixedIdGen)
        .with_stop_hook_runner(BlockingStopHook)
        .with_call_model(PanicCallModel)
        .build();
    let mut engine = QueryEngine::with_deps(sample_config(), deps);

    let plan = engine.submit_message("hooked turn", SubmitMessageOptions::default());

    assert_eq!(plan.model_response.response_id, "resp-custom");
    assert_eq!(
        plan.steps
            .iter()
            .map(|step| step.action.as_str())
            .collect::<Vec<_>>(),
        vec![
            "bind-tool-context",
            "request-tool:Read",
            "tool-progress:toolu-bootstrap-1",
            "resolve-tool:completed",
            "request-tool:Read",
            "tool-progress:toolu-bootstrap-2",
            "resolve-tool:completed",
            "request-tool:Read",
            "tool-progress:toolu-bootstrap-3",
            "resolve-tool:completed",
            "flush-tool-batch",
            "stop-hooks:blocking",
            "continue:stop-hook-blocking",
        ]
    );
    match &plan.loop_outcome {
        QueryLoopOutcome::Continue(state) => {
            assert!(state.stop_hook_active);
            assert_eq!(
                state.transition,
                Some(QueryLoopContinueReason::StopHookBlocking)
            );
            assert!(state.last_stop_hook_result.is_some());
        }
        QueryLoopOutcome::Terminal(reason) => panic!("unexpected terminal outcome: {reason:?}"),
    }
    assert_eq!(
        engine
            .state()
            .completed_responses
            .first()
            .map(|response| response.response_id.as_str()),
        Some("resp-custom")
    );
    assert!(engine.state().session_persistence.transcript_entries > 19);
}

#[test]
fn submit_message_with_custom_compactors_rewrites_session_messages() {
    let deps = SharedQueryDeps::builder()
        .with_microcompact(CollapseCompactor)
        .with_autocompact(CollapseCompactor)
        .build();
    let mut engine = QueryEngine::with_deps(sample_config(), deps);

    let plan = engine.submit_message("compact this turn", SubmitMessageOptions::default());

    assert_eq!(plan.loop_params.messages.len(), 3);
    assert_eq!(
        plan.loop_params.messages[0],
        SharedQueryMessage::system("compacted")
    );
    assert_eq!(engine.state().mutable_messages.len(), 2);
    assert_eq!(
        engine.state().mutable_messages[0],
        SharedQueryMessage::system("compacted")
    );
}

#[test]
fn submit_message_with_executor_can_fall_back_to_deps_tool_runner() {
    let deps = SharedQueryDeps::builder()
        .with_tool_runner(RescuingToolRunner)
        .build();
    let mut engine = QueryEngine::with_deps(sample_config(), deps);

    let plan = engine.submit_message_with_executor(
        "rescue tools",
        SubmitMessageOptions::default(),
        &FailingExecutor,
    );

    assert_eq!(
        plan.tool_results
            .iter()
            .map(SharedToolCallResult::status_label)
            .collect::<Vec<_>>(),
        vec!["completed", "completed", "completed"]
    );
    assert!(
        plan.assistant_turn
            .response_messages
            .iter()
            .any(|message| message.content.contains("rescued toolu-bootstrap-1"))
    );
    assert_eq!(
        plan.model_response.final_assistant_message,
        Some(SharedQueryMessage::assistant(
            "nocode response: rescue tools"
        ))
    );
}
