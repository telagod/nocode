use crate::budget::{BudgetTracker, TokenBudgetDecision, check_token_budget};
use crate::message::QueryMessage;
use crate::provider::ModelError;
use crate::stop_hook::StopHookResult;
use crate::tool_execution::{ToolCallInput, ToolCallResult, ToolProgressUpdate};
use crate::transcript::{QueryTranscript, TranscriptEntry, TranscriptRole};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueryLoopModule;

impl QueryLoopModule {
    pub const LABEL: &'static str = "query-loop";
    pub const TS_SOURCE: &'static str = "src/query.ts";
    pub const RESPONSIBILITY: &'static str =
        "Drives the iterative agent loop, loop state transitions, and recovery/continuation paths.";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuerySource {
    Sdk,
    Repl,
    Print,
}

impl QuerySource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sdk => "sdk",
            Self::Repl => "repl",
            Self::Print => "print",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskBudget {
    pub total: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryLoopParams {
    pub messages: Vec<QueryMessage>,
    pub system_prompt: Vec<QueryMessage>,
    pub user_context_keys: Vec<String>,
    pub system_context_keys: Vec<String>,
    pub fallback_model: Option<String>,
    pub query_source: QuerySource,
    pub max_output_tokens_override: Option<u32>,
    pub max_turns: Option<u32>,
    pub task_budget: Option<TaskBudget>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryLoopContinueReason {
    NextTurn,
    ReactiveCompactRetry,
    MaxOutputTokensEscalate,
    MaxOutputTokensRecovery,
    StopHookBlocking,
    TokenBudgetContinuation,
    CollapseDrainRetry,
}

impl QueryLoopContinueReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NextTurn => "next_turn",
            Self::ReactiveCompactRetry => "reactive_compact_retry",
            Self::MaxOutputTokensEscalate => "max_output_tokens_escalate",
            Self::MaxOutputTokensRecovery => "max_output_tokens_recovery",
            Self::StopHookBlocking => "stop_hook_blocking",
            Self::TokenBudgetContinuation => "token_budget_continuation",
            Self::CollapseDrainRetry => "collapse_drain_retry",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryLoopTerminal {
    BlockingLimit,
    ImageError,
    ModelError { error: ModelError },
    AbortedStreaming,
    PromptTooLong,
    Completed,
    StopHookPrevented,
    AbortedTools,
    HookStopped,
    MaxTurns { turn_count: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryLoopState {
    pub messages: Vec<QueryMessage>,
    pub pending_turn_messages: Vec<QueryMessage>,
    pub tool_use_context_label: String,
    pub auto_compact_active: bool,
    pub max_output_tokens_recovery_count: u32,
    pub has_attempted_reactive_compact: bool,
    pub max_output_tokens_override: Option<u32>,
    pub pending_tool_use_summary: bool,
    pub stop_hook_active: bool,
    pub pending_tool_call: Option<ToolCallInput>,
    pub tool_progress_log: Vec<ToolProgressUpdate>,
    pub tool_results: Vec<ToolCallResult>,
    pub budget_tracker: Option<BudgetTracker>,
    pub token_budget_decision: Option<TokenBudgetDecision>,
    pub last_stop_hook_result: Option<StopHookResult>,
    pub transcript: QueryTranscript,
    pub transcript_flushed_index: usize,
    pub turn_count: u32,
    pub transition: Option<QueryLoopContinueReason>,
}

impl QueryLoopState {
    pub fn new(params: &QueryLoopParams) -> Self {
        let mut transcript_messages = params.system_prompt.clone();
        transcript_messages.extend(params.messages.clone());
        let started_at_ms = current_time_ms();
        Self {
            messages: params.messages.clone(),
            pending_turn_messages: Vec::new(),
            tool_use_context_label: String::from("unbound"),
            auto_compact_active: false,
            max_output_tokens_recovery_count: 0,
            has_attempted_reactive_compact: false,
            max_output_tokens_override: params.max_output_tokens_override,
            pending_tool_use_summary: false,
            stop_hook_active: false,
            pending_tool_call: None,
            tool_progress_log: Vec::new(),
            tool_results: Vec::new(),
            budget_tracker: params
                .task_budget
                .map(|_| BudgetTracker::new(started_at_ms)),
            token_budget_decision: None,
            last_stop_hook_result: None,
            transcript: QueryTranscript::from_messages(&transcript_messages, 1),
            transcript_flushed_index: 0,
            turn_count: 1,
            transition: None,
        }
    }
}

fn current_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map_or(0, |duration| duration.as_millis() as u64)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryLoopAction {
    BindToolUseContext {
        label: String,
    },
    RequestTool {
        call: ToolCallInput,
    },
    PushToolProgress {
        update: ToolProgressUpdate,
    },
    ResolveTool {
        result: ToolCallResult,
    },
    FlushToolBatch,
    CheckTokenBudget {
        global_turn_tokens: u64,
        now_ms: u64,
    },
    RecordStopHookResult {
        result: StopHookResult,
    },
    PushAssistantMessage {
        message: QueryMessage,
    },
    SetPendingToolUseSummary(bool),
    SetStopHookActive(bool),
    Continue(QueryLoopContinueReason),
    AdvanceTurn {
        next_messages: Vec<QueryMessage>,
    },
    Complete,
    Fail(QueryLoopTerminal),
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryLoopOutcome {
    Continue(QueryLoopState),
    Terminal(QueryLoopTerminal),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryLoopRunner {
    params: QueryLoopParams,
    state: QueryLoopState,
}

impl QueryLoopRunner {
    pub fn new(params: QueryLoopParams) -> Self {
        let state = QueryLoopState::new(&params);
        Self { params, state }
    }

    pub fn params(&self) -> &QueryLoopParams {
        &self.params
    }

    pub fn state(&self) -> &QueryLoopState {
        &self.state
    }

    pub fn into_state(self) -> QueryLoopState {
        self.state
    }

    /// Returns transcript entries that have not yet been flushed to disk,
    /// and advances the flushed index so the same entries are not returned twice.
    pub fn drain_unflushed_transcript_entries(&mut self) -> &[TranscriptEntry] {
        let start = self.state.transcript_flushed_index;
        let end = self.state.transcript.entries.len();
        self.state.transcript_flushed_index = end;
        &self.state.transcript.entries[start..end]
    }

    fn advance_to_next_turn(&mut self, next_messages: Vec<QueryMessage>) -> QueryLoopOutcome {
        self.state.messages = next_messages;
        self.state.pending_turn_messages.clear();
        self.state.turn_count += 1;
        self.state.transition = Some(QueryLoopContinueReason::NextTurn);
        self.state.max_output_tokens_recovery_count = 0;
        self.state.has_attempted_reactive_compact = false;
        self.state.max_output_tokens_override = None;

        if let Some(limit) = self.params.max_turns
            && self.state.turn_count > limit
        {
            return QueryLoopOutcome::Terminal(QueryLoopTerminal::MaxTurns {
                turn_count: self.state.turn_count,
            });
        }

        QueryLoopOutcome::Continue(self.state.clone())
    }

    fn record_transcript(&mut self, role: TranscriptRole, content: impl Into<String>) {
        self.state
            .transcript
            .push(self.state.turn_count, role, content);
    }

    fn resolve_tool_in_place(&mut self, result: ToolCallResult) -> QueryLoopOutcome {
        let result_message = result.message();
        self.state.pending_tool_call = None;
        self.state.pending_tool_use_summary = false;
        self.state
            .pending_turn_messages
            .push(QueryMessage::tool(result_message.clone()));
        self.record_transcript(TranscriptRole::ToolResult, result_message);

        if let ToolCallResult::Completed { output, .. } = &result {
            self.state
                .tool_progress_log
                .extend(output.progress_updates.clone());
            for update in &output.progress_updates {
                self.record_transcript(TranscriptRole::ToolProgress, update.message.clone());
            }
            for message in &output.generated_messages {
                self.state.pending_turn_messages.push(message.clone());
                self.record_transcript(TranscriptRole::ToolMessage, message.summary());
            }
            if let Some(context_label) = &output.context_label {
                self.state.tool_use_context_label = context_label.clone();
            }
        }

        self.state.tool_results.push(result);
        QueryLoopOutcome::Continue(self.state.clone())
    }

    pub fn apply(&mut self, action: QueryLoopAction) -> QueryLoopOutcome {
        match action {
            QueryLoopAction::BindToolUseContext { label } => {
                self.state.tool_use_context_label = label;
                QueryLoopOutcome::Continue(self.state.clone())
            }
            QueryLoopAction::RequestTool { call } => {
                self.state.tool_use_context_label = call.context_label.clone();
                self.state.pending_tool_use_summary = true;
                self.record_transcript(TranscriptRole::ToolRequest, call.summary());
                self.state.pending_tool_call = Some(call);
                QueryLoopOutcome::Continue(self.state.clone())
            }
            QueryLoopAction::PushToolProgress { update } => {
                self.record_transcript(TranscriptRole::ToolProgress, update.message.clone());
                self.state.tool_progress_log.push(update);
                QueryLoopOutcome::Continue(self.state.clone())
            }
            QueryLoopAction::ResolveTool { result } => self.resolve_tool_in_place(result),
            QueryLoopAction::FlushToolBatch => {
                let mut next_messages = self.state.messages.clone();
                next_messages.extend(self.state.pending_turn_messages.clone());
                self.advance_to_next_turn(next_messages)
            }
            QueryLoopAction::CheckTokenBudget {
                global_turn_tokens,
                now_ms,
            } => {
                let Some(total_budget) = self
                    .params
                    .task_budget
                    .map(|budget| u64::from(budget.total))
                else {
                    self.state.token_budget_decision = None;
                    return QueryLoopOutcome::Continue(self.state.clone());
                };
                let Some(tracker) = self.state.budget_tracker.as_mut() else {
                    self.state.token_budget_decision = None;
                    return QueryLoopOutcome::Continue(self.state.clone());
                };
                let decision = check_token_budget(
                    tracker,
                    None,
                    Some(total_budget),
                    global_turn_tokens,
                    now_ms,
                );
                if matches!(decision, TokenBudgetDecision::Continue { .. }) {
                    self.state.transition = Some(QueryLoopContinueReason::TokenBudgetContinuation);
                }
                self.state.token_budget_decision = Some(decision);
                QueryLoopOutcome::Continue(self.state.clone())
            }
            QueryLoopAction::RecordStopHookResult { result } => {
                self.record_transcript(TranscriptRole::Conversation, result.summary());
                self.state.stop_hook_active = result.prevent_continuation;
                self.state.last_stop_hook_result = Some(result);
                QueryLoopOutcome::Continue(self.state.clone())
            }
            QueryLoopAction::PushAssistantMessage { message } => {
                self.record_transcript(TranscriptRole::Conversation, message.summary());
                self.state.messages.push(message);
                QueryLoopOutcome::Continue(self.state.clone())
            }
            QueryLoopAction::SetPendingToolUseSummary(value) => {
                self.state.pending_tool_use_summary = value;
                QueryLoopOutcome::Continue(self.state.clone())
            }
            QueryLoopAction::SetStopHookActive(value) => {
                self.state.stop_hook_active = value;
                QueryLoopOutcome::Continue(self.state.clone())
            }
            QueryLoopAction::Continue(reason) => {
                self.state.transition = Some(reason);
                QueryLoopOutcome::Continue(self.state.clone())
            }
            QueryLoopAction::AdvanceTurn { next_messages } => {
                self.advance_to_next_turn(next_messages)
            }
            QueryLoopAction::Complete => QueryLoopOutcome::Terminal(QueryLoopTerminal::Completed),
            QueryLoopAction::Fail(reason) => QueryLoopOutcome::Terminal(reason),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::QueryMessage;
    use crate::stop_hook::StopHookResult;

    fn sample_params() -> QueryLoopParams {
        QueryLoopParams {
            messages: vec![QueryMessage::user("hello")],
            system_prompt: vec![QueryMessage::system("you are helpful")],
            user_context_keys: vec![],
            system_context_keys: vec![],
            fallback_model: None,
            query_source: QuerySource::Repl,
            max_output_tokens_override: None,
            max_turns: None,
            task_budget: None,
        }
    }

    fn sample_tool_call() -> ToolCallInput {
        ToolCallInput::new("Read", "toolu-1")
            .with_argument("file_path", "src/main.rs")
            .with_context_label("test")
    }

    fn completed_result(call: ToolCallInput, summary: &str) -> ToolCallResult {
        ToolCallResult::Completed {
            call,
            user_modified: false,
            output: crate::tool_execution::ToolCallOutput {
                summary: summary.to_string(),
                generated_messages: Vec::new(),
                context_label: None,
                progress_updates: Vec::new(),
            },
        }
    }

    fn blocking_stop_hook() -> StopHookResult {
        StopHookResult {
            prevent_continuation: true,
            stop_reason: Some(String::from("blocked by hook")),
            ..StopHookResult::default()
        }
    }

    fn permissive_stop_hook() -> StopHookResult {
        StopHookResult {
            prevent_continuation: false,
            ..StopHookResult::default()
        }
    }

    // -----------------------------------------------------------------------
    // Construction & initial state
    // -----------------------------------------------------------------------

    #[test]
    fn runner_initializes_with_correct_state() {
        let runner = QueryLoopRunner::new(sample_params());
        let state = runner.state();
        assert_eq!(state.turn_count, 1);
        assert!(state.pending_turn_messages.is_empty());
        assert!(state.tool_results.is_empty());
        assert!(!state.auto_compact_active);
        assert!(!state.stop_hook_active);
        assert!(state.pending_tool_call.is_none());
        assert!(state.budget_tracker.is_none());
        assert!(state.transition.is_none());
        assert_eq!(state.tool_use_context_label, "unbound");
    }

    #[test]
    fn state_new_merges_system_and_user_messages_into_transcript() {
        let params = sample_params();
        let state = QueryLoopState::new(&params);
        // Transcript should contain both system prompt and user messages
        assert!(state.transcript.len() >= 2);
    }

    #[test]
    fn params_accessible_after_construction() {
        let runner = QueryLoopRunner::new(sample_params());
        assert_eq!(runner.params().query_source, QuerySource::Repl);
        assert!(runner.params().max_turns.is_none());
    }

    // -----------------------------------------------------------------------
    // QueryLoopAction::BindToolUseContext
    // -----------------------------------------------------------------------

    #[test]
    fn bind_tool_use_context_updates_label() {
        let mut runner = QueryLoopRunner::new(sample_params());
        let outcome = runner.apply(QueryLoopAction::BindToolUseContext {
            label: String::from("agent-task"),
        });
        assert!(matches!(outcome, QueryLoopOutcome::Continue(_)));
        assert_eq!(runner.state().tool_use_context_label, "agent-task");
    }

    // -----------------------------------------------------------------------
    // QueryLoopAction::RequestTool
    // -----------------------------------------------------------------------

    #[test]
    fn request_tool_sets_pending_call() {
        let mut runner = QueryLoopRunner::new(sample_params());
        let call = sample_tool_call();
        let outcome = runner.apply(QueryLoopAction::RequestTool { call: call.clone() });
        assert!(matches!(outcome, QueryLoopOutcome::Continue(_)));
        assert!(runner.state().pending_tool_call.is_some());
        assert!(runner.state().pending_tool_use_summary);
        assert_eq!(
            runner.state().pending_tool_call.as_ref().unwrap().tool_name,
            "Read"
        );
    }

    // -----------------------------------------------------------------------
    // QueryLoopAction::ResolveTool
    // -----------------------------------------------------------------------

    #[test]
    fn resolve_tool_clears_pending_and_records_result() {
        let mut runner = QueryLoopRunner::new(sample_params());
        let call = sample_tool_call();
        runner.apply(QueryLoopAction::RequestTool { call: call.clone() });

        let result = completed_result(call, "file contents here");
        let outcome = runner.apply(QueryLoopAction::ResolveTool { result });
        assert!(matches!(outcome, QueryLoopOutcome::Continue(_)));
        assert!(runner.state().pending_tool_call.is_none());
        assert_eq!(runner.state().tool_results.len(), 1);
        assert!(!runner.state().pending_turn_messages.is_empty());
    }

    // -----------------------------------------------------------------------
    // QueryLoopAction::FlushToolBatch → AdvanceTurn
    // -----------------------------------------------------------------------

    #[test]
    fn flush_tool_batch_advances_turn() {
        let mut runner = QueryLoopRunner::new(sample_params());
        let call = sample_tool_call();
        runner.apply(QueryLoopAction::RequestTool { call: call.clone() });
        runner.apply(QueryLoopAction::ResolveTool {
            result: completed_result(call, "ok"),
        });

        let outcome = runner.apply(QueryLoopAction::FlushToolBatch);
        assert!(matches!(outcome, QueryLoopOutcome::Continue(_)));
        assert_eq!(runner.state().turn_count, 2);
        assert!(runner.state().pending_turn_messages.is_empty());
        assert_eq!(
            runner.state().transition,
            Some(QueryLoopContinueReason::NextTurn)
        );
    }

    // -----------------------------------------------------------------------
    // Max turns enforcement
    // -----------------------------------------------------------------------

    #[test]
    fn max_turns_terminates_when_exceeded() {
        let mut params = sample_params();
        params.max_turns = Some(2);
        let mut runner = QueryLoopRunner::new(params);

        // Turn 1 → 2
        runner.apply(QueryLoopAction::AdvanceTurn {
            next_messages: vec![QueryMessage::user("turn 2")],
        });
        assert_eq!(runner.state().turn_count, 2);

        // Turn 2 → 3 should hit max_turns
        let outcome = runner.apply(QueryLoopAction::AdvanceTurn {
            next_messages: vec![QueryMessage::user("turn 3")],
        });
        assert!(matches!(
            outcome,
            QueryLoopOutcome::Terminal(QueryLoopTerminal::MaxTurns { turn_count: 3 })
        ));
    }

    #[test]
    fn no_max_turns_allows_unlimited_advancement() {
        let mut runner = QueryLoopRunner::new(sample_params());
        for i in 0..10 {
            let outcome = runner.apply(QueryLoopAction::AdvanceTurn {
                next_messages: vec![QueryMessage::user(format!("turn {i}"))],
            });
            assert!(matches!(outcome, QueryLoopOutcome::Continue(_)));
        }
        assert_eq!(runner.state().turn_count, 11);
    }

    // -----------------------------------------------------------------------
    // QueryLoopAction::Complete / Fail
    // -----------------------------------------------------------------------

    #[test]
    fn complete_returns_terminal_completed() {
        let mut runner = QueryLoopRunner::new(sample_params());
        let outcome = runner.apply(QueryLoopAction::Complete);
        assert!(matches!(
            outcome,
            QueryLoopOutcome::Terminal(QueryLoopTerminal::Completed)
        ));
    }

    #[test]
    fn fail_returns_terminal_with_reason() {
        let mut runner = QueryLoopRunner::new(sample_params());
        let outcome = runner.apply(QueryLoopAction::Fail(QueryLoopTerminal::PromptTooLong));
        assert!(matches!(
            outcome,
            QueryLoopOutcome::Terminal(QueryLoopTerminal::PromptTooLong)
        ));
    }

    // -----------------------------------------------------------------------
    // QueryLoopAction::PushAssistantMessage
    // -----------------------------------------------------------------------

    #[test]
    fn push_assistant_message_appends_to_messages() {
        let mut runner = QueryLoopRunner::new(sample_params());
        let initial_len = runner.state().messages.len();
        runner.apply(QueryLoopAction::PushAssistantMessage {
            message: QueryMessage::assistant("I will help"),
        });
        assert_eq!(runner.state().messages.len(), initial_len + 1);
    }

    // -----------------------------------------------------------------------
    // QueryLoopAction::SetPendingToolUseSummary / SetStopHookActive
    // -----------------------------------------------------------------------

    #[test]
    fn set_pending_tool_use_summary_toggles_flag() {
        let mut runner = QueryLoopRunner::new(sample_params());
        assert!(!runner.state().pending_tool_use_summary);
        runner.apply(QueryLoopAction::SetPendingToolUseSummary(true));
        assert!(runner.state().pending_tool_use_summary);
        runner.apply(QueryLoopAction::SetPendingToolUseSummary(false));
        assert!(!runner.state().pending_tool_use_summary);
    }

    #[test]
    fn set_stop_hook_active_toggles_flag() {
        let mut runner = QueryLoopRunner::new(sample_params());
        assert!(!runner.state().stop_hook_active);
        runner.apply(QueryLoopAction::SetStopHookActive(true));
        assert!(runner.state().stop_hook_active);
    }

    // -----------------------------------------------------------------------
    // QueryLoopAction::Continue
    // -----------------------------------------------------------------------

    #[test]
    fn continue_action_sets_transition_reason() {
        let mut runner = QueryLoopRunner::new(sample_params());
        runner.apply(QueryLoopAction::Continue(
            QueryLoopContinueReason::ReactiveCompactRetry,
        ));
        assert_eq!(
            runner.state().transition,
            Some(QueryLoopContinueReason::ReactiveCompactRetry)
        );
    }

    // -----------------------------------------------------------------------
    // QueryLoopAction::PushToolProgress
    // -----------------------------------------------------------------------

    #[test]
    fn push_tool_progress_records_update() {
        let mut runner = QueryLoopRunner::new(sample_params());
        let update = ToolProgressUpdate::new(String::from("toolu-1"), String::from("reading file"));
        runner.apply(QueryLoopAction::PushToolProgress { update });
        assert_eq!(runner.state().tool_progress_log.len(), 1);
        assert_eq!(runner.state().tool_progress_log[0].message, "reading file");
    }

    // -----------------------------------------------------------------------
    // QueryLoopAction::RecordStopHookResult
    // -----------------------------------------------------------------------

    #[test]
    fn record_stop_hook_result_stores_and_sets_active() {
        let mut runner = QueryLoopRunner::new(sample_params());
        let hook_result = blocking_stop_hook();
        runner.apply(QueryLoopAction::RecordStopHookResult {
            result: hook_result,
        });
        assert!(runner.state().stop_hook_active);
        assert!(runner.state().last_stop_hook_result.is_some());
        assert!(
            runner
                .state()
                .last_stop_hook_result
                .as_ref()
                .unwrap()
                .prevent_continuation
        );
    }

    #[test]
    fn record_non_blocking_stop_hook_does_not_activate() {
        let mut runner = QueryLoopRunner::new(sample_params());
        let hook_result = permissive_stop_hook();
        runner.apply(QueryLoopAction::RecordStopHookResult {
            result: hook_result,
        });
        assert!(!runner.state().stop_hook_active);
    }

    // -----------------------------------------------------------------------
    // QueryLoopAction::CheckTokenBudget
    // -----------------------------------------------------------------------

    #[test]
    fn check_token_budget_without_budget_is_noop() {
        let mut runner = QueryLoopRunner::new(sample_params());
        runner.apply(QueryLoopAction::CheckTokenBudget {
            global_turn_tokens: 1000,
            now_ms: 100_000,
        });
        assert!(runner.state().token_budget_decision.is_none());
    }

    #[test]
    fn check_token_budget_with_budget_produces_decision() {
        let mut params = sample_params();
        params.task_budget = Some(TaskBudget { total: 50_000 });
        let mut runner = QueryLoopRunner::new(params);
        runner.apply(QueryLoopAction::CheckTokenBudget {
            global_turn_tokens: 1000,
            now_ms: 100_000,
        });
        assert!(runner.state().token_budget_decision.is_some());
    }

    // -----------------------------------------------------------------------
    // into_state
    // -----------------------------------------------------------------------

    #[test]
    fn into_state_consumes_runner() {
        let runner = QueryLoopRunner::new(sample_params());
        let state = runner.into_state();
        assert_eq!(state.turn_count, 1);
    }

    // -----------------------------------------------------------------------
    // QuerySource
    // -----------------------------------------------------------------------

    #[test]
    fn query_source_as_str() {
        assert_eq!(QuerySource::Sdk.as_str(), "sdk");
        assert_eq!(QuerySource::Repl.as_str(), "repl");
        assert_eq!(QuerySource::Print.as_str(), "print");
    }

    // -----------------------------------------------------------------------
    // QueryLoopContinueReason
    // -----------------------------------------------------------------------

    #[test]
    fn continue_reason_as_str_covers_all_variants() {
        let reasons = [
            (QueryLoopContinueReason::NextTurn, "next_turn"),
            (
                QueryLoopContinueReason::ReactiveCompactRetry,
                "reactive_compact_retry",
            ),
            (
                QueryLoopContinueReason::MaxOutputTokensEscalate,
                "max_output_tokens_escalate",
            ),
            (
                QueryLoopContinueReason::MaxOutputTokensRecovery,
                "max_output_tokens_recovery",
            ),
            (
                QueryLoopContinueReason::StopHookBlocking,
                "stop_hook_blocking",
            ),
            (
                QueryLoopContinueReason::TokenBudgetContinuation,
                "token_budget_continuation",
            ),
            (
                QueryLoopContinueReason::CollapseDrainRetry,
                "collapse_drain_retry",
            ),
        ];
        for (reason, expected) in reasons {
            assert_eq!(reason.as_str(), expected);
        }
    }

    // -----------------------------------------------------------------------
    // Turn advancement resets recovery state
    // -----------------------------------------------------------------------

    #[test]
    fn advance_turn_resets_recovery_counters() {
        let mut runner = QueryLoopRunner::new(sample_params());
        // Simulate some recovery state
        runner.apply(QueryLoopAction::Continue(
            QueryLoopContinueReason::MaxOutputTokensRecovery,
        ));

        runner.apply(QueryLoopAction::AdvanceTurn {
            next_messages: vec![QueryMessage::user("next")],
        });

        let state = runner.state();
        assert_eq!(state.max_output_tokens_recovery_count, 0);
        assert!(!state.has_attempted_reactive_compact);
        assert!(state.max_output_tokens_override.is_none());
    }
}
