use super::support::*;
use crate::assistant_turn::AssistantTurnStatus;
use crate::provider::RecordingModelStream;
use crate::query_loop::QueryLoopOutcome;
use serde_json::json;

#[test]
fn submit_message_updates_state_and_builds_loop_plan() {
    let mut engine = QueryEngine::new(sample_config());
    let plan = engine.submit_message(
        "rewrite query loop",
        SubmitMessageOptions {
            uuid: Some(String::from("msg-1")),
            is_meta: false,
        },
    );

    assert_eq!(plan.message_count_after_submit, 9);
    assert_eq!(plan.prompt_uuid.as_deref(), Some("msg-1"));
    assert_eq!(plan.query_config.selected_model(), Some("sonnet"));
    assert_eq!(
        plan.query_config.model_selection.requested_model.as_deref(),
        Some("sonnet")
    );
    assert_eq!(plan.query_config.fallback_model(), Some("haiku"));
    assert!(plan.query_config.runtime_gates.verbose);
    assert!(plan.query_config.runtime_gates.stream_model_responses);
    assert_eq!(
        plan.query_config.task_budget,
        Some(TaskBudget { total: 20_000 })
    );
    assert_eq!(plan.budget_state.current_turn_budget, Some(20_000));
    assert_eq!(
        plan.available_tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Read", "Edit"]
    );
    assert_step(&plan.steps[0], "bind-tool-context");
    assert_step(&plan.steps[1], "request-tool:Read");
    assert_step(&plan.steps[2], "tool-progress:toolu-bootstrap-1");
    assert_step(&plan.steps[3], "resolve-tool:completed");
    assert_step(&plan.steps[4], "request-tool:Read");
    assert_step(&plan.steps[5], "tool-progress:toolu-bootstrap-2");
    assert_step(&plan.steps[6], "resolve-tool:completed");
    assert_step(&plan.steps[7], "request-tool:Read");
    assert_step(&plan.steps[8], "tool-progress:toolu-bootstrap-3");
    assert_step(&plan.steps[9], "resolve-tool:completed");
    assert_step(&plan.steps[10], "flush-tool-batch");
    assert_step(&plan.steps[11], "token-budget:continue");
    assert_step(&plan.steps[12], "call-model");
    assert_eq!(plan.steps[13].action, "complete");
    assert!(matches!(
        plan.steps[13].outcome,
        QueryLoopOutcome::Terminal(crate::query_loop::QueryLoopTerminal::Completed)
    ));
    assert_seeded_plan(&plan);
    assert_eq!(engine.state().completed_turns.len(), 1);
    assert_eq!(engine.state().completed_responses.len(), 1);
    assert_eq!(engine.state().completed_turns[0], plan.assistant_turn);
    assert_eq!(engine.state().completed_responses[0], plan.model_response);
    assert_eq!(engine.state().usage_tracker.completed_turns(), 1);
    assert_eq!(engine.state().total_usage, plan.usage_snapshot.total_usage);
    assert_eq!(engine.state().budget_state, plan.budget_state);
    assert_eq!(plan.history_store.session_id, "session-1");
    assert_eq!(plan.history_store.pending_entries, 1);
    assert_eq!(plan.history_store.flush_count, 1);
    assert!(plan.file_history.snapshot_requested);
    assert_eq!(plan.file_history.total_requests, 1);
    assert_eq!(plan.file_history.total_committed, 1);
    assert!(plan.usage_snapshot.input_tokens > 0);
    assert!(plan.usage_snapshot.output_tokens > 0);
    assert_eq!(plan.usage_snapshot.turn_index, 1);
    assert!(plan.budget_state.current_turn_output_tokens > 0);
    assert_eq!(plan.budget_state.continuation_count(), 1);
    assert!(plan.session_persistence.persist_session);
    assert_eq!(plan.session_persistence.history_entries, 1);
    assert_eq!(plan.session_persistence.transcript_entries, 20);
    assert_eq!(plan.session_persistence.history_flushes, 1);
    assert_eq!(plan.session_persistence.history_pending_entries, 1);
    assert!(plan.session_persistence.file_history_requested);
    assert_eq!(plan.session_persistence.file_history_requests, 1);
    assert_eq!(plan.session_persistence.file_history_committed, 1);
    assert_eq!(
        engine.state().mutable_messages[1],
        SharedQueryMessage::user("rewrite query loop")
    );
    assert!(engine.state().mutable_messages.iter().any(|message| {
        message
            .content
            .contains("tool-result:Read#toolu-bootstrap-1")
    }));
    assert_eq!(
        plan.model_response.final_assistant_message,
        Some(SharedQueryMessage::assistant(
            "nocode response: rewrite query loop"
        ))
    );
    assert_eq!(
        plan.model_invocation
            .as_ref()
            .map(|invocation| invocation.provider),
        Some(SharedModelProvider::Mock)
    );
    assert_eq!(
        plan.model_invocation
            .as_ref()
            .map(|invocation| invocation.stream_mode),
        Some(SharedModelStreamMode::Enabled)
    );
    assert_eq!(
        engine.state().mutable_messages.last(),
        Some(&SharedQueryMessage::assistant(
            "nocode response: rewrite query loop"
        ))
    );
    assert_eq!(
        plan.loop_outcome,
        QueryLoopOutcome::Terminal(crate::query_loop::QueryLoopTerminal::Completed)
    );
}

