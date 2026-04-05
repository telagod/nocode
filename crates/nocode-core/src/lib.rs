pub mod assistant_turn;
pub mod bridge_runtime;
pub mod budget;
pub mod budget_state;
pub mod file_history;
pub mod file_safety;
pub mod history_store;
pub mod mcp_client;
pub mod message;
pub mod model_response;
pub mod persistence_backend;
pub mod policy_engine;
pub mod provider;
pub mod provider_transport;
pub mod query_config;
pub mod query_deps;
pub mod query_engine;
pub mod query_loop;
pub mod recovery;
pub mod roadmap;
pub mod session_compaction;
pub mod session_persistence;
pub mod stop_hook;
pub mod task_runtime;
pub mod tool_execution;
pub mod tool_registry;
pub mod tool_validation;
pub mod transcript;
pub mod usage_tracker;
pub mod worker_boot;
pub use assistant_turn::{AssistantToolUse, AssistantTurn, AssistantTurnStatus};
pub use bridge_runtime::{
    BridgeEventPayloadWire, BridgeEventWire, BridgeMode, BridgeModule, BridgeRequest,
    BridgeSubmittedWire, BridgeTransportError, BridgeTurn, BridgeTurnOutcome,
    BridgeTurnOutcomeWire, BridgeTurnWire, BridgeWireRequest, BridgeWireResponse,
    HttpRemoteBridgeAuth, HttpRemoteBridgeTransport, HttpRemoteBridgeTransportConfig,
    PermissionCallback, PermissionRequestWire, PermissionResponseWire, RemoteBridgeTransport,
    SessionPointer, SessionRunner, SubmitMessageOptionsWire,
};
pub use budget::{BudgetCompletionEvent, BudgetTracker, TokenBudgetDecision, check_token_budget};
pub use budget_state::{BudgetState, TaskBudget};
pub use file_history::{FileHistoryConfig, FileHistoryPlan, FileHistoryState};
pub use file_safety::{
    check_file_size, check_symlink_escape, is_binary_file, validate_read_target,
    validate_write_target, BINARY_CHECK_SIZE, MAX_FILE_SIZE,
};
pub use history_store::{HistoryEntry, HistoryStore, HistoryStoreConfig, HistoryStorePlan};
pub use message::{QueryMessage, QueryMessageRole};
pub use model_response::{ModelResponse, ModelResponseStopReason, ModelResponseToolPhase};
pub use persistence_backend::{
    FileHistorySnapshot, LocalPersistenceBackend, NoopPersistenceBackend, NoopPersistenceReader,
    PersistedTranscriptEntry, PersistenceBackend, PersistenceDispatchResult, PersistenceReader,
};
pub use policy_engine::{
    DiffScope, GreenLevel, LaneBlocker, LaneContext, PolicyAction, PolicyCondition, PolicyEngine,
    PolicyRule, ReconcileReason, ReviewStatus,
};
pub use provider::{
    ChannelModelStreamSink, HttpStatusClass, ModelCallOutput, ModelError, ModelErrorKind,
    ModelErrorWire, ModelInvocation, ModelProvider, ModelProviderCapabilities, ModelRequest,
    ModelSelection, ModelStreamEvent, ModelStreamEventWire, ModelStreamMode, ModelStreamSink,
    ProviderHeader, ProviderHttpRequest, RecordingModelStream, ToolSchema,
};
pub use query_config::{QueryConfig, QueryRuntimeGates};
pub use query_deps::{
    CallModel, Clock, Compactor, IdGen, QueryDeps, QueryDepsBuilder, StopHookRunner, ToolRunner,
    TruncatingCompactor,
};
pub use query_engine::{
    QueryEngine, QueryEngineConfig, QueryEngineModule, QueryEngineState, QueryPlanStep,
    QuerySubmissionPlan, ResumeSnapshot, SubmitMessageOptions, ThinkingMode, ask,
    ask_with_executor,
};
pub use query_loop::{
    QueryLoopAction, QueryLoopContinueReason, QueryLoopModule, QueryLoopOutcome, QueryLoopParams,
    QueryLoopRunner, QueryLoopState, QueryLoopTerminal, QuerySource,
};
pub use roadmap::{MigrationSurface, RewriteRoadmap, RewriteStage, default_roadmap, render_status};
pub use recovery::{
    EscalationPolicy, FailureScenario, RecoveryContext, RecoveryEvent, RecoveryRecipe,
    RecoveryResult, RecoveryStep, recipe_for,
};
pub use session_compaction::{
    CompactionConfig, CompactionResult, RichCompactor, compact_session, estimate_message_tokens,
    should_compact, summarize_messages,
};
pub use session_persistence::{
    ReadFileCacheState, SessionIdentity, SessionPersistenceConfig, SessionPersistencePlan,
    SessionPersistenceState, SessionResumePlan,
};
pub use stop_hook::{StopHookInfo, StopHookResult};
pub use task_runtime::{
    AgentProgress, AgentStep, AgentTaskResult, BashTaskKind, CommandResult, DefaultDreamHost,
    DreamPhase, DreamStep, DreamTaskResult, DreamTurn, InProcessAgentHost, LiveTaskRuntimeDriver,
    LiveTaskShellHost, LocalAgentTaskData, LocalShellTaskData, ProcessAgentBackoffPolicy,
    ProcessAgentBackoffProfile, ProcessAgentBackoffProfileKind, ProcessAgentBackoffStrategy,
    ProcessAgentFailureKind, ProcessAgentOutputWire, ProcessAgentRequestWire,
    ProcessAgentResponseWire, ProcessAgentRestartPolicy, ProcessAgentStatusWire,
    ProcessAgentSupervisorPolicy, ProcessTaskAgentHost, StopTaskError, StopTaskResult,
    TaskAgentHost, TaskCoordinator, TaskDreamHost, TaskDriveError, TaskDriveReport, TaskId,
    TaskPayload, TaskRecord, TaskResult, TaskRuntimeDriver, TaskShellHost, TaskStateBase,
    TaskStatus, TaskType, stop_task,
};
pub use tool_execution::{
    DefaultToolExecutor, LiveToolHost, ToolCallArgument, ToolCallInput, ToolCallOutput,
    ToolCallResult, ToolCommandOutput, ToolExecutionContext, ToolExecutionModule,
    ToolExecutionRequest, ToolExecutionTrace, ToolExecutor, ToolHost, ToolPermissionDecision,
    ToolProgressUpdate,
};
pub use tool_execution::mcp_bridge::{McpToolBridge, McpToolInfo, execute_mcp_tool_bridged};
pub use tool_registry::{
    PermissionCondition, PermissionMode, PermissionRule, ToolDefinition, ToolKind,
    ToolPermissionContext, ToolRegistry, ToolRegistryModule, ToolRegistrySelection,
    ToolRuntimeMode, ToolSelectionIssue,
};
pub use tool_validation::{get_tool_schema, validate_tool_input};
pub use transcript::{QueryTranscript, TranscriptEntry, TranscriptRole};
pub use usage_tracker::{UsageSnapshot, UsageTotals, UsageTracker};
pub use worker_boot::{
    Worker, WorkerEvent, WorkerEventKind, WorkerFailure, WorkerFailureKind, WorkerRegistry,
    WorkerStatus, global_worker_registry,
};
