mod persistence;
mod runtime;
mod state;

use crate::assistant_turn::AssistantTurn;
use crate::budget::TokenBudgetDecision;
use crate::budget_state::BudgetState;
use crate::file_history::{FileHistoryPlan, FileHistoryState};
use crate::history_store::{HistoryEntry, HistoryStore, HistoryStorePlan};
use crate::message::QueryMessage;
use crate::model_response::ModelResponse;
use crate::persistence_backend::{
    FileHistorySnapshot, PersistedTranscriptEntry, PersistenceDispatchResult, PersistenceReader,
};
use crate::provider::{
    ModelError, ModelInvocation, ModelProvider, ModelSelection, ModelStreamSink, ToolSchema,
};
use crate::query_config::{QueryConfig, QueryRuntimeGates};
use crate::query_deps::{QueryDeps, production_deps};
use crate::query_loop::{QueryLoopOutcome, QueryLoopParams, QuerySource, TaskBudget};
use crate::session_persistence::{SessionPersistencePlan, SessionPersistenceState};
use crate::tool_execution::{ToolCallInput, ToolCallResult, ToolExecutionRequest, ToolExecutor};
use crate::tool_registry::{
    ToolDefinition, ToolPermissionContext, ToolRegistry, ToolRegistrySelection, ToolRuntimeMode,
    ToolSelectionIssue,
};
use crate::transcript::QueryTranscript;
use crate::usage_tracker::{UsageSnapshot, UsageTotals, UsageTracker};
use serde_json::Value;
use std::io;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueryEngineModule;