#[test]
fn ask_creates_one_shot_submission_plan() {
    let plan = ask(
        sample_config(),
        "one shot",
        SubmitMessageOptions {
            uuid: None,
            is_meta: true,
        },
    );

    assert!(plan.is_meta);
    assert_eq!(plan.prompt, "one shot");
    assert_eq!(
        plan.loop_params
            .messages
            .last()
            .map(|message| message.content.as_str()),
        Some("one shot")
    );
    assert_eq!(
        plan.requested_tools
            .iter()
            .map(|call| call.summary())
            .collect::<Vec<_>>(),
        vec![
            String::from("Read#toolu-bootstrap-1(file_path=src/query.ts)"),
            String::from("Read#toolu-bootstrap-2(file_path=src/QueryEngine.ts)"),
            String::from("Read#toolu-bootstrap-3(file_path=src/services/tools/toolExecution.ts)"),
        ]
    );
    assert_eq!(
        plan.tool_results
            .iter()
            .map(SharedToolCallResult::status_label)
            .collect::<Vec<_>>(),
        vec!["completed", "completed", "completed"]
    );
    assert_eq!(plan.usage_snapshot.turn_index, 1);
    assert!(plan.usage_snapshot.total_usage.input_tokens > 0);
    assert!(plan.session_persistence.persist_session);
    assert_eq!(plan.history_store.flush_count, 1);
    assert_eq!(plan.session_persistence.history_flushes, 1);
    assert_seeded_plan(&plan);
}

#[test]
fn submit_message_with_stream_fans_out_runtime_events() {
    let deps = SharedQueryDeps::builder()
        .with_call_model(FixedCallModel)
        .build();
    let mut engine = QueryEngine::with_deps(sample_config(), deps);
    let mut stream = RecordingModelStream::default();

    let plan = engine.submit_message_with_stream(
        "stream outward",
        SubmitMessageOptions::default(),
        &mut stream,
    );

    let invocation = plan
        .model_invocation
        .as_ref()
        .expect("provider invocation should be recorded");
    assert_eq!(stream.events, invocation.stream_events);
    assert_eq!(
        stream
            .events
            .iter()
            .map(crate::provider::ModelStreamEvent::kind_label)
            .collect::<Vec<_>>(),
        vec!["start", "delta", "complete"]
    );
}

#[test]
fn submit_message_persists_with_local_backend_by_default() {
    let mut engine = QueryEngine::new(sample_config());
    let plan = engine.submit_message("persist default", SubmitMessageOptions::default());

    assert_eq!(
        plan.persistence_dispatch.transcript_entries_flushed,
        plan.transcript.entries.len()
    );
    assert!(plan.persistence_dispatch.history_persisted);
    assert!(plan.persistence_dispatch.file_history_persisted);
}

