mod claudemd;
#[allow(dead_code)]
mod markdown_render;
#[allow(dead_code, clippy::collapsible_if)]
mod markdown_stream;
mod repl;
#[allow(dead_code)]
mod spinner;
#[allow(dead_code)]
mod status_hud;
mod task_panel;
#[allow(dead_code)]
mod tool_render;
#[allow(dead_code, clippy::empty_line_after_doc_comments)]
mod tool_truncate;
mod tui;

use nocode_core::{
    AssistantTurnStatus, BridgeEventWire, BridgeMode, BridgeRequest, BridgeTransportError,
    BridgeWireRequest, BridgeWireResponse, HttpRemoteBridgeAuth, HttpRemoteBridgeTransport,
    HttpRemoteBridgeTransportConfig, LocalPersistenceBackend, ModelError, ModelProvider,
    PermissionCondition, PermissionRequestWire, PermissionResponseWire, PermissionRule,
    ProcessAgentOutputWire, ProcessAgentRequestWire, ProcessAgentResponseWire,
    ProcessAgentStatusWire, QueryEngine, QueryEngineConfig, QueryEngineModule, QueryLoopModule,
    QueryLoopOutcome, QueryLoopTerminal, QueryMessage, QuerySubmissionPlan, RecordingModelStream,
    RemoteBridgeTransport, SessionIdentity, SessionRunner, SubmitMessageOptions, TaskBudget,
    ThinkingMode, ToolExecutionModule, ToolPermissionContext, ToolRegistryModule, ToolRuntimeMode,
    default_roadmap, render_status,
};
use repl::{ReplSession, TuiPermissionRequest};
use std::env;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::Path;
use std::sync::mpsc;

fn workspace_root() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("workspace root should resolve")
        .to_string_lossy()
        .into_owned()
}

fn env_var_optional(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_provider_override() -> Option<ModelProvider> {
    env_var_optional("NOCODE_MODEL_PROVIDER")
        .or_else(|| env_var_optional("NOCODE_PROVIDER"))
        .and_then(|value| ModelProvider::parse(&value))
}

fn env_default_provider() -> ModelProvider {
    if let Some(provider) = env_provider_override() {
        return provider;
    }
    if env_var_optional("GEMINI_API_KEY").is_some() {
        return ModelProvider::Gemini;
    }
    if env_var_optional("OPENAI_API_KEY").is_some() || env_var_optional("OPENAI_BASE_URL").is_some()
    {
        return ModelProvider::OpenAi;
    }
    if env_var_optional("ANTHROPIC_API_KEY").is_some()
        || env_var_optional("ANTHROPIC_BASE_URL").is_some()
    {
        return ModelProvider::Claude;
    }
    ModelProvider::Mock
}

fn env_default_model(provider: ModelProvider) -> Option<String> {
    env_var_optional("NOCODE_MODEL").or_else(|| match provider {
        ModelProvider::Mock => Some(String::from("sonnet")),
        ModelProvider::Claude | ModelProvider::Custom => {
            env_var_optional("ANTHROPIC_MODEL").or_else(|| Some(String::from("claude-opus-4-6")))
        }
        ModelProvider::OpenAi => {
            env_var_optional("OPENAI_MODEL").or_else(|| Some(String::from("gpt-5.4")))
        }
        ModelProvider::Gemini => {
            env_var_optional("GEMINI_MODEL").or_else(|| Some(String::from("gemini-3.1-pro")))
        }
    })
}

fn env_thinking_mode() -> ThinkingMode {
    match env_var_optional("NOCODE_MODEL_REASONING_EFFORT")
        .or_else(|| env_var_optional("MODEL_REASONING_EFFORT"))
        .or_else(|| env_var_optional("OPENAI_REASONING_EFFORT"))
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("off" | "none" | "disabled" | "low") => ThinkingMode::Disabled,
        Some("medium" | "high" | "adaptive") => ThinkingMode::Adaptive,
        _ => ThinkingMode::Adaptive,
    }
}

fn env_reasoning_effort() -> Option<String> {
    match env_var_optional("NOCODE_MODEL_REASONING_EFFORT")
        .or_else(|| env_var_optional("MODEL_REASONING_EFFORT"))
        .or_else(|| env_var_optional("OPENAI_REASONING_EFFORT"))
        .map(|value| value.to_ascii_lowercase())
    {
        Some(value) if matches!(value.as_str(), "minimal" | "low" | "medium" | "high") => {
            Some(value)
        }
        _ => None,
    }
}

fn build_system_prompt() -> String {
    if let Some(custom) = env_var_optional("NOCODE_SYSTEM_PROMPT") {
        return custom;
    }
    let cwd = workspace_root();
    format!(
        "You are nocode, a coding assistant running in a terminal.\n\
         \n\
         # Environment\n\
         - Working directory: {cwd}\n\
         - Platform: {os}\n\
         \n\
         # Available tools\n\
         You have access to these tools:\n\
         - Read: Read a file. Parameters: file_path (required)\n\
         - Edit: Replace text in a file. Parameters: file_path, old_string, new_string (all required)\n\
         - Write: Create or overwrite a file. Parameters: file_path, content (both required)\n\
         - Bash: Run a shell command. Parameters: command (required)\n\
         - Glob: Find files by pattern. Parameters: pattern (required), path (optional)\n\
         - Grep: Search file contents. Parameters: pattern (required), path (optional), glob (optional)\n\
         \n\
         # Guidelines\n\
         - Be concise and direct\n\
         - Use tools to explore and modify the codebase\n\
         - Read files before editing them\n\
         - Prefer Edit over Write for modifying existing files\n\
         - When running shell commands, prefer non-interactive commands\n\
         - Show your work: explain what you find and what you change",
        os = std::env::consts::OS
    )
}

