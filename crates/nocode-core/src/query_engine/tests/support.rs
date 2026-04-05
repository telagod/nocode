pub(super) use super::super::{
    QueryEngine, QueryEngineConfig, QueryPlanStep, QuerySubmissionPlan, SubmitMessageOptions,
    TaskBudget, ThinkingMode, ask, ask_with_executor,
};
use crate::assistant_turn::AssistantTurnStatus;
use crate::budget::TokenBudgetDecision;
use crate::history_store::HistoryEntry;
use crate::message::QueryMessage;
use crate::model_response::ModelResponseStopReason;
use crate::persistence_backend::{
    FileHistorySnapshot, PersistedTranscriptEntry, PersistenceBackend, PersistenceReader,
};
use crate::provider::{
    ModelCallOutput, ModelError, ModelProvider, ModelRequest, ModelStreamEvent, ModelStreamSink,
};
use crate::query_deps::{CallModel, Clock, Compactor, IdGen, StopHookRunner, ToolRunner};
use crate::query_loop::{QueryLoopOutcome, QuerySource};
use crate::session_persistence::SessionResumePlan;
use crate::stop_hook::{StopHookInfo, StopHookResult};
use crate::tool_execution::{
    ToolCallOutput, ToolCallResult, ToolExecutionRequest, ToolExecutionTrace, ToolExecutor,
};
pub(super) use crate::tool_registry::{ToolPermissionContext, ToolRuntimeMode};
use crate::transcript::TranscriptRole;
use serde_json::json;
use std::io;

pub(super) struct FailingExecutor;

#[derive(Default)]
pub(super) struct RecordingPersistenceBackend {
    pub transcript_entries_flushed: usize,
    pub history_persisted: bool,
    pub file_history_persisted: bool,
    pub finalized: bool,
}

#[derive(Default)]
pub(super) struct RecordingPersistenceReader {
    pub transcript: Vec<PersistedTranscriptEntry>,
    pub history: Vec<HistoryEntry>,
    pub file_history: Option<FileHistorySnapshot>,
}

#[derive(Debug)]
pub(super) struct FixedClock;

#[derive(Debug)]
pub(super) struct FixedIdGen;

#[derive(Debug)]
pub(super) struct BlockingStopHook;

#[derive(Debug)]
pub(super) struct CollapseCompactor;

#[derive(Debug)]
pub(super) struct RescuingToolRunner;

#[derive(Debug)]
pub(super) struct FixedCallModel;

#[derive(Debug)]
pub(super) struct StructuredCallModel;

#[derive(Debug)]
pub(super) struct PanicCallModel;

#[derive(Debug)]
pub(super) struct FailingCallModel;

impl ToolExecutor for FailingExecutor {
    fn execute(&self, request: ToolExecutionRequest) -> ToolExecutionTrace {
        ToolExecutionTrace {
            progress_updates: Vec::new(),
            result: ToolCallResult::failed(request.call, "executor explosion"),
            permission_denial: None,
        }
    }
}

impl PersistenceBackend for RecordingPersistenceBackend {
    fn persist_transcript(&mut self, entries: &[String]) -> usize {
        self.transcript_entries_flushed = entries.len();
        entries.len()
    }

    fn persist_history(&mut self, plan: &crate::history_store::HistoryStorePlan) -> bool {
        self.history_persisted = plan.persist_history && plan.pending_entries > 0;
        self.history_persisted
    }

    fn persist_file_history(&mut self, plan: &crate::file_history::FileHistoryPlan) -> bool {
        self.file_history_persisted = plan.snapshot_requested;
        self.file_history_persisted
    }

    fn finalize(&mut self, _plan: &crate::session_persistence::SessionPersistencePlan) {
        self.finalized = true;
    }
}

impl PersistenceReader for RecordingPersistenceReader {
    fn read_transcript(
        &self,
        _plan: &SessionResumePlan,
    ) -> io::Result<Vec<PersistedTranscriptEntry>> {
        Ok(self.transcript.clone())
    }

    fn read_history(&self, _plan: &SessionResumePlan) -> io::Result<Vec<HistoryEntry>> {
        Ok(self.history.clone())
    }

    fn read_file_history(
        &self,
        _plan: &SessionResumePlan,
    ) -> io::Result<Option<FileHistorySnapshot>> {
        Ok(self.file_history.clone())
    }
}

impl Clock for FixedClock {
    fn now_ms(&self) -> u64 {
        42_000
    }
}

impl IdGen for FixedIdGen {
    fn generate(&self) -> String {
        String::from("custom")
    }
}