#[test]
fn submit_message_records_permission_denial_when_read_is_filtered_out() {
    let mut config = sample_config();
    config.tools = vec![String::from("Edit"), String::from("Read")];
    config.tool_permission_context = ToolPermissionContext::default().deny("Read");
    let mut engine = QueryEngine::new(config);
    let plan = engine.submit_message("rewrite without read", SubmitMessageOptions::default());

    assert_eq!(
        plan.available_tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Edit"]
    );
    assert_eq!(
        plan.unavailable_tools
            .iter()
            .map(|issue| issue.tool_name.as_str())
            .collect::<Vec<_>>(),
        vec!["Read"]
    );
    assert_eq!(plan.requested_tools.len(), 1);
    assert_eq!(plan.tool_results[0].status_label(), "denied");
    assert_eq!(
        plan.steps
            .iter()
            .map(|step| step.action.as_str())
            .collect::<Vec<_>>(),
        vec![
            "bind-tool-context",
            "request-tool:Read",
            "resolve-tool:denied",
            "flush-tool-batch",
            "token-budget:continue",
            "call-model",
            "complete"
        ]
    );
    assert_eq!(plan.transcript.entries.len(), 7);
    assert_eq!(plan.assistant_turn.response_messages.len(), 2);
    assert_eq!(plan.assistant_turn.tool_uses[0].status, "denied");
    assert_eq!(
        plan.model_response.stop_reason,
        SharedModelResponseStopReason::Completed
    );
    assert_eq!(plan.model_response.tool_phase.requested_tools, 1);
    assert_eq!(plan.model_response.tool_phase.resolved_tools, 1);
    assert_eq!(
        plan.model_response.final_assistant_message,
        Some(SharedQueryMessage::assistant(
            "nocode response: rewrite without read"
        ))
    );
    assert_eq!(
        engine.state().permission_denials,
        vec![String::from("tool denied by permission context")]
    );
    assert_eq!(plan.file_history.total_committed, 1);
}

#[test]
fn submit_message_can_use_custom_executor() {
    let plan = ask_with_executor(
        sample_config(),
        "one shot",
        SubmitMessageOptions::default(),
        &FailingExecutor,
    );

    assert_eq!(
        plan.tool_results
            .iter()
            .map(SharedToolCallResult::status_label)
            .collect::<Vec<_>>(),
        vec!["failed", "failed", "failed"]
    );
    assert_eq!(
        plan.steps
            .iter()
            .map(|step| step.action.as_str())
            .collect::<Vec<_>>(),
        vec![
            "bind-tool-context",
            "request-tool:Read",
            "resolve-tool:failed",
            "request-tool:Read",
            "resolve-tool:failed",
            "request-tool:Read",
            "resolve-tool:failed",
            "flush-tool-batch",
            "token-budget:continue",
            "call-model",
            "complete"
        ]
    );
    assert_eq!(plan.transcript.entries.len(), 11);
    assert_eq!(plan.assistant_turn.response_messages.len(), 4);
    assert!(
        plan.assistant_turn
            .tool_uses
            .iter()
            .all(|tool_use| tool_use.status == "failed")
    );
    assert_eq!(
        plan.model_response.stop_reason,
        SharedModelResponseStopReason::Completed
    );
    assert_eq!(plan.model_response.tool_phase.requested_tools, 3);
    assert_eq!(plan.model_response.tool_phase.resolved_tools, 3);
    assert_eq!(
        plan.model_response.final_assistant_message,
        Some(SharedQueryMessage::assistant("nocode response: one shot"))
    );
    assert_eq!(
        plan.requested_tools,
        vec![
            SharedToolCallInput::new("Read", "toolu-bootstrap-1")
                .with_argument("file_path", "src/query.ts")
                .with_context_label("sdk-bootstrap"),
            SharedToolCallInput::new("Read", "toolu-bootstrap-2")
                .with_argument("file_path", "src/QueryEngine.ts")
                .with_context_label("sdk-bootstrap"),
            SharedToolCallInput::new("Read", "toolu-bootstrap-3")
                .with_argument("file_path", "src/services/tools/toolExecution.ts")
                .with_context_label("sdk-bootstrap"),
        ]
    );
}

