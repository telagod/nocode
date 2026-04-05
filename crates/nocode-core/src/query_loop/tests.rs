use super::{
    QueryLoopAction, QueryLoopContinueReason, QueryLoopOutcome, QueryLoopParams, QueryLoopRunner,
    QueryLoopTerminal, QuerySource, TaskBudget,
};
use crate::budget::TokenBudgetDecision;
use crate::message::QueryMessage;
use crate::stop_hook::{StopHookInfo, StopHookResult};
use crate::tool_execution::{
    ToolCallInput, ToolCallOutput, ToolCallResult, ToolPermissionDecision, ToolProgressUpdate,
};

fn sample_params() -> QueryLoopParams {
    QueryLoopParams {
        messages: vec![QueryMessage::user("hello")],
        system_prompt: vec![QueryMessage::system("system")],
        user_context_keys: vec![String::from("cwd")],
        system_context_keys: vec![String::from("os")],
        fallback_model: Some(String::from("sonnet")),
        query_source: QuerySource::Sdk,
        max_output_tokens_override: Some(2048),
        max_turns: Some(3),
        task_budget: Some(TaskBudget { total: 10_000 }),
    }
}

#[test]
fn runner_is_seeded_from_query_params() {
    let runner = QueryLoopRunner::new(sample_params());
    assert_eq!(runner.state().messages, vec![QueryMessage::user("hello")]);
    assert_eq!(runner.state().max_output_tokens_override, Some(2048));
    assert!(runner.state().pending_tool_call.is_none());
    assert!(runner.state().pending_turn_messages.is_empty());
    assert!(runner.state().budget_tracker.is_some());
    assert!(runner.state().token_budget_decision.is_none());
    assert!(runner.state().last_stop_hook_result.is_none());
    assert_eq!(runner.state().transcript.len(), 2);
    assert_eq!(runner.state().turn_count, 1);
}

#[test]
fn continue_reason_is_recorded() {
    let mut runner = QueryLoopRunner::new(sample_params());
    let outcome = runner.apply(QueryLoopAction::Continue(
        QueryLoopContinueReason::TokenBudgetContinuation,
    ));
    assert_eq!(outcome, QueryLoopOutcome::Continue(runner.state().clone()));
    assert_eq!(
        runner.state().transition,
        Some(QueryLoopContinueReason::TokenBudgetContinuation)
    );
}

#[test]
fn advance_turn_resets_recovery_state() {
    let mut runner = QueryLoopRunner::new(sample_params());
    runner.apply(QueryLoopAction::Continue(
        QueryLoopContinueReason::MaxOutputTokensRecovery,
    ));
    let outcome = runner.apply(QueryLoopAction::AdvanceTurn {
        next_messages: vec![
            QueryMessage::user("hello"),
            QueryMessage::assistant("world"),
        ],
    });
    assert_eq!(outcome, QueryLoopOutcome::Continue(runner.state().clone()));
    assert_eq!(runner.state().turn_count, 2);
    assert_eq!(
        runner.state().transition,
        Some(QueryLoopContinueReason::NextTurn)
    );
    assert_eq!(runner.state().max_output_tokens_override, None);
}

#[test]
fn request_tool_marks_pending_summary() {
    let mut runner = QueryLoopRunner::new(sample_params());
    let call = ToolCallInput::new("Read", "toolu-1")
        .with_argument("file_path", "src/query.ts")
        .with_context_label("sdk-bootstrap");
    let outcome = runner.apply(QueryLoopAction::RequestTool { call: call.clone() });
    assert_eq!(outcome, QueryLoopOutcome::Continue(runner.state().clone()));
    assert_eq!(runner.state().pending_tool_call, Some(call));
    assert!(runner.state().pending_tool_use_summary);
}

#[test]
fn stop_hook_result_is_recorded() {
    let mut runner = QueryLoopRunner::new(sample_params());
    let outcome = runner.apply(QueryLoopAction::RecordStopHookResult {
        result: StopHookResult {
            blocking_errors: vec![QueryMessage::system("blocked by hook")],
            prevent_continuation: true,
            stop_reason: Some(String::from("hook blocked")),
            hook_count: 1,
            has_output: true,
            hook_errors: vec![String::from("stderr")],
            hook_infos: vec![StopHookInfo::new("check.sh", "before continue")],
        },
    });

    assert_eq!(outcome, QueryLoopOutcome::Continue(runner.state().clone()));
    assert!(runner.state().stop_hook_active);
    assert_eq!(
        runner
            .state()
            .last_stop_hook_result
            .as_ref()
            .map(StopHookResult::summary),
        Some(String::from(
            "stop-hooks:blocking_errors=1 prevent_continuation=true hook_count=1 has_output=true"
        ))
    );
}

#[test]
fn assistant_message_is_appended_to_messages_and_transcript() {
    let mut runner = QueryLoopRunner::new(sample_params());
    let outcome = runner.apply(QueryLoopAction::PushAssistantMessage {
        message: QueryMessage::assistant("model says continue"),
    });

    assert_eq!(outcome, QueryLoopOutcome::Continue(runner.state().clone()));
    assert_eq!(
        runner.state().messages.last(),
        Some(&QueryMessage::assistant("model says continue"))
    );
    assert_eq!(runner.state().transcript.entries.len(), 3);
    assert_eq!(
        runner
            .state()
            .transcript
            .entries
            .last()
            .map(|entry| entry.content.as_str()),
        Some("assistant: model says continue")
    );
}