fn build_claude_md_prompt() -> Option<String> {
    let root = workspace_root();
    let cwd = std::path::Path::new(&root);
    let files = claudemd::discover_claude_md_files(cwd);
    claudemd::format_claude_md_for_prompt(&files)
}

fn default_permission_context() -> ToolPermissionContext {
    ToolPermissionContext::default()
        .with_rule(PermissionRule {
            tool_name: String::from("Bash"),
            condition: PermissionCondition::CommandContains(String::from("rm -rf /")),
            reason: String::from("destructive: recursive delete of root filesystem"),
        })
        .with_rule(PermissionRule {
            tool_name: String::from("Bash"),
            condition: PermissionCondition::CommandContains(String::from("rm -rf ~")),
            reason: String::from("destructive: recursive delete of home directory"),
        })
        .with_rule(PermissionRule {
            tool_name: String::from("Bash"),
            condition: PermissionCondition::CommandContains(String::from("mkfs")),
            reason: String::from("destructive: filesystem format command"),
        })
        .with_rule(PermissionRule {
            tool_name: String::from("Bash"),
            condition: PermissionCondition::CommandContains(String::from("dd if=")),
            reason: String::from("destructive: raw disk write"),
        })
        .with_rule(PermissionRule {
            tool_name: String::from("Bash"),
            condition: PermissionCondition::CommandContains(String::from("> /dev/sd")),
            reason: String::from("destructive: raw device write"),
        })
        .with_rule(PermissionRule {
            tool_name: String::from("Bash"),
            condition: PermissionCondition::CommandContains(String::from("shutdown")),
            reason: String::from("system shutdown command"),
        })
        .with_rule(PermissionRule {
            tool_name: String::from("Bash"),
            condition: PermissionCondition::CommandContains(String::from("reboot")),
            reason: String::from("system reboot command"),
        })
        .with_rule(PermissionRule {
            tool_name: String::from("Write"),
            condition: PermissionCondition::ArgumentContains {
                arg_name: String::from("file_path"),
                pattern: String::from("/etc/"),
            },
            reason: String::from("write to system config directory blocked"),
        })
        .with_rule(PermissionRule {
            tool_name: String::from("Write"),
            condition: PermissionCondition::ArgumentContains {
                arg_name: String::from("file_path"),
                pattern: String::from("/boot/"),
            },
            reason: String::from("write to boot directory blocked"),
        })
}