#[test]
fn submit_message_can_use_custom_call_model() {
    let deps = SharedQueryDeps::builder()
        .with_call_model(FixedCallModel)
        .build();
    let mut engine = QueryEngine::with_deps(sample_config(), deps);

    let plan = engine.submit_message("model turn", SubmitMessageOptions::default());

    assert_eq!(plan.steps[12].action, "call-model");
    assert_eq!(plan.steps[13].action, "complete");
    assert_eq!(plan.assistant_turn.status, AssistantTurnStatus::Completed);
    assert_eq!(
        plan.model_invocation
            .as_ref()
            .map(|invocation| invocation.model.as_str()),
        Some("sonnet")
    );
    assert_eq!(
        plan.model_invocation
            .as_ref()
            .map(|invocation| invocation.stream_events.len()),
        Some(3)
    );
    let final_message = plan
        .model_response
        .final_assistant_message
        .as_ref()
        .expect("custom call model should emit assistant message");
    assert_eq!(
        final_message.role,
        crate::message::QueryMessageRole::Assistant
    );
    assert_eq!(final_message.content, "model-output:mock:sonnet:model turn");
}

#[test]
fn submit_message_records_model_error_when_provider_fails() {
    let deps = SharedQueryDeps::builder()
        .with_call_model(FailingCallModel)
        .build();
    let mut engine = QueryEngine::with_deps(sample_config(), deps);

    let plan = engine.submit_message("broken model", SubmitMessageOptions::default());

    assert_eq!(
        plan.steps.last().map(|step| step.action.as_str()),
        Some("call-model:error")
    );
    assert!(plan.model_invocation.is_none());
    assert_eq!(
        plan.model_response.stop_reason,
        SharedModelResponseStopReason::Terminal
    );
    assert_eq!(plan.assistant_turn.status, AssistantTurnStatus::Terminal);
    assert!(
        plan.model_response
            .final_assistant_message
            .as_ref()
            .is_some_and(|message| message.content.contains("tool-message:"))
    );
    assert_eq!(
        plan.loop_outcome,
        QueryLoopOutcome::Terminal(crate::query_loop::QueryLoopTerminal::ModelError {
            error: crate::provider::ModelError::provider_failure("provider timeout", true)
        })
    );
    assert_eq!(
        plan.model_error,
        Some(crate::provider::ModelError::provider_failure(
            "provider timeout",
            true
        ))
    );
}

#[test]
fn submit_message_builds_protocol_specific_http_requests() {
    let cases = [
        (
            SharedModelProvider::ClaudeMessages,
            "/v1/messages",
            "\"messages\"",
        ),
        (
            SharedModelProvider::OpenAiChatCompletions,
            "/v1/chat/completions",
            "\"messages\"",
        ),
        (
            SharedModelProvider::OpenAiResponses,
            "/v1/responses",
            "\"input\"",
        ),
    ];

    for (provider, path, marker) in cases {
        let mut config = sample_config();
        config.model_provider = provider;
        let deps = SharedQueryDeps::builder()
            .with_call_model(FixedCallModel)
            .build();
        let mut engine = QueryEngine::with_deps(config, deps);

        let plan = engine.submit_message("adapter turn", SubmitMessageOptions::default());
        let invocation = plan
            .model_invocation
            .as_ref()
            .expect("provider invocation should be recorded");

        assert_eq!(invocation.provider, provider);
        assert_eq!(invocation.http_request.path, path);
        assert!(invocation.http_request.body.contains(marker));
        assert_eq!(invocation.http_request.method, "POST");
    }
}

#[test]
fn submit_message_adds_json_schema_to_openai_chat_completions_requests() {
    let mut config = sample_config();
    config.model_provider = SharedModelProvider::OpenAiChatCompletions;
    config.json_schema = Some(String::from(
        "{\"type\":\"object\",\"properties\":{\"ok\":{\"type\":\"boolean\"}},\"required\":[\"ok\"]}",
    ));
    let deps = SharedQueryDeps::builder()
        .with_call_model(FixedCallModel)
        .build();
    let mut engine = QueryEngine::with_deps(config, deps);

    let plan = engine.submit_message("adapter turn", SubmitMessageOptions::default());
    let invocation = plan
        .model_invocation
        .as_ref()
        .expect("provider invocation should be recorded");

    assert_eq!(
        invocation.provider,
        SharedModelProvider::OpenAiChatCompletions
    );
    assert_eq!(invocation.http_request.path, "/v1/chat/completions");
    assert!(invocation.http_request.body.contains("\"response_format\""));
    assert!(invocation.http_request.body.contains("\"json_schema\""));
    assert!(
        invocation
            .http_request
            .body
            .contains("\"structured_output\"")
    );
}