impl StopHookRunner for BlockingStopHook {
    fn run_stop_hooks(&self, messages: &[QueryMessage]) -> StopHookResult {
        StopHookResult {
            blocking_errors: vec![QueryMessage::system("hook blocked")],
            prevent_continuation: true,
            stop_reason: Some(String::from("stop hook veto")),
            hook_count: 1,
            has_output: !messages.is_empty(),
            hook_errors: Vec::new(),
            hook_infos: vec![StopHookInfo::new("hook", "prompt")],
        }
    }
}

impl Compactor for CollapseCompactor {
    fn compact(&self, messages: &[QueryMessage]) -> Vec<QueryMessage> {
        let Some(last) = messages.last().cloned() else {
            return Vec::new();
        };
        vec![QueryMessage::system("compacted"), last]
    }
}

impl ToolRunner for RescuingToolRunner {
    fn run_tool(&self, request: crate::tool_execution::ToolCallInput) -> ToolCallResult {
        ToolCallResult::Completed {
            call: request.clone(),
            user_modified: false,
            output: ToolCallOutput {
                summary: format!("rescued {}", request.tool_name),
                generated_messages: vec![QueryMessage::assistant(format!(
                    "tool-message: rescued {}",
                    request.tool_use_id
                ))],
                context_label: Some(request.context_label.clone()),
                progress_updates: Vec::new(),
            },
        }
    }
}

impl CallModel for FixedCallModel {
    fn call_model(
        &self,
        request: &ModelRequest,
        stream: &mut dyn ModelStreamSink,
    ) -> Result<ModelCallOutput, ModelError> {
        let selected_model = request
            .selection
            .selected_model()
            .ok_or_else(ModelError::no_model_configured)?;
        let content = format!(
            "model-output:{}:{}:{}",
            request.selection.provider.as_str(),
            selected_model,
            request.reply_target().unwrap_or("none")
        );
        let message = QueryMessage::assistant(content);
        stream.push(ModelStreamEvent::Start {
            provider: request.selection.provider,
            model: selected_model.to_string(),
        });
        stream.push(ModelStreamEvent::Delta {
            text: String::from("delta"),
        });
        stream.push(ModelStreamEvent::Complete {
            message: message.clone(),
        });
        Ok(ModelCallOutput::new(
            request.selection.provider,
            selected_model,
            message,
        ))
    }
}

impl CallModel for StructuredCallModel {
    fn call_model(
        &self,
        request: &ModelRequest,
        stream: &mut dyn ModelStreamSink,
    ) -> Result<ModelCallOutput, ModelError> {
        let selected_model = request
            .selection
            .selected_model()
            .ok_or_else(ModelError::no_model_configured)?;
        let message = QueryMessage::assistant("{\"ok\":true,\"source\":\"query-plan\"}");
        stream.push(ModelStreamEvent::Start {
            provider: request.selection.provider,
            model: selected_model.to_string(),
        });
        stream.push(ModelStreamEvent::Complete {
            message: message.clone(),
        });
        Ok(
            ModelCallOutput::new(request.selection.provider, selected_model, message)
                .with_response_result(json!({"ok": true, "source": "query-plan"})),
        )
    }
}

impl CallModel for PanicCallModel {
    fn call_model(
        &self,
        _request: &ModelRequest,
        _stream: &mut dyn ModelStreamSink,
    ) -> Result<ModelCallOutput, ModelError> {
        panic!("call_model should not run");
    }
}

impl CallModel for FailingCallModel {
    fn call_model(
        &self,
        _request: &ModelRequest,
        _stream: &mut dyn ModelStreamSink,
    ) -> Result<ModelCallOutput, ModelError> {
        Err(ModelError::provider_failure("provider timeout", true))
    }
}

pub(super) fn repo_root() -> String {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("repo root should resolve")
        .to_string_lossy()
        .into_owned()
}