impl QueryEngineModule {
    pub const LABEL: &'static str = "query-engine";
    pub const TS_SOURCE: &'static str = "src/QueryEngine.ts";
    pub const RESPONSIBILITY: &'static str =
        "Owns conversation lifecycle, session-scoped state, and the ask()/submitMessage bridge.";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThinkingMode {
    #[default]
    Adaptive,
    Disabled,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueryEngineConfig {
    pub cwd: String,
    pub session_id: String,
    pub persist_session: bool,
    pub persist_history: bool,
    pub file_history_enabled: bool,
    pub tools: Vec<String>,
    pub tool_runtime_mode: ToolRuntimeMode,
    pub tool_permission_context: ToolPermissionContext,
    pub commands: Vec<String>,
    pub mcp_clients: Vec<String>,
    pub agents: Vec<String>,
    pub initial_messages: Vec<QueryMessage>,
    pub read_file_cache_entries: usize,
    pub custom_system_prompt: Option<String>,
    pub append_system_prompt: Option<String>,
    pub model_provider: ModelProvider,
    pub user_specified_model: Option<String>,
    pub fallback_model: Option<String>,
    pub model_reasoning_effort: Option<String>,
    pub thinking_mode: ThinkingMode,
    pub max_turns: Option<u32>,
    pub max_budget_usd: Option<f64>,
    pub task_budget: Option<TaskBudget>,
    pub json_schema: Option<String>,
    pub verbose: bool,
    pub replay_user_messages: bool,
    pub include_partial_messages: bool,
    pub stream_model_responses: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResumeSnapshot {
    pub transcript: Vec<PersistedTranscriptEntry>,
    pub history: Vec<HistoryEntry>,
    pub file_history: Option<FileHistorySnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryEngineState {
    pub mutable_messages: Vec<QueryMessage>,
    pub completed_turns: Vec<AssistantTurn>,
    pub completed_responses: Vec<ModelResponse>,
    pub permission_denials: Vec<String>,
    pub total_usage: UsageTotals,
    pub usage_tracker: UsageTracker,
    pub budget_state: BudgetState,
    pub history_store: HistoryStore,
    pub file_history: FileHistoryState,
    pub session_persistence: SessionPersistenceState,
    pub resume_snapshot: ResumeSnapshot,
    pub has_handled_orphaned_permission: bool,
    pub read_file_cache_entries: usize,
}

impl QueryEngineState {
    fn from_config(config: &QueryEngineConfig) -> Self {
        Self::from_config_with_resume(config, ResumeSnapshot::default())
    }

    fn from_config_with_resume(
        config: &QueryEngineConfig,
        resume_snapshot: ResumeSnapshot,
    ) -> Self {
        state::build_query_engine_state(config, resume_snapshot)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SubmitMessageOptions {
    pub uuid: Option<String>,
    pub is_meta: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuerySubmissionPlan {
    pub prompt: String,
    pub prompt_uuid: Option<String>,
    pub is_meta: bool,
    pub query_config: QueryConfig,
    pub message_count_after_submit: usize,
    pub loop_params: QueryLoopParams,
    pub available_tools: Vec<ToolDefinition>,
    pub unavailable_tools: Vec<ToolSelectionIssue>,
    pub steps: Vec<QueryPlanStep>,
    pub requested_tools: Vec<ToolCallInput>,
    pub tool_results: Vec<ToolCallResult>,
    pub token_budget_decision: Option<TokenBudgetDecision>,
    pub budget_state: BudgetState,
    pub history_store: HistoryStorePlan,
    pub file_history: FileHistoryPlan,
    pub assistant_turn: AssistantTurn,
    pub model_response: ModelResponse,
    pub model_error: Option<ModelError>,
    pub model_invocation: Option<ModelInvocation>,
    pub response_result: Option<Value>,
    pub transcript: QueryTranscript,
    pub usage_snapshot: UsageSnapshot,
    pub session_persistence: SessionPersistencePlan,
    pub persistence_dispatch: PersistenceDispatchResult,
    pub loop_outcome: QueryLoopOutcome,
}

impl QuerySubmissionPlan {
    pub fn response_result_preview(&self, max_chars: usize) -> String {
        let Some(response_result) = self.response_result.as_ref() else {
            return String::from("none");
        };
        truncate_preview(response_result.to_string(), max_chars)
    }

    pub fn response_result_pretty(&self) -> Option<String> {
        self.response_result
            .as_ref()
            .and_then(|value| serde_json::to_string_pretty(value).ok())
    }
}

fn truncate_preview(value: String, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value;
    }
    let keep = max_chars.saturating_sub(3);
    let preview = value.chars().take(keep).collect::<String>();
    format!("{preview}...")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryPlanStep {
    pub action: String,
    pub outcome: QueryLoopOutcome,
}

#[derive(Debug, Clone)]
pub struct QueryEngine {
    config: QueryEngineConfig,
    state: QueryEngineState,
    deps: QueryDeps,
}

impl QueryEngine {
    pub fn new(config: QueryEngineConfig) -> Self {
        Self::with_deps(config, production_deps())
    }

    pub fn with_deps(config: QueryEngineConfig, deps: QueryDeps) -> Self {
        let state = QueryEngineState::from_config(&config);
        Self {
            config,
            state,
            deps,
        }
    }

    pub fn resume_with_reader(
        config: QueryEngineConfig,
        reader: &impl PersistenceReader,
    ) -> io::Result<Self> {
        Self::resume_with_reader_and_deps(config, reader, production_deps())
    }

    pub fn resume_with_reader_and_deps(
        config: QueryEngineConfig,
        reader: &impl PersistenceReader,
        deps: QueryDeps,
    ) -> io::Result<Self> {
        let resume_snapshot = state::load_resume_snapshot(&config, reader)?;
        let state = QueryEngineState::from_config_with_resume(&config, resume_snapshot);
        Ok(Self {
            config,
            state,
            deps,
        })
    }

    pub fn config(&self) -> &QueryEngineConfig {
        &self.config
    }

    pub fn state(&self) -> &QueryEngineState {
        &self.state
    }

    pub fn deps(&self) -> &QueryDeps {
        &self.deps
    }

    pub fn build_query_config(&self) -> QueryConfig {
        let mut system_prompt = Vec::new();
        if let Some(prompt) = &self.config.custom_system_prompt {
            system_prompt.push(QueryMessage::system(prompt.clone()));
        }
        if let Some(prompt) = &self.config.append_system_prompt {
            system_prompt.push(QueryMessage::system(prompt.clone()));
        }

        QueryConfig {
            system_prompt,
            user_context_keys: vec![String::from("cwd")],
            system_context_keys: vec![String::from("model")],
            model_selection: ModelSelection {
                provider: self.config.model_provider,
                requested_model: self.config.user_specified_model.clone(),
                fallback_model: self.config.fallback_model.clone(),
            },
            model_reasoning_effort: self.config.model_reasoning_effort.clone(),
            json_schema: self.config.json_schema.clone(),
            query_source: QuerySource::Sdk,
            max_turns: self.config.max_turns,
            task_budget: self.config.task_budget,
            runtime_gates: QueryRuntimeGates {
                verbose: self.config.verbose,
                replay_user_messages: self.config.replay_user_messages,
                include_partial_messages: self.config.include_partial_messages,
                stream_model_responses: self.config.stream_model_responses,
            },
            tool_definitions: build_tool_schemas(&self.config.tools),
        }
    }

    pub fn build_loop_params(&self) -> QueryLoopParams {
        self.build_query_config()
            .to_loop_params(self.state.mutable_messages.clone())
    }

    fn resolve_tool_pool(&self) -> ToolRegistrySelection {
        ToolRegistry::default().select_tools(
            &self.config.tools,
            self.config.tool_runtime_mode,
            &self.config.tool_permission_context,
        )
    }

    fn bootstrap_tool_calls(&self, tool_pool: &ToolRegistrySelection) -> Vec<ToolCallInput> {
        let targets = [
            "src/query.ts",
            "src/QueryEngine.ts",
            "src/services/tools/toolExecution.ts",
        ];
        let selected_targets = if tool_pool.has_tool("Read") {
            targets.as_slice()
        } else {
            &targets[..1]
        };

        selected_targets
            .iter()
            .enumerate()
            .map(|(index, target)| {
                ToolCallInput::new("Read", format!("toolu-bootstrap-{}", index + 1))
                    .with_argument("file_path", *target)
                    .with_context_label("sdk-bootstrap")
            })
            .collect()
    }

    fn bootstrap_execution_request(
        &self,
        tool_call: ToolCallInput,
        tool_pool: &ToolRegistrySelection,
    ) -> ToolExecutionRequest {
        if !tool_pool.has_tool(tool_call.tool_name.as_str()) {
            let deny_reason = tool_pool
                .issue_for(tool_call.tool_name.as_str())
                .map(|issue| issue.reason.clone())
                .unwrap_or_else(|| String::from("tool missing from requested tool pool"));
            return ToolExecutionRequest::denied(tool_call, deny_reason);
        }

        // Evaluate permission rules.
        let args: Vec<(String, String)> = tool_call
            .arguments
            .iter()
            .map(|a| (a.key.clone(), a.value.clone()))
            .collect();
        if let Some(reason) = self
            .config
            .tool_permission_context
            .evaluate(tool_call.tool_name.as_str(), &args)
        {
            return ToolExecutionRequest::denied(tool_call, reason);
        }

        ToolExecutionRequest::allowed(tool_call)
    }

    pub fn submit_message(
        &mut self,
        prompt: impl Into<String>,
        options: SubmitMessageOptions,
    ) -> QuerySubmissionPlan {
        runtime::submit_message(self, prompt.into(), options)
    }

    pub fn submit_message_with_stream(
        &mut self,
        prompt: impl Into<String>,
        options: SubmitMessageOptions,
        stream: &mut dyn ModelStreamSink,
    ) -> QuerySubmissionPlan {
        runtime::submit_message_with_stream(self, prompt.into(), options, stream)
    }

    pub fn submit_message_with_executor(
        &mut self,
        prompt: impl Into<String>,
        options: SubmitMessageOptions,
        executor: &impl ToolExecutor,
    ) -> QuerySubmissionPlan {
        runtime::submit_message_with_executor(self, prompt.into(), options, executor)
    }

    pub fn submit_message_with_executor_and_stream(
        &mut self,
        prompt: impl Into<String>,
        options: SubmitMessageOptions,
        executor: &impl ToolExecutor,
        stream: &mut dyn ModelStreamSink,
    ) -> QuerySubmissionPlan {
        runtime::submit_message_with_executor_and_stream(
            self,
            prompt.into(),
            options,
            executor,
            stream,
        )
    }

    pub fn submit_message_with_runtime(
        &mut self,
        prompt: impl Into<String>,
        options: SubmitMessageOptions,
        executor: &impl ToolExecutor,
        persistence_backend: &mut impl crate::persistence_backend::PersistenceBackend,
    ) -> QuerySubmissionPlan {
        runtime::submit_message_with_runtime(
            self,
            prompt.into(),
            options,
            executor,
            persistence_backend,
        )
    }

    pub fn submit_message_with_runtime_and_stream(
        &mut self,
        prompt: impl Into<String>,
        options: SubmitMessageOptions,
        executor: &impl ToolExecutor,
        persistence_backend: &mut impl crate::persistence_backend::PersistenceBackend,
        stream: &mut dyn ModelStreamSink,
    ) -> QuerySubmissionPlan {
        runtime::submit_message_with_runtime_and_stream(
            self,
            prompt.into(),
            options,
            executor,
            persistence_backend,
            Some(stream),
        )
    }
}

fn build_tool_schemas(tools: &[String]) -> Vec<ToolSchema> {
    use serde_json::json;
    tools
        .iter()
        .filter_map(|name| {
            let schema = match name.as_str() {
                "Read" => ToolSchema {
                    name: String::from("Read"),
                    description: String::from("Read a file from the filesystem."),
                    parameters: json!({
                        "type": "object",
                        "properties": {
                            "file_path": {"type": "string", "description": "Absolute or relative path to the file"}
                        },
                        "required": ["file_path"]
                    }),
                },
                "Edit" => ToolSchema {
                    name: String::from("Edit"),
                    description: String::from("Replace text in a file. The old_string must match exactly."),
                    parameters: json!({
                        "type": "object",
                        "properties": {
                            "file_path": {"type": "string", "description": "Path to the file"},
                            "old_string": {"type": "string", "description": "Exact text to find"},
                            "new_string": {"type": "string", "description": "Replacement text"}
                        },
                        "required": ["file_path", "old_string", "new_string"]
                    }),
                },
                "Write" => ToolSchema {
                    name: String::from("Write"),
                    description: String::from("Create or overwrite a file with the given content."),
                    parameters: json!({
                        "type": "object",
                        "properties": {
                            "file_path": {"type": "string", "description": "Path to the file"},
                            "content": {"type": "string", "description": "File content to write"}
                        },
                        "required": ["file_path", "content"]
                    }),
                },
                "Bash" => ToolSchema {
                    name: String::from("Bash"),
                    description: String::from("Execute a shell command. Use for system commands and terminal operations."),
                    parameters: json!({
                        "type": "object",
                        "properties": {
                            "command": {"type": "string", "description": "The shell command to execute"}
                        },
                        "required": ["command"]
                    }),
                },
                "Glob" => ToolSchema {
                    name: String::from("Glob"),
                    description: String::from("Find files matching a glob pattern."),
                    parameters: json!({
                        "type": "object",
                        "properties": {
                            "pattern": {"type": "string", "description": "Glob pattern (e.g. **/*.rs)"},
                            "path": {"type": "string", "description": "Base directory to search in"}
                        },
                        "required": ["pattern"]
                    }),
                },
                "Grep" => ToolSchema {
                    name: String::from("Grep"),
                    description: String::from("Search file contents for a regex pattern."),
                    parameters: json!({
                        "type": "object",
                        "properties": {
                            "pattern": {"type": "string", "description": "Regex pattern to search for"},
                            "path": {"type": "string", "description": "File or directory to search"},
                            "glob": {"type": "string", "description": "File glob filter (e.g. *.rs)"}
                        },
                        "required": ["pattern"]
                    }),
                },
                "WebFetch" => ToolSchema {
                    name: String::from("WebFetch"),
                    description: String::from("Fetch content from a URL."),
                    parameters: json!({
                        "type": "object",
                        "properties": {
                            "url": {"type": "string", "description": "URL to fetch"}
                        },
                        "required": ["url"]
                    }),
                },
                "WebSearch" => ToolSchema {
                    name: String::from("WebSearch"),
                    description: String::from("Search the web for a query."),
                    parameters: json!({
                        "type": "object",
                        "properties": {
                            "query": {"type": "string", "description": "Search query"}
                        },
                        "required": ["query"]
                    }),
                },
                "Agent" => ToolSchema {
                    name: String::from("Agent"),
                    description: String::from("Spawn a sub-agent to handle a complex task autonomously."),
                    parameters: json!({
                        "type": "object",
                        "properties": {
                            "agent_id": {"type": "string", "description": "Unique agent identifier"},
                            "prompt": {"type": "string", "description": "Task description for the agent"}
                        },
                        "required": ["prompt"]
                    }),
                },
                _ => return None,
            };
            Some(schema)
        })
        .collect()
}

pub fn ask(
    config: QueryEngineConfig,
    prompt: impl Into<String>,
    options: SubmitMessageOptions,
) -> QuerySubmissionPlan {
    let mut engine = QueryEngine::new(config);
    engine.submit_message(prompt, options)
}

pub fn ask_with_executor(
    config: QueryEngineConfig,
    prompt: impl Into<String>,
    options: SubmitMessageOptions,
    executor: &impl ToolExecutor,
) -> QuerySubmissionPlan {
    let mut engine = QueryEngine::new(config);
    engine.submit_message_with_executor(prompt, options, executor)
}

#[cfg(test)]
mod tests;