#[test]
fn submit_message_adds_reasoning_effort_to_openai_chat_completions_requests() {
    let mut config = sample_config();
    config.model_provider = SharedModelProvider::OpenAiChatCompletions;
    config.model_reasoning_effort = Some(String::from("high"));
    let deps = SharedQueryDeps::builder()
        .with_call_model(FixedCallModel)
        .build();
    let mut engine = QueryEngine::with_deps(config, deps);

    let plan = engine.submit_message("adapter turn", SubmitMessageOptions::default());
    let invocation = plan
        .model_invocation
        .as_ref()
        .expect("provider invocation should be recorded");

    assert_eq!(invocation.http_request.path, "/v1/chat/completions");
    assert!(
        invocation
            .http_request
            .body
            .contains("\"reasoning_effort\":\"high\"")
    );
}

#[test]
fn submit_message_adds_json_schema_to_openai_responses_requests() {
    let mut config = sample_config();
    config.model_provider = SharedModelProvider::OpenAiResponses;
    config.json_schema = Some(String::from(
        "{\"type\":\"object\",\"properties\":{\"ok\":{\"type\":\"boolean\"}},\"required\":[\"ok\"]}",
    ));
    let deps = SharedQueryDeps::builder()
        .with_call_model(FixedCallModel)
        .build();
    let mut engine = QueryEngine::with_deps(config, deps);

    let plan = engine.submit_message("adapter turn", SubmitMessageOptions::default());
    let invocation = plan
        .model_invocation
        .as_ref()
        .expect("provider invocation should be recorded");

    assert_eq!(invocation.provider, SharedModelProvider::OpenAiResponses);
    assert_eq!(invocation.http_request.path, "/v1/responses");
    assert!(invocation.http_request.body.contains("\"text\""));
    assert!(invocation.http_request.body.contains("\"format\""));
    assert!(invocation.http_request.body.contains("\"json_schema\""));
    assert!(
        invocation
            .http_request
            .body
            .contains("\"structured_output\"")
    );
}

#[test]
fn submit_message_adds_reasoning_effort_to_openai_responses_requests() {
    let mut config = sample_config();
    config.model_provider = SharedModelProvider::OpenAiResponses;
    config.model_reasoning_effort = Some(String::from("high"));
    let deps = SharedQueryDeps::builder()
        .with_call_model(FixedCallModel)
        .build();
    let mut engine = QueryEngine::with_deps(config, deps);

    let plan = engine.submit_message("adapter turn", SubmitMessageOptions::default());
    let invocation = plan
        .model_invocation
        .as_ref()
        .expect("provider invocation should be recorded");

    assert_eq!(invocation.http_request.path, "/v1/responses");
    assert!(
        invocation
            .http_request
            .body
            .contains("\"reasoning\":{\"effort\":\"high\"}")
    );
}

#[test]
fn submit_message_lifts_response_result_into_plan() {
    let deps = SharedQueryDeps::builder()
        .with_call_model(StructuredCallModel)
        .build();
    let mut engine = QueryEngine::with_deps(sample_config(), deps);

    let plan = engine.submit_message("structured turn", SubmitMessageOptions::default());

    assert_eq!(
        plan.response_result,
        Some(json!({"ok": true, "source": "query-plan"}))
    );
    assert_eq!(
        plan.model_invocation
            .as_ref()
            .and_then(|invocation| invocation.response_result.clone()),
        Some(json!({"ok": true, "source": "query-plan"}))
    );
    let preview = plan.response_result_preview(32);
    assert!(preview.starts_with("{\"ok\":true,\"source\":\"query-pl"));
    assert!(preview.ends_with("..."));
}

#[test]
fn submit_message_can_use_custom_persistence_backend() {
    let mut engine = QueryEngine::new(sample_config());
    let executor = FailingExecutor;
    let mut persistence = RecordingPersistenceBackend::default();

    let plan = engine.submit_message_with_runtime(
        "persist this turn",
        SubmitMessageOptions::default(),
        &executor,
        &mut persistence,
    );

    assert_eq!(
        plan.persistence_dispatch,
        SharedPersistenceDispatchResult {
            transcript_entries_flushed: plan.transcript.entries.len(),
            history_persisted: true,
            file_history_persisted: true,
        }
    );
    assert!(persistence.finalized);
}