pub(super) fn sample_config() -> QueryEngineConfig {
    QueryEngineConfig {
        cwd: repo_root(),
        session_id: String::from("session-1"),
        persist_session: true,
        persist_history: true,
        file_history_enabled: true,
        tools: vec![String::from("Read"), String::from("Edit")],
        tool_runtime_mode: ToolRuntimeMode::Standard,
        tool_permission_context: ToolPermissionContext::default(),
        commands: vec![String::from("/help")],
        mcp_clients: vec![String::from("filesystem")],
        agents: vec![String::from("leader")],
        initial_messages: vec![QueryMessage::system("seed")],
        read_file_cache_entries: 2,
        custom_system_prompt: Some(String::from("custom")),
        append_system_prompt: Some(String::from("append")),
        model_provider: ModelProvider::Mock,
        user_specified_model: Some(String::from("sonnet")),
        fallback_model: Some(String::from("haiku")),
        model_reasoning_effort: None,
        thinking_mode: ThinkingMode::Adaptive,
        max_turns: Some(4),
        max_budget_usd: Some(5.0),
        task_budget: Some(TaskBudget { total: 20_000 }),
        json_schema: None,
        verbose: true,
        replay_user_messages: false,
        include_partial_messages: false,
        stream_model_responses: true,
    }
}

pub(super) fn assert_seeded_plan(plan: &QuerySubmissionPlan) {
    assert_eq!(plan.loop_params.query_source, QuerySource::Sdk);
    assert_eq!(plan.loop_params.messages.len(), 2);
    assert_eq!(plan.loop_params.system_prompt.len(), 2);
    assert_eq!(
        plan.loop_params.system_prompt,
        plan.query_config.system_prompt
    );
    assert_eq!(plan.available_tools.len(), 2);
    assert_eq!(plan.steps.len(), 14);
    assert_eq!(plan.requested_tools.len(), 3);
    assert_eq!(plan.tool_results.len(), 3);
    assert_eq!(plan.transcript.entries.len(), 20);
    assert!(matches!(
        plan.token_budget_decision,
        Some(TokenBudgetDecision::Continue { .. })
    ));
    assert_eq!(plan.assistant_turn.status, AssistantTurnStatus::Completed);
    assert_eq!(plan.assistant_turn.response_messages.len(), 7);
    assert_eq!(plan.assistant_turn.tool_uses.len(), 3);
    assert_eq!(plan.model_response.response_id, "resp-1");
    assert_eq!(plan.model_response.status, AssistantTurnStatus::Completed);
    assert_eq!(
        plan.model_response.stop_reason,
        ModelResponseStopReason::Completed
    );
    assert_eq!(plan.model_response.tool_phase.requested_tools, 3);
    assert_eq!(plan.model_response.tool_phase.resolved_tools, 3);
    assert_eq!(
        plan.model_invocation
            .as_ref()
            .map(|invocation| invocation.provider),
        Some(ModelProvider::Mock)
    );
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
    assert_eq!(
        plan.model_invocation
            .as_ref()
            .map(|invocation| invocation.http_request.path.as_str()),
        Some("/mock")
    );
    assert!(matches!(
        plan.model_response.final_assistant_message.as_ref(),
        Some(message)
            if message.role == crate::message::QueryMessageRole::Assistant
                && message.content.starts_with("nocode response: ")
    ));
    assert_eq!(
        plan.transcript.entries[0].role,
        TranscriptRole::Conversation
    );
    assert!(matches!(
        plan.loop_outcome,
        QueryLoopOutcome::Terminal(crate::query_loop::QueryLoopTerminal::Completed)
    ));
    assert_eq!(
        plan.persistence_dispatch.transcript_entries_flushed,
        plan.transcript.entries.len()
    );
    assert!(plan.persistence_dispatch.history_persisted);
    assert!(plan.persistence_dispatch.file_history_persisted);
}

pub(super) fn assert_step(plan_step: &QueryPlanStep, action: &str) {
    assert_eq!(plan_step.action, action);
    assert!(matches!(plan_step.outcome, QueryLoopOutcome::Continue(_)));
}

pub(super) use crate::history_store::HistoryEntry as SharedHistoryEntry;
pub(super) use crate::message::QueryMessage as SharedQueryMessage;
pub(super) use crate::model_response::ModelResponseStopReason as SharedModelResponseStopReason;
pub(super) use crate::persistence_backend::{
    FileHistorySnapshot as SharedFileHistorySnapshot,
    PersistedTranscriptEntry as SharedPersistedTranscriptEntry,
    PersistenceDispatchResult as SharedPersistenceDispatchResult,
};
pub(super) use crate::provider::{
    ModelProvider as SharedModelProvider, ModelStreamMode as SharedModelStreamMode,
};
pub(super) use crate::query_deps::QueryDeps as SharedQueryDeps;
pub(super) use crate::query_loop::QueryLoopContinueReason;
pub(super) use crate::tool_execution::{
    ToolCallInput as SharedToolCallInput, ToolCallResult as SharedToolCallResult,
};
pub(super) use crate::transcript::TranscriptRole as SharedTranscriptRole;
