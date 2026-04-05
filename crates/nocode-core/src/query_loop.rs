use crate::budget::{BudgetTracker, TokenBudgetDecision, check_token_budget};
use crate::message::QueryMessage;
use crate::provider::ModelError;
use crate::stop_hook::StopHookResult;
use crate::tool_execution::{ToolCallInput, ToolCallResult, ToolProgressUpdate};
use crate::transcript::{QueryTranscript, TranscriptRole};
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
mod tests;