fn bootstrap_config() -> QueryEngineConfig {
    let provider = if cfg!(test) {
        ModelProvider::Mock
    } else {
        env_default_provider()
    };
    let requested_model = if cfg!(test) {
        Some(String::from("sonnet"))
    } else {
        env_default_model(provider)
    };
    let fallback_model = if cfg!(test) {
        Some(String::from("haiku"))
    } else {
        requested_model.clone()
    };
    let thinking_mode = if cfg!(test) {
        ThinkingMode::Adaptive
    } else {
        env_thinking_mode()
    };
    let model_reasoning_effort = if cfg!(test) {
        None
    } else {
        env_reasoning_effort()
    };
    QueryEngineConfig {
        cwd: workspace_root(),
        session_id: String::from("bootstrap-session"),
        persist_session: true,
        persist_history: true,
        file_history_enabled: true,
        tools: vec![
            String::from("Read"),
            String::from("Edit"),
            String::from("Write"),
            String::from("Bash"),
            String::from("Glob"),
            String::from("Grep"),
            String::from("WebFetch"),
            String::from("WebSearch"),
            String::from("Agent"),
        ],
        tool_runtime_mode: ToolRuntimeMode::Standard,
        tool_permission_context: default_permission_context(),
        commands: vec![String::from("/help")],
        mcp_clients: Vec::new(),
        agents: vec![String::from("leader")],
        initial_messages: vec![QueryMessage::system("bootstrap")],
        read_file_cache_entries: 0,
        custom_system_prompt: Some(build_system_prompt()),
        append_system_prompt: build_claude_md_prompt(),
        model_provider: provider,
        user_specified_model: requested_model,
        fallback_model,
        model_reasoning_effort,
        thinking_mode,
        max_turns: Some(4),
        max_budget_usd: None,
        task_budget: Some(TaskBudget { total: 20_000 }),
        json_schema: None,
        verbose: false,
        replay_user_messages: false,
        include_partial_messages: false,
        stream_model_responses: true,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--process-agent-daemon") {
        run_process_agent_daemon();
        return;
    }
    if args.iter().any(|arg| arg == "--process-agent-host") {
        run_process_agent_host();
        return;
    }
    if args.iter().any(|arg| arg == "--tui") {
        run_tui();
        return;
    }
    if args.iter().any(|arg| arg == "--repl") {
        run_repl();
        return;
    }
    if let Some(prompt) = bridge_remote_prompt_from_args(&args) {
        run_bridge_remote_once(prompt);
        return;
    }
    if let Some(prompt) = bridge_prompt_from_args(&args) {
        run_bridge_once(prompt);
        return;
    }
    let show_status = args.iter().any(|arg| arg == "--status");
    if show_status {
        let roadmap = default_roadmap();
        let mut engine = QueryEngine::new(bootstrap_config());
        let plan = engine.submit_message("continue rewrite", SubmitMessageOptions::default());
        println!("{}", render_cli_status_report(&roadmap, &plan));
        return;
    }

    let roadmap = default_roadmap();
    println!("nocode bootstrap");
    print!("{}", render_status(&roadmap));
    println!();
    println!("module-summaries:");
    println!(
        "- {}: {}",
        QueryEngineModule::LABEL,
        QueryEngineModule::RESPONSIBILITY
    );
    println!(
        "- {}: {}",
        QueryLoopModule::LABEL,
        QueryLoopModule::RESPONSIBILITY
    );
    println!(
        "- {}: {}",
        ToolExecutionModule::LABEL,
        ToolExecutionModule::RESPONSIBILITY
    );
    println!(
        "- {}: {}",
        ToolRegistryModule::LABEL,
        ToolRegistryModule::RESPONSIBILITY
    );

    let config = bootstrap_config();
    let mut engine = QueryEngine::new(config.clone());
    let plan = engine.submit_message("continue rewrite", SubmitMessageOptions::default());
    let identity = SessionIdentity::new(config.session_id.clone(), config.cwd.clone());
    let reader = LocalPersistenceBackend::new(
        identity.transcript_path(),
        identity.history_path(),
        identity.file_history_path(),
    );
    let resumed =
        QueryEngine::resume_with_reader(config, &reader).expect("resume should read local state");
    println!();
    println!("query-config:");
    println!(
        "- provider: {}",
        plan.query_config.model_selection.provider.as_str()
    );
    println!(
        "- requested-model: {}",
        plan.query_config
            .model_selection
            .requested_model
            .as_deref()
            .unwrap_or("none")
    );
    println!(
        "- effective-model: {}",
        plan.query_config.selected_model().unwrap_or("none")
    );
    println!(
        "- fallback-model: {}",
        plan.query_config.fallback_model().unwrap_or("none")
    );
    println!(
        "- provider-capabilities: {}",
        plan.query_config
            .model_selection
            .provider
            .capability_summary()
    );
    println!(
        "- provider-capability-matrix: {}",
        ModelProvider::capability_matrix_summary()
    );
    println!(
        "- runtime-gates: verbose={} replay_user_messages={} include_partial_messages={} stream_model_responses={}",
        plan.query_config.runtime_gates.verbose,
        plan.query_config.runtime_gates.replay_user_messages,
        plan.query_config.runtime_gates.include_partial_messages,
        plan.query_config.runtime_gates.stream_model_responses
    );
    println!(
        "- task-budget: {}",
        plan.query_config
            .task_budget
            .map_or(String::from("none"), |budget| budget.total.to_string())
    );
    println!();
    println!("submission-plan:");
    println!("- prompt: {}", plan.prompt);
    println!("- messages: {}", plan.message_count_after_submit);
    println!("- query-source: {}", plan.loop_params.query_source.as_str());
    println!(
        "- available-tools: {}",
        plan.available_tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "- unavailable-tools: {}",
        if plan.unavailable_tools.is_empty() {
            String::from("none")
        } else {
            plan.unavailable_tools
                .iter()
                .map(|issue| format!("{}({})", issue.tool_name, issue.reason))
                .collect::<Vec<_>>()
                .join(", ")
        }
    );
    println!("- requested-tools: {}", plan.requested_tools.len());
    println!("- tool-results: {}", plan.tool_results.len());
    println!(
        "- assistant-turn-status: {}",
        plan.assistant_turn.status.as_str()
    );
    println!(
        "- assistant-turn-messages: {}",
        plan.assistant_turn.response_messages.len()
    );
    println!(
        "- assistant-turn-tools: {}",
        plan.assistant_turn
            .tool_uses
            .iter()
            .map(|tool_use| format!(
                "{}#{}:{}",
                tool_use.tool_name, tool_use.tool_use_id, tool_use.status
            ))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("- model-response-id: {}", plan.model_response.response_id);
    println!(
        "- model-invocation: provider={} path={} model={} requested={} fallback={} stream-mode={} stream={}",
        plan.model_invocation
            .as_ref()
            .map(|invocation| invocation.provider.as_str())
            .unwrap_or("none"),
        plan.model_invocation
            .as_ref()
            .map(|invocation| invocation.http_request.path.as_str())
            .unwrap_or("none"),
        plan.model_invocation
            .as_ref()
            .map(|invocation| invocation.model.as_str())
            .unwrap_or("none"),
        plan.model_invocation
            .as_ref()
            .and_then(|invocation| invocation.requested_model.as_deref())
            .unwrap_or("none"),
        plan.model_invocation
            .as_ref()
            .and_then(|invocation| invocation.fallback_model.as_deref())
            .unwrap_or("none"),
        plan.model_invocation
            .as_ref()
            .map(|invocation| invocation.stream_mode.as_str())
            .unwrap_or("none"),
        plan.model_invocation.as_ref().map_or_else(
            || String::from("total=0 delta=0 chars=0 start=no complete=no"),
            |invocation| invocation.stream_summary(),
        )
    );
    println!(
        "- transport-url: {}",
        plan.model_invocation
            .as_ref()
            .map(|invocation| invocation.transport_request.url.as_str())
            .unwrap_or("none")
    );
    println!(
        "- model-response-stop: {}",
        plan.model_response.stop_reason.as_str()
    );
    println!(
        "- model-response-tool-phase: {}/{}",
        plan.model_response.tool_phase.resolved_tools,
        plan.model_response.tool_phase.requested_tools
    );
    println!(
        "- model-response-final-assistant: {}",
        plan.model_response
            .final_assistant_message
            .as_ref()
            .map(|message| message.content.as_str())
            .unwrap_or("none")
    );
    if let Some(error) = &plan.model_error {
        println!(
            "- model-error: kind={} retryable={} provider={} status={} class={} message={}",
            error.kind.as_str(),
            error.retryable,
            error
                .provider
                .map(|provider| provider.as_str())
                .unwrap_or("none"),
            error
                .status_code
                .map(|status| status.to_string())
                .unwrap_or_else(|| String::from("none")),
            error
                .status_class
                .map(|class| class.as_str())
                .unwrap_or("none"),
            error.message
        );
    }
    println!(
        "- token-budget-action: {}",
        plan.token_budget_decision
            .as_ref()
            .map_or("none", |decision| decision.action_label())
    );
    println!(
        "- budget-state: budget={} turn-output={} continuations={}",
        plan.budget_state
            .current_turn_budget
            .map_or(String::from("none"), |budget| budget.to_string()),
        plan.budget_state.current_turn_output_tokens,
        plan.budget_state.continuation_count()
    );
    println!("- usage-input-tokens: {}", plan.usage_snapshot.input_tokens);
    println!(
        "- usage-output-tokens: {}",
        plan.usage_snapshot.output_tokens
    );
    println!(
        "- usage-total: in={} out={}",
        plan.usage_snapshot.total_usage.input_tokens, plan.usage_snapshot.total_usage.output_tokens
    );
    println!(
        concat!(
            "- session-persistence: persist={} transcript_flushes={} ",
            "history_entries={} transcript_entries={} history_flushes={} ",
            "file_committed={} session={}"
        ),
        plan.session_persistence.persist_session,
        plan.session_persistence.transcript_flushes,
        plan.session_persistence.history_entries,
        plan.session_persistence.transcript_entries,
        plan.session_persistence.history_flushes,
        plan.session_persistence.file_history_committed,
        plan.session_persistence.session_id
    );
    println!(
        "- history-store: persist={} pending={} flushes={}",
        plan.history_store.persist_history,
        plan.history_store.pending_entries,
        plan.history_store.flush_count
    );
    println!(
        "- file-history: requested={} requests={} committed={}",
        plan.file_history.snapshot_requested,
        plan.file_history.total_requests,
        plan.file_history.total_committed
    );
    println!(
        "- persistence-backend: transcript={} history={} file_history={}",
        plan.persistence_dispatch.transcript_entries_flushed,
        plan.persistence_dispatch.history_persisted,
        plan.persistence_dispatch.file_history_persisted
    );
    println!(
        "- resume-snapshot: transcript={} history={} file_history={}",
        resumed.state().resume_snapshot.transcript.len(),
        resumed.state().resume_snapshot.history.len(),
        resumed
            .state()
            .resume_snapshot
            .file_history
            .as_ref()
            .map_or(String::from("none"), |snapshot| format!(
                "requested={} committed={}",
                snapshot.total_requests, snapshot.total_committed
            ))
    );
    let stop_hook_summary = match &plan.loop_outcome {
        QueryLoopOutcome::Continue(state) => state
            .last_stop_hook_result
            .as_ref()
            .map_or(String::from("none"), |result| result.summary()),
        QueryLoopOutcome::Terminal(_) => String::from("none"),
    };
    println!("- stop-hooks: {}", stop_hook_summary);
    println!("- transcript-entries: {}", plan.transcript.entries.len());
    println!("- step-trace:");
    for step in &plan.steps {
        let status = match &step.outcome {
            QueryLoopOutcome::Continue(state) => format!(
                "continue(turn={}, pending-tool={})",
                state.turn_count,
                state.pending_tool_call.is_some()
            ),
            QueryLoopOutcome::Terminal(reason) => format!("terminal({reason:?})"),
        };
        println!("  - {} => {}", step.action, status);
    }
    match &plan.loop_outcome {
        QueryLoopOutcome::Continue(state) => {
            println!("- outcome: continue");
            println!("- tool-context: {}", state.tool_use_context_label);
            println!("- turn-count: {}", state.turn_count);
            println!("- tool-progress-events: {}", state.tool_progress_log.len());
            println!("- resolved-tools: {}", state.tool_results.len());
        }
        QueryLoopOutcome::Terminal(reason) => {
            println!("- outcome: terminal");
            println!("- reason: {}", render_terminal_reason(reason));
        }
    }
}

fn bridge_prompt_from_args(args: &[String]) -> Option<String> {
    let index = args.iter().position(|arg| arg == "--bridge-once")?;
    args.get(index + 1)
        .filter(|next| !next.starts_with("--"))
        .cloned()
        .or_else(|| Some(String::from("bridge rewrite")))
}

fn bridge_remote_prompt_from_args(args: &[String]) -> Option<String> {
    let index = args.iter().position(|arg| arg == "--bridge-remote-once")?;
    args.get(index + 1)
        .filter(|next| !next.starts_with("--"))
        .cloned()
        .or_else(|| Some(String::from("remote bridge rewrite")))
}

fn run_repl() {
    let stdin = io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    let mut engine = QueryEngine::new(bootstrap_config());
    let mut session = ReplSession::new("nocode");
    if let Err(err) = session.run_loop(&mut engine, &mut reader, &mut writer) {
        eprintln!("nocode repl error: {err}");
    }
}

fn run_tui() {
    if let Err(err) = tui::run_tui() {
        eprintln!("nocode tui error: {err}");
    }
}

fn run_bridge_once(prompt: String) {
    let engine = QueryEngine::new(bootstrap_config());
    let mut runner = SessionRunner::new(engine, BridgeMode::LocalRepl);
    let turn = runner.run(BridgeRequest::approved(prompt, "cli-bridge"));
    println!("{}", render_bridge_turn_output(&turn));
}

#[derive(Debug)]
struct CliLoopbackRemoteTransport {
    permission_tx: Option<mpsc::Sender<TuiPermissionRequest>>,
}

impl CliLoopbackRemoteTransport {
    fn new() -> Self {
        Self {
            permission_tx: None,
        }
    }

    #[allow(dead_code)]
    fn with_permission_channel(tx: mpsc::Sender<TuiPermissionRequest>) -> Self {
        Self {
            permission_tx: Some(tx),
        }
    }
}

impl RemoteBridgeTransport for CliLoopbackRemoteTransport {
    fn publish_request(&mut self, request: &BridgeWireRequest) -> Result<(), BridgeTransportError> {
        let encoded = request.to_json().map_err(|error| {
            BridgeTransportError::transport("publish_request", error.to_string())
        })?;
        let _decoded = BridgeWireRequest::from_json(&encoded).map_err(|error| {
            BridgeTransportError::transport("publish_request", error.to_string())
        })?;
        Ok(())
    }

    fn request_permission(
        &mut self,
        request: &PermissionRequestWire,
    ) -> Result<PermissionResponseWire, BridgeTransportError> {
        let encoded = request.to_json().map_err(|error| {
            BridgeTransportError::transport("request_permission", error.to_string())
        })?;
        let decoded = PermissionRequestWire::from_json(&encoded).map_err(|error| {
            BridgeTransportError::transport("request_permission", error.to_string())
        })?;

        // If a TUI permission channel is wired, send the request and wait for response.
        if let Some(tx) = &self.permission_tx {
            let (response_tx, response_rx) = mpsc::channel();
            let tui_req = TuiPermissionRequest {
                id: decoded.request_id.clone(),
                tool_name: decoded.mode.as_str().to_string(),
                description: decoded.prompt.clone(),
                response_tx,
            };
            tx.send(tui_req).map_err(|error| {
                BridgeTransportError::transport("request_permission", error.to_string())
            })?;
            let approved = response_rx.recv().map_err(|error| {
                BridgeTransportError::transport("request_permission", error.to_string())
            })?;
            if approved {
                Ok(PermissionResponseWire::approved(decoded.request_id))
            } else {
                Ok(PermissionResponseWire::denied(
                    decoded.request_id,
                    "user denied via TUI",
                ))
            }
        } else {
            // No TUI channel — auto-approve (loopback demo mode).
            Ok(PermissionResponseWire::approved(decoded.request_id))
        }
    }

    fn publish_event(&mut self, event: &BridgeEventWire) -> Result<(), BridgeTransportError> {
        let encoded = event
            .to_json()
            .map_err(|error| BridgeTransportError::transport("publish_event", error.to_string()))?;
        let _decoded = BridgeEventWire::from_json(&encoded)
            .map_err(|error| BridgeTransportError::transport("publish_event", error.to_string()))?;
        Ok(())
    }

    fn publish_response(
        &mut self,
        response: &BridgeWireResponse,
    ) -> Result<(), BridgeTransportError> {
        let encoded = response.to_json().map_err(|error| {
            BridgeTransportError::transport("publish_response", error.to_string())
        })?;
        let _decoded = BridgeWireResponse::from_json(&encoded).map_err(|error| {
            BridgeTransportError::transport("publish_response", error.to_string())
        })?;
        Ok(())
    }
}

fn bridge_http_transport_from_env()
-> Option<Result<HttpRemoteBridgeTransport, BridgeTransportError>> {
    let base_url = std::env::var("NOCODE_BRIDGE_BASE_URL").ok()?;
    let mut config = HttpRemoteBridgeTransportConfig::new(base_url);
    if let Ok(path) = std::env::var("NOCODE_BRIDGE_REQUEST_PATH") {
        config.request_path = path;
    }
    if let Ok(path) = std::env::var("NOCODE_BRIDGE_PERMISSION_PATH") {
        config.permission_path = path;
    }
    if let Ok(path) = std::env::var("NOCODE_BRIDGE_EVENT_PATH") {
        config.event_path = path;
    }
    if let Ok(path) = std::env::var("NOCODE_BRIDGE_RESPONSE_PATH") {
        config.response_path = path;
    }
    if let Ok(timeout_secs) = std::env::var("NOCODE_BRIDGE_TIMEOUT_SECS")
        && let Ok(parsed) = timeout_secs.parse::<u64>()
    {
        config.timeout_secs = parsed.max(1);
    }
    if let Ok(token) = std::env::var("NOCODE_BRIDGE_AUTH_TOKEN") {
        config.auth = Some(HttpRemoteBridgeAuth::BearerToken(token));
    } else if let (Ok(name), Ok(value)) = (
        std::env::var("NOCODE_BRIDGE_AUTH_HEADER"),
        std::env::var("NOCODE_BRIDGE_AUTH_VALUE"),
    ) {
        config.auth = Some(HttpRemoteBridgeAuth::Header { name, value });
    }
    Some(HttpRemoteBridgeTransport::new(config))
}

fn run_bridge_remote_once(prompt: String) {
    let engine = QueryEngine::new(bootstrap_config());
    let mut runner = SessionRunner::new(engine, BridgeMode::Remote);
    let http_transport = bridge_http_transport_from_env();
    let request = if http_transport.is_some() {
        BridgeRequest::remote(prompt, "cli-remote-http")
    } else {
        BridgeRequest::remote(prompt, "cli-remote-loopback")
    };

    let turn = match http_transport {
        Some(Ok(mut transport)) => runner.run_remote(request, &mut transport),
        Some(Err(error)) => Err(error),
        None => {
            let mut transport = CliLoopbackRemoteTransport::new();
            runner.run_remote(request, &mut transport)
        }
    };

    match turn {
        Ok(turn) => println!("{}", render_bridge_turn_output(&turn)),
        Err(error) => eprintln!("bridge-remote error: {:?}", error),
    }
}

fn render_bridge_turn_output(turn: &nocode_core::BridgeTurn) -> String {
    match &turn.outcome {
        nocode_core::BridgeTurnOutcome::Submitted(plan) => {
            let mut sections = vec![turn.summary()];
            if let Some(invocation) = plan.model_invocation.as_ref() {
                sections.push(format!("stream: {}", invocation.stream_summary()));
            }
            if let Some(error) = plan.model_error.as_ref() {
                sections.push(format!(
                    "model error: surface={} kind={} retryable={} message={}",
                    error.surface_label(),
                    error.kind.as_str(),
                    error.retryable,
                    error.message
                ));
            }
            if let Some(pretty) = plan.response_result_pretty() {
                sections.push(format!("response result:\n{pretty}"));
            }
            sections.join("\n")
        }
        nocode_core::BridgeTurnOutcome::PermissionDenied { .. } => turn.summary(),
    }
}

fn render_cli_status_report(
    roadmap: &nocode_core::RewriteRoadmap,
    plan: &QuerySubmissionPlan,
) -> String {
    format!(
        "nocode status\n{}\nquery-status:\n{}",
        render_status(roadmap),
        render_submission_summary(plan)
    )
}

fn run_process_agent_host() {
    let result = (|| -> Result<(), String> {
        let mut raw = String::new();
        io::stdin()
            .lock()
            .read_to_string(&mut raw)
            .map_err(|error| format!("failed to read process agent request: {error}"))?;
        let request = ProcessAgentRequestWire::from_json(raw.trim())
            .map_err(|error| format!("failed to decode process agent request: {error}"))?;
        let frames = handle_process_agent_request_frames(bootstrap_config(), request)?;
        for frame in frames {
            println!(
                "{}",
                frame
                    .to_json()
                    .map_err(|error| format!("failed to encode process agent response: {error}"))?
            );
        }
        Ok(())
    })();

    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run_process_agent_daemon() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = stdout.lock();
    let mut line = String::new();

    loop {
        line.clear();
        let read = match reader.read_line(&mut line) {
            Ok(read) => read,
            Err(error) => {
                eprintln!("failed to read process agent daemon request: {error}");
                std::process::exit(1);
            }
        };
        if read == 0 {
            break;
        }
        let raw = line.trim();
        if raw.is_empty() {
            continue;
        }

        let request = match ProcessAgentRequestWire::from_json(raw) {
            Ok(request) => request,
            Err(error) => {
                eprintln!("failed to decode process agent daemon request: {error}");
                std::process::exit(1);
            }
        };
        let frames = match handle_process_agent_request_frames(bootstrap_config(), request) {
            Ok(response) => response,
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        };
        for frame in frames {
            let encoded = match frame.to_json() {
                Ok(encoded) => encoded,
                Err(error) => {
                    eprintln!("failed to encode process agent daemon response: {error}");
                    std::process::exit(1);
                }
            };
            if writeln!(writer, "{encoded}")
                .and_then(|_| writer.flush())
                .is_err()
            {
                eprintln!("failed to write process agent daemon response");
                std::process::exit(1);
            }
        }
    }
}

fn handle_process_agent_request(
    mut config: QueryEngineConfig,
    request: ProcessAgentRequestWire,
) -> Result<ProcessAgentResponseWire, String> {
    if let Ok(cwd) = std::env::current_dir() {
        config.cwd = cwd.to_string_lossy().into_owned();
    }
    config.session_id = format!("{}-{}", config.session_id, request.agent_id);
    let mut engine = QueryEngine::new(config);
    let mut stream = RecordingModelStream::default();
    let plan = engine.submit_message_with_stream(
        request.prompt,
        SubmitMessageOptions::default(),
        &mut stream,
    );
    let status = match plan.assistant_turn.status {
        AssistantTurnStatus::Continue => ProcessAgentStatusWire::Running,
        AssistantTurnStatus::Completed => ProcessAgentStatusWire::Completed,
        AssistantTurnStatus::Terminal => ProcessAgentStatusWire::Failed,
    };
    let tool_use_delta = u32::try_from(plan.tool_results.len()).unwrap_or(u32::MAX);
    let token_delta = u32::try_from(plan.usage_snapshot.output_tokens).unwrap_or(u32::MAX);
    let retrieved = plan.model_response.final_assistant_message.is_some();
    let mut response =
        ProcessAgentResponseWire::new(tool_use_delta, token_delta, retrieved, status)
            .with_stream_events(
                stream
                    .events
                    .iter()
                    .map(nocode_core::ModelStreamEventWire::from)
                    .collect::<Vec<_>>(),
            );
    if let Some(response_result) = plan.response_result.clone() {
        response = response.with_response_result(response_result);
    }
    if let Some(model_error) = plan
        .model_error
        .as_ref()
        .map(nocode_core::ModelErrorWire::from)
    {
        response = response.with_model_error(model_error);
    }
    Ok(response)
}

fn handle_process_agent_request_frames(
    config: QueryEngineConfig,
    request: ProcessAgentRequestWire,
) -> Result<Vec<ProcessAgentOutputWire>, String> {
    let response = handle_process_agent_request(config, request)?;
    let mut frames = response
        .stream_events
        .iter()
        .cloned()
        .map(ProcessAgentOutputWire::event)
        .collect::<Vec<_>>();
    if let Some(model_error) = response.model_error.clone() {
        frames.push(ProcessAgentOutputWire::model_error(model_error));
    }
    let mut completion = ProcessAgentResponseWire::new(
        response.tool_use_delta,
        response.token_delta,
        response.retrieved,
        response.status,
    );
    if let Some(response_result) = response.response_result.clone() {
        completion = completion.with_response_result(response_result);
    }
    frames.push(ProcessAgentOutputWire::complete(completion));
    Ok(frames)
}

fn render_submission_summary(plan: &QuerySubmissionPlan) -> String {
    let invocation = plan.model_invocation.as_ref();
    let selected_provider = invocation
        .map(|inv| inv.provider)
        .unwrap_or(plan.query_config.model_selection.provider);
    let provider = invocation
        .map(|inv| inv.provider.as_str())
        .unwrap_or(selected_provider.as_str());
    let model = invocation.map(|inv| inv.model.as_str()).unwrap_or("none");
    let (transport_url, transport_method) = invocation
        .map(|inv| {
            (
                inv.transport_request.url.as_str(),
                format!("{:?}", inv.transport_request.method),
            )
        })
        .unwrap_or(("none", "none".to_string()));
    let headers = invocation
        .map(|inv| inv.transport_request.headers.len())
        .unwrap_or(0);
    let body_preview = invocation
        .and_then(|inv| inv.transport_request.body.as_deref())
        .unwrap_or("none");
    let stream_summary = invocation
        .map(|inv| inv.stream_summary())
        .unwrap_or_else(|| String::from("total=0 delta=0 chars=0 start=no complete=no"));
    let capabilities = selected_provider.capability_summary();
    let capability_matrix = ModelProvider::capability_matrix_summary();
    let tools = plan.tool_results.len();
    let turn_count = match &plan.loop_outcome {
        QueryLoopOutcome::Continue(state) => state.turn_count,
        QueryLoopOutcome::Terminal(_) => 0,
    };
    let error_summary = plan.model_error.as_ref().map_or_else(
        || String::from("none"),
        |error| {
            format!(
                "{}:{}:{}:{}",
                error.surface_label(),
                error.kind.as_str(),
                error
                    .status_class
                    .map(|class| class.as_str())
                    .unwrap_or("none"),
                error.retryable
            )
        },
    );
    format!(
        "status summary: provider={} caps={} matrix={} model={} transport={}({}) headers={} body={} stream={} tools={} turn-count={} response-result={} error={}",
        provider,
        capabilities,
        capability_matrix,
        model,
        transport_url,
        transport_method,
        headers,
        body_preview,
        stream_summary,
        tools,
        turn_count,
        plan.response_result_preview(96),
        error_summary
    )
}

fn render_terminal_reason(reason: &QueryLoopTerminal) -> String {
    match reason {
        QueryLoopTerminal::ModelError { error } => {
            format!("model_error({})", render_model_error(error))
        }
        QueryLoopTerminal::MaxTurns { turn_count } => format!("max_turns({turn_count})"),
        other => format!("{other:?}"),
    }
}

fn render_model_error(error: &ModelError) -> String {
    format!(
        "surface={} kind={} retryable={} provider={} status={} class={} message={}",
        error.surface_label(),
        error.kind.as_str(),
        error.retryable,
        error
            .provider
            .map(|provider| provider.as_str())
            .unwrap_or("none"),
        error
            .status_code
            .map(|status| status.to_string())
            .unwrap_or_else(|| String::from("none")),
        error
            .status_class
            .map(|class| class.as_str())
            .unwrap_or("none"),
        error.message
    )
}

#[cfg(test)]
mod tests {
    use super::{
        bootstrap_config, handle_process_agent_request, handle_process_agent_request_frames,
        render_bridge_turn_output, render_cli_status_report, render_submission_summary,
    };
    use nocode_core::{
        BridgeMode, BridgeRequest, CallModel, ModelCallOutput, ModelError, ModelRequest,
        ModelStreamSink, ProcessAgentOutputWire, ProcessAgentRequestWire, ProcessAgentStatusWire,
        QueryDeps, QueryEngine, SessionRunner, SubmitMessageOptions, default_roadmap,
    };
    use serde_json::json;

    #[derive(Debug)]
    struct StructuredCallModel;

    impl CallModel for StructuredCallModel {
        fn call_model(
            &self,
            request: &ModelRequest,
            _stream: &mut dyn ModelStreamSink,
        ) -> Result<ModelCallOutput, ModelError> {
            let selected_model = request
                .selection
                .selected_model()
                .ok_or_else(ModelError::no_model_configured)?;
            Ok(ModelCallOutput::new(
                request.selection.provider,
                selected_model,
                nocode_core::QueryMessage::assistant("{\"ok\":true,\"source\":\"bridge-cli\"}"),
            )
            .with_response_result(json!({"ok": true, "source": "bridge-cli"})))
        }
    }

    #[test]
    fn process_agent_host_handles_prompt_with_query_engine() {
        let response = handle_process_agent_request(
            bootstrap_config(),
            ProcessAgentRequestWire::new("agent-alpha", "process-host turn"),
        )
        .expect("process agent host should handle prompt");

        assert_eq!(response.status, ProcessAgentStatusWire::Completed);
        assert!(response.retrieved);
        assert!(response.token_delta > 0);
        assert_eq!(response.stream_events.len(), 3);
    }

    #[test]
    fn process_agent_host_frames_emit_stream_then_complete() {
        let frames = handle_process_agent_request_frames(
            bootstrap_config(),
            ProcessAgentRequestWire::new("agent-alpha", "process-host turn"),
        )
        .expect("process agent host should build protocol frames");

        assert_eq!(frames.len(), 4);
        assert!(matches!(frames[0], ProcessAgentOutputWire::Event { .. }));
        assert!(matches!(frames[1], ProcessAgentOutputWire::Event { .. }));
        assert!(matches!(frames[2], ProcessAgentOutputWire::Event { .. }));
        assert!(matches!(frames[3], ProcessAgentOutputWire::Complete { .. }));
    }

    #[test]
    fn status_summary_surfaces_provider_capability_matrix() {
        let mut engine = QueryEngine::new(bootstrap_config());
        let plan = engine.submit_message("cap matrix", SubmitMessageOptions::default());
        let rendered = render_submission_summary(&plan);

        assert!(rendered.contains("caps=stream(request=yes,live=no,sse=no)"));
        assert!(rendered.contains("matrix=mock[stream(request=yes,live=no,sse=no)"));
        assert!(rendered.contains("anthropic[stream(request=yes,live=yes,sse=yes)"));
        assert!(
            rendered.contains(
                "openai[stream(request=yes,live=yes,sse=yes) tool-use=yes json-schema=yes"
            )
        );
        assert!(
            rendered.contains(
                "google[stream(request=yes,live=yes,sse=yes) tool-use=yes json-schema=yes"
            )
        );
        assert!(rendered.contains("stream=total=3 delta=1 chars="));
        assert!(rendered.contains("start=yes complete=yes"));
        assert!(rendered.contains("response-result=none"));
    }

    #[test]
    fn cli_status_report_combines_roadmap_and_query_status() {
        let roadmap = default_roadmap();
        let mut engine = QueryEngine::new(bootstrap_config());
        let plan = engine.submit_message("cap matrix", SubmitMessageOptions::default());

        let rendered = render_cli_status_report(&roadmap, &plan);

        assert!(rendered.starts_with("nocode status"));
        assert!(rendered.contains("active-stage:"));
        assert!(rendered.contains("launch-readiness: no"));
        assert!(rendered.contains("progress: completed="));
        assert!(rendered.contains("current-focus:"));
        assert!(rendered.contains("next-blockers:"));
        assert!(rendered.contains("release-gates:"));
        assert!(rendered.contains("[in_progress] provider-productization"));
        assert!(rendered.contains("[blocked] platform-release"));
        assert!(rendered.contains("query-status:"));
        assert!(rendered.contains("status summary: provider="));
    }

    #[test]
    fn bridge_turn_output_expands_structured_result() {
        let deps = QueryDeps::builder()
            .with_call_model(StructuredCallModel)
            .build();
        let engine = QueryEngine::with_deps(bootstrap_config(), deps);
        let mut runner = SessionRunner::new(engine, BridgeMode::LocalRepl);
        let turn = runner.run(BridgeRequest::approved("bridge detail", "cli-bridge"));

        let rendered = render_bridge_turn_output(&turn);

        assert!(rendered.contains("bridge-turn: mode=local-repl"));
        assert!(rendered.contains("response-result=yes"));
        assert!(rendered.contains("response result:"));
        assert!(rendered.contains("\"source\": \"bridge-cli\""));
    }
}