#[test]
fn resolved_tool_stages_messages_until_batch_flush() {
    let mut runner = QueryLoopRunner::new(sample_params());
    let call = ToolCallInput::new("Read", "toolu-2")
        .with_argument("file_path", "src/query.ts")
        .with_context_label("sdk-bootstrap");
    let _ = runner.apply(QueryLoopAction::RequestTool { call: call.clone() });
    let outcome = runner.apply(QueryLoopAction::ResolveTool {
        result: ToolPermissionDecision::allow(false).settle(
            call,
            ToolCallOutput {
                summary: String::from("loaded migration seed"),
                generated_messages: vec![QueryMessage::assistant(
                    "tool-message: query.ts mirrored",
                )],
                context_label: Some(String::from("sdk-bootstrap")),
                progress_updates: vec![ToolProgressUpdate::new("toolu-2", "done")],
            },
        ),
    });

    assert_eq!(outcome, QueryLoopOutcome::Continue(runner.state().clone()));
    assert_eq!(runner.state().turn_count, 1);
    assert!(runner.state().messages.iter().all(|message| {
        !message
            .content
            .contains("tool-result:Read#toolu-2(file_path=src/query.ts)")
    }));
    assert_eq!(runner.state().pending_turn_messages.len(), 2);
    assert!(runner.state().pending_tool_call.is_none());
    assert!(!runner.state().pending_tool_use_summary);
    assert_eq!(runner.state().tool_results.len(), 1);
    assert_eq!(runner.state().tool_progress_log.len(), 1);
    assert_eq!(
        runner
            .state()
            .transcript
            .entries
            .iter()
            .filter(|entry| entry.role == crate::transcript::TranscriptRole::ToolResult)
            .count(),
        1
    );

    let outcome = runner.apply(QueryLoopAction::FlushToolBatch);
    assert_eq!(outcome, QueryLoopOutcome::Continue(runner.state().clone()));
    assert_eq!(runner.state().turn_count, 2);
    assert!(runner.state().messages.iter().any(|message| {
        message
            .content
            .contains("tool-result:Read#toolu-2(file_path=src/query.ts)")
    }));
    assert!(
        runner
            .state()
            .messages
            .iter()
            .any(|message| message.content.contains("tool-message: query.ts mirrored"))
    );
    assert!(runner.state().pending_turn_messages.is_empty());
}

#[test]
fn denied_tool_result_flushes_on_next_turn_boundary() {
    let mut runner = QueryLoopRunner::new(sample_params());
    let call = ToolCallInput::new("Edit", "toolu-3")
        .with_argument("file_path", "src/query.ts")
        .with_context_label("sdk-bootstrap");
    let _ = runner.apply(QueryLoopAction::RequestTool { call: call.clone() });
    let outcome = runner.apply(QueryLoopAction::ResolveTool {
        result: ToolPermissionDecision::deny("write denied by policy")
            .settle(call, ToolCallOutput::default()),
    });

    assert_eq!(outcome, QueryLoopOutcome::Continue(runner.state().clone()));
    assert_eq!(runner.state().turn_count, 1);
    assert_eq!(runner.state().pending_turn_messages.len(), 1);

    let outcome = runner.apply(QueryLoopAction::FlushToolBatch);
    assert_eq!(outcome, QueryLoopOutcome::Continue(runner.state().clone()));
    assert_eq!(runner.state().turn_count, 2);
    assert_eq!(
        runner.state().tool_results,
        vec![ToolCallResult::Denied {
            call: ToolCallInput::new("Edit", "toolu-3")
                .with_argument("file_path", "src/query.ts")
                .with_context_label("sdk-bootstrap"),
            reason: String::from("write denied by policy"),
        }]
    );
}

#[test]
fn max_turn_limit_returns_terminal_reason() {
    let mut runner = QueryLoopRunner::new(sample_params());
    let _ = runner.apply(QueryLoopAction::AdvanceTurn {
        next_messages: vec![QueryMessage::assistant("1")],
    });
    let _ = runner.apply(QueryLoopAction::AdvanceTurn {
        next_messages: vec![QueryMessage::assistant("2")],
    });
    let outcome = runner.apply(QueryLoopAction::AdvanceTurn {
        next_messages: vec![QueryMessage::assistant("3")],
    });
    assert_eq!(
        outcome,
        QueryLoopOutcome::Terminal(QueryLoopTerminal::MaxTurns { turn_count: 4 })
    );
}

#[test]
fn token_budget_check_records_continue_transition() {
    let mut runner = QueryLoopRunner::new(sample_params());
    let outcome = runner.apply(QueryLoopAction::CheckTokenBudget {
        global_turn_tokens: 1_000,
        now_ms: 5_000,
    });

    assert_eq!(outcome, QueryLoopOutcome::Continue(runner.state().clone()));
    assert_eq!(
        runner.state().transition,
        Some(QueryLoopContinueReason::TokenBudgetContinuation)
    );
    assert!(matches!(
        runner.state().token_budget_decision,
        Some(TokenBudgetDecision::Continue { .. })
    ));
}
