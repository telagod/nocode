use super::model::{
    ToolCallInput, ToolCallOutput, ToolCallResult, ToolExecutionRequest, ToolExecutionTrace,
    ToolPermissionDecision, ToolProgressUpdate,
};
use crate::bash_validation::validate_bash_command;
use crate::file_safety::{validate_read_target, validate_write_target};
use crate::message::QueryMessage;
use crate::provider::ModelProvider;
use crate::query_engine::QueryEngineConfig;
use crate::query_engine::ThinkingMode;
use crate::sandbox::{FilesystemIsolationMode, SandboxRequest, resolve_sandbox_status};
use crate::task_runtime::{InProcessAgentHost, TaskAgentHost, TaskId};
use crate::tool_registry::ToolRuntimeMode;
use crate::worker_boot::{WorkerEventKind, global_worker_registry};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Strategy for resolving tool permission decisions.
/// Implementations can auto-approve, auto-deny, or prompt the user interactively.
pub trait PermissionPrompter: Send + Sync + std::fmt::Debug {
    /// Decide whether a tool call should proceed.
    fn check(&self, tool_name: &str, arguments_summary: &str) -> ToolPermissionDecision;
    fn name(&self) -> &str;
}

/// Auto-approves all tool calls. Used in non-interactive / test contexts.
#[derive(Debug, Clone, Copy, Default)]
pub struct AutoApprovePrompter;

impl PermissionPrompter for AutoApprovePrompter {
    fn check(&self, _tool_name: &str, _arguments_summary: &str) -> ToolPermissionDecision {
        ToolPermissionDecision::allow(false)
    }
    fn name(&self) -> &str {
        "auto-approve"
    }
}

/// Auto-denies all tool calls. Used in read-only / locked contexts.
#[derive(Debug, Clone, Copy, Default)]
pub struct AutoDenyPrompter {
    _private: (),
}

impl AutoDenyPrompter {
    pub fn new() -> Self {
        Self { _private: () }
    }
}

impl PermissionPrompter for AutoDenyPrompter {
    fn check(&self, tool_name: &str, _arguments_summary: &str) -> ToolPermissionDecision {
        ToolPermissionDecision::deny(format!("auto-deny: {tool_name}"))
    }
    fn name(&self) -> &str {
        "auto-deny"
    }
}

/// Returns Prompt for tools that require elevated permissions, Allow for safe tools.
#[derive(Debug, Clone)]
pub struct InteractivePrompter {
    /// Tools that always require user approval.
    pub prompt_tools: Vec<String>,
}

impl InteractivePrompter {
    pub fn new(prompt_tools: Vec<String>) -> Self {
        Self { prompt_tools }
    }

    pub fn default_dangerous() -> Self {
        Self {
            prompt_tools: vec![
                "Bash".into(),
                "Write".into(),
                "Edit".into(),
                "WebFetch".into(),
                "WebSearch".into(),
                "Agent".into(),
                "TeamCreate".into(),
                "TeamDelete".into(),
                "CronCreate".into(),
                "CronDelete".into(),
            ],
        }
    }
}

impl PermissionPrompter for InteractivePrompter {
    fn check(&self, tool_name: &str, _arguments_summary: &str) -> ToolPermissionDecision {
        if self.prompt_tools.iter().any(|t| t == tool_name) {
            ToolPermissionDecision::prompt(tool_name, format!("{tool_name} requires approval"))
        } else {
            ToolPermissionDecision::allow(false)
        }
    }
    fn name(&self) -> &str {
        "interactive"
    }
}

/// Audit record of a permission decision.
#[derive(Debug, Clone)]
pub struct PermissionAuditEntry {
    pub tool_name: String,
    pub decision: String,
    pub prompter: String,
    pub timestamp_ms: u64,
}

/// Tracks permission decisions for observability.
#[derive(Debug, Default)]
pub struct PermissionAuditLog {
    pub entries: Vec<PermissionAuditEntry>,
}

impl PermissionAuditLog {
    pub fn record(&mut self, tool_name: &str, decision: &str, prompter: &str) {
        self.entries.push(PermissionAuditEntry {
            tool_name: tool_name.to_string(),
            decision: decision.to_string(),
            prompter: prompter.to_string(),
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        });
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolExecutionContext {
    pub cwd: String,
    pub model_provider: ModelProvider,
    pub model_name: Option<String>,
    pub permission_mode: crate::tool_registry::PermissionMode,
}

impl ToolExecutionContext {
    pub fn new(cwd: impl Into<String>) -> Self {
        Self {
            cwd: cwd.into(),
            model_provider: ModelProvider::Mock,
            model_name: None,
            permission_mode: crate::tool_registry::PermissionMode::WorkspaceWrite,
        }
    }

    pub fn with_provider(mut self, provider: ModelProvider, model: Option<String>) -> Self {
        self.model_provider = provider;
        self.model_name = model;
        self
    }

    pub fn with_permission_mode(mut self, mode: crate::tool_registry::PermissionMode) -> Self {
        self.permission_mode = mode;
        self
    }

    pub fn cwd_path(&self) -> PathBuf {
        PathBuf::from(&self.cwd)
    }

    pub fn resolve_path(&self, file_path: &str) -> Result<PathBuf, String> {
        let cwd = fs::canonicalize(self.cwd_path())
            .map_err(|error| format!("failed to resolve cwd {}: {error}", self.cwd))?;
        let candidate = if Path::new(file_path).is_absolute() {
            PathBuf::from(file_path)
        } else {
            cwd.join(file_path)
        };
        let parent = candidate
            .parent()
            .ok_or_else(|| format!("path has no parent: {file_path}"))?;
        let canonical_parent = fs::canonicalize(parent)
            .map_err(|error| format!("failed to resolve parent for {file_path}: {error}"))?;
        let Some(file_name) = candidate.file_name() else {
            return Err(format!("path has no file name: {file_path}"));
        };
        let normalized = canonical_parent.join(file_name);
        if !normalized.starts_with(&cwd) {
            return Err(format!("path escapes cwd boundary: {file_path}"));
        }
        Ok(normalized)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCommandOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub trait ToolHost {
    fn read_to_string(&self, path: &Path) -> Result<String, String>;
    fn write_string(&self, path: &Path, content: &str) -> Result<(), String>;
    fn run_command(&self, cwd: &Path, command: &str) -> Result<ToolCommandOutput, String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LiveToolHost;

impl ToolHost for LiveToolHost {
    fn read_to_string(&self, path: &Path) -> Result<String, String> {
        fs::read_to_string(path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))
    }

    fn write_string(&self, path: &Path, content: &str) -> Result<(), String> {
        fs::write(path, content)
            .map_err(|error| format!("failed to write {}: {error}", path.display()))
    }

    fn run_command(&self, cwd: &Path, command: &str) -> Result<ToolCommandOutput, String> {
        let output = Command::new("bash")
            .arg("-lc")
            .arg(command)
            .current_dir(cwd)
            .output()
            .map_err(|error| format!("failed to run bash command: {error}"))?;
        Ok(ToolCommandOutput {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultToolExecutor<H = LiveToolHost> {
    pub context: ToolExecutionContext,
    pub host: H,
}

impl DefaultToolExecutor<LiveToolHost> {
    pub fn new(cwd: impl Into<String>) -> Self {
        Self {
            context: ToolExecutionContext::new(cwd),
            host: LiveToolHost,
        }
    }

    pub fn with_provider(mut self, provider: ModelProvider, model: Option<String>) -> Self {
        self.context = self.context.with_provider(provider, model);
        self
    }
}

impl<H> DefaultToolExecutor<H> {
    pub fn with_host(context: ToolExecutionContext, host: H) -> Self {
        Self { context, host }
    }
}

pub trait ToolExecutor {
    fn execute(&self, request: ToolExecutionRequest) -> ToolExecutionTrace;
}

impl<H: ToolHost> DefaultToolExecutor<H> {
    fn missing_argument(call: ToolCallInput, key: &str) -> ToolExecutionTrace {
        ToolExecutionTrace {
            progress_updates: Vec::new(),
            result: ToolCallResult::failed(call, format!("missing required argument: {key}")),
            permission_denial: None,
        }
    }

    fn preview(content: &str, max_chars: usize) -> String {
        let preview = content.chars().take(max_chars).collect::<String>();
        if content.chars().count() > max_chars {
            format!("{preview}...")
        } else {
            preview
        }
    }

    fn execute_read(&self, call: ToolCallInput) -> ToolExecutionTrace {
        let Some(file_path) = call.argument("file_path") else {
            return Self::missing_argument(call, "file_path");
        };
        let file_path = file_path.to_string();
        let progress =
            ToolProgressUpdate::new(call.tool_use_id.clone(), format!("reading {file_path}"));
        let resolved = match self.context.resolve_path(&file_path) {
            Ok(path) => path,
            Err(error) => {
                return ToolExecutionTrace {
                    progress_updates: vec![progress],
                    result: ToolCallResult::failed(call, error),
                    permission_denial: None,
                };
            }
        };
        // File safety: symlink escape, size limit, binary detection.
        let resolved = match validate_read_target(&resolved, &self.context.cwd_path()) {
            Ok(path) => path,
            Err(error) => {
                return ToolExecutionTrace {
                    progress_updates: vec![progress],
                    result: ToolCallResult::failed(call, error),
                    permission_denial: None,
                };
            }
        };
        match self.host.read_to_string(&resolved) {
            Ok(content) => {
                // Return full content to model, truncated at 100K chars for safety.
                let display_content = if content.len() > 100_000 {
                    format!(
                        "{}\n\n[truncated: file is {} bytes, showing first 100000 chars]",
                        &content[..100_000],
                        content.len()
                    )
                } else {
                    content.clone()
                };
                ToolExecutionTrace {
                    progress_updates: vec![progress],
                    result: ToolPermissionDecision::allow(false).settle(
                        call.clone(),
                        ToolCallOutput {
                            summary: format!("read {} bytes from {}", content.len(), file_path),
                            generated_messages: vec![QueryMessage::assistant(format!(
                                "tool-message: read {file_path} ({} bytes)\n{display_content}",
                                content.len()
                            ))],
                            context_label: Some(call.context_label.clone()),
                            progress_updates: vec![ToolProgressUpdate::new(
                                call.tool_use_id,
                                format!("read complete: {file_path}"),
                            )],
                        },
                    ),
                    permission_denial: None,
                }
            }
            Err(error) => ToolExecutionTrace {
                progress_updates: vec![progress],
                result: ToolCallResult::failed(call, error),
                permission_denial: None,
            },
        }
    }

    fn execute_edit(&self, call: ToolCallInput) -> ToolExecutionTrace {
        let Some(file_path) = call.argument("file_path") else {
            return Self::missing_argument(call, "file_path");
        };
        let Some(old_string) = call.argument("old_string") else {
            return Self::missing_argument(call, "old_string");
        };
        let Some(new_string) = call.argument("new_string") else {
            return Self::missing_argument(call, "new_string");
        };
        let file_path = file_path.to_string();
        let old_string = old_string.to_string();
        let new_string = new_string.to_string();
        let progress =
            ToolProgressUpdate::new(call.tool_use_id.clone(), format!("editing {file_path}"));
        let resolved = match self.context.resolve_path(&file_path) {
            Ok(path) => path,
            Err(error) => {
                return ToolExecutionTrace {
                    progress_updates: vec![progress],
                    result: ToolCallResult::failed(call, error),
                    permission_denial: None,
                };
            }
        };
        // File safety: symlink escape check for edit target.
        let resolved = match validate_write_target(&resolved, &self.context.cwd_path()) {
            Ok(path) => path,
            Err(error) => {
                return ToolExecutionTrace {
                    progress_updates: vec![progress],
                    result: ToolCallResult::failed(call, error),
                    permission_denial: None,
                };
            }
        };
        let original = match self.host.read_to_string(&resolved) {
            Ok(content) => content,
            Err(error) => {
                return ToolExecutionTrace {
                    progress_updates: vec![progress],
                    result: ToolCallResult::failed(call, error),
                    permission_denial: None,
                };
            }
        };
        if !original.contains(&old_string) {
            return ToolExecutionTrace {
                progress_updates: vec![progress],
                result: ToolCallResult::failed(
                    call,
                    format!("target text not found in {file_path}"),
                ),
                permission_denial: None,
            };
        }
        let updated = original.replacen(&old_string, &new_string, 1);
        if let Err(error) = self.host.write_string(&resolved, &updated) {
            return ToolExecutionTrace {
                progress_updates: vec![progress],
                result: ToolCallResult::failed(call, error),
                permission_denial: None,
            };
        }
        // Build a simple unified diff preview for TUI display
        let diff_preview = build_edit_diff_preview(&old_string, &new_string, 3);
        ToolExecutionTrace {
            progress_updates: vec![progress],
            result: ToolPermissionDecision::allow(false).settle(
                call.clone(),
                ToolCallOutput {
                    summary: format!("edited {file_path} with 1 replacement"),
                    generated_messages: vec![QueryMessage::assistant(format!(
                        "tool-message: edited {file_path}\n{diff_preview}"
                    ))],
                    context_label: Some(call.context_label.clone()),
                    progress_updates: vec![ToolProgressUpdate::new(
                        call.tool_use_id,
                        format!("edit complete: {file_path}"),
                    )],
                },
            ),
            permission_denial: None,
        }
    }

    fn execute_bash(&self, call: ToolCallInput) -> ToolExecutionTrace {
        let Some(command) = call.argument("command") else {
            return Self::missing_argument(call, "command");
        };
        let command = command.to_string();

        // Bash sandbox: block dangerous commands.
        let validation = validate_bash_command(
            &command,
            crate::tool_registry::PermissionMode::WorkspaceWrite,
            &self.context.cwd,
        );
        if !validation.allowed {
            let reason = validation
                .denial_reason
                .unwrap_or_else(|| "command blocked".to_string());
            return ToolExecutionTrace {
                progress_updates: Vec::new(),
                result: ToolCallResult::failed(call, format!("command blocked: {reason}")),
                permission_denial: Some(reason),
            };
        }

        // Sandbox: check command paths against workspace boundary.
        let sandbox_request = SandboxRequest {
            enabled: true,
            filesystem_mode: FilesystemIsolationMode::WorkspaceOnly,
            network_isolation: false,
            allowed_mounts: vec![self.context.cwd.clone()],
        };
        let sandbox_status = resolve_sandbox_status(&sandbox_request);

        let mut progress_updates = vec![ToolProgressUpdate::new(
            call.tool_use_id.clone(),
            format!(
                "sandbox: active={} fs={:?}",
                sandbox_status.active, sandbox_status.filesystem_mode
            ),
        )];

        if sandbox_status.active
            && sandbox_status.filesystem_mode == FilesystemIsolationMode::WorkspaceOnly
            && let Some(reason) = check_command_paths(&command, &self.context.cwd)
        {
            return ToolExecutionTrace {
                progress_updates,
                result: ToolCallResult::failed(call, format!("sandbox blocked: {reason}")),
                permission_denial: Some(reason),
            };
        }

        progress_updates.push(ToolProgressUpdate::new(
            call.tool_use_id.clone(),
            format!("running bash: {command}"),
        ));
        match self.host.run_command(&self.context.cwd_path(), &command) {
            Ok(output) if output.exit_code == 0 => {
                let combined = if output.stderr.trim().is_empty() {
                    output.stdout.trim().to_string()
                } else if output.stdout.trim().is_empty() {
                    output.stderr.trim().to_string()
                } else {
                    format!(
                        "stdout={} stderr={}",
                        output.stdout.trim(),
                        output.stderr.trim()
                    )
                };
                ToolExecutionTrace {
                    progress_updates,
                    result: ToolPermissionDecision::allow(false).settle(
                        call.clone(),
                        ToolCallOutput {
                            summary: format!("bash exited {}", output.exit_code),
                            generated_messages: vec![QueryMessage::assistant(format!(
                                "tool-message: bash {command} (exit {})\n{combined}",
                                output.exit_code
                            ))],
                            context_label: Some(call.context_label.clone()),
                            progress_updates: vec![ToolProgressUpdate::new(
                                call.tool_use_id,
                                format!("bash complete: {}", output.exit_code),
                            )],
                        },
                    ),
                    permission_denial: None,
                }
            }
            Ok(output) => ToolExecutionTrace {
                progress_updates,
                result: ToolCallResult::failed(
                    call,
                    format!(
                        "bash exited {}: {}",
                        output.exit_code,
                        Self::preview(output.stderr.trim(), 160)
                    ),
                ),
                permission_denial: None,
            },
            Err(error) => ToolExecutionTrace {
                progress_updates,
                result: ToolCallResult::failed(call, error),
                permission_denial: None,
            },
        }
    }

    fn execute_write(&self, call: ToolCallInput) -> ToolExecutionTrace {
        let Some(file_path) = call.argument("file_path") else {
            return Self::missing_argument(call, "file_path");
        };
        let Some(content) = call.argument("content") else {
            return Self::missing_argument(call, "content");
        };
        let file_path = file_path.to_string();
        let content = content.to_string();
        let progress =
            ToolProgressUpdate::new(call.tool_use_id.clone(), format!("writing {file_path}"));

        // Resolve path — for new files, parent must exist.
        let cwd = match fs::canonicalize(self.context.cwd_path()) {
            Ok(p) => p,
            Err(error) => {
                return ToolExecutionTrace {
                    progress_updates: vec![progress],
                    result: ToolCallResult::failed(call, format!("failed to resolve cwd: {error}")),
                    permission_denial: None,
                };
            }
        };
        let candidate = if Path::new(&file_path).is_absolute() {
            PathBuf::from(&file_path)
        } else {
            cwd.join(&file_path)
        };
        // Ensure parent directory exists.
        if let Some(parent) = candidate.parent()
            && !parent.exists()
            && let Err(error) = fs::create_dir_all(parent)
        {
            return ToolExecutionTrace {
                progress_updates: vec![progress],
                result: ToolCallResult::failed(
                    call,
                    format!("failed to create parent dirs: {error}"),
                ),
                permission_denial: None,
            };
        }

        // File safety: symlink escape check for write target.
        let candidate = match validate_write_target(&candidate, &self.context.cwd_path()) {
            Ok(path) => path,
            Err(error) => {
                return ToolExecutionTrace {
                    progress_updates: vec![progress],
                    result: ToolCallResult::failed(call, error),
                    permission_denial: None,
                };
            }
        };

        match self.host.write_string(&candidate, &content) {
            Ok(()) => ToolExecutionTrace {
                progress_updates: vec![progress],
                result: ToolPermissionDecision::allow(false).settle(
                    call.clone(),
                    ToolCallOutput {
                        summary: format!("wrote {} bytes to {}", content.len(), file_path),
                        generated_messages: vec![QueryMessage::assistant(format!(
                            "tool-message: wrote {} bytes to {}",
                            content.len(),
                            file_path
                        ))],
                        context_label: Some(call.context_label.clone()),
                        progress_updates: vec![ToolProgressUpdate::new(
                            call.tool_use_id,
                            format!("write complete: {file_path}"),
                        )],
                    },
                ),
                permission_denial: None,
            },
            Err(error) => ToolExecutionTrace {
                progress_updates: vec![progress],
                result: ToolCallResult::failed(call, error),
                permission_denial: None,
            },
        }
    }

    fn execute_glob(&self, call: ToolCallInput) -> ToolExecutionTrace {
        let Some(pattern) = call.argument("pattern") else {
            return Self::missing_argument(call, "pattern");
        };
        let pattern = pattern.to_string();
        let base_dir = call
            .argument("path")
            .map(ToString::to_string)
            .unwrap_or_else(|| self.context.cwd.clone());
        let progress =
            ToolProgressUpdate::new(call.tool_use_id.clone(), format!("globbing {pattern}"));

        // Use bash find + pattern matching as a portable glob.
        let glob_command = format!(
            "find {} -path '{}' -type f 2>/dev/null | head -200",
            shell_escape(&base_dir),
            shell_escape(&pattern)
        );
        match self
            .host
            .run_command(&self.context.cwd_path(), &glob_command)
        {
            Ok(output) => {
                let files: Vec<&str> = output
                    .stdout
                    .lines()
                    .filter(|line| !line.is_empty())
                    .collect();
                let count = files.len();
                let listing = if files.is_empty() {
                    String::from("no matches found")
                } else {
                    files.join("\n")
                };
                ToolExecutionTrace {
                    progress_updates: vec![progress],
                    result: ToolPermissionDecision::allow(false).settle(
                        call.clone(),
                        ToolCallOutput {
                            summary: format!("glob found {count} files matching {pattern}"),
                            generated_messages: vec![QueryMessage::assistant(format!(
                                "tool-message: glob {pattern} -> {count} matches\n{listing}"
                            ))],
                            context_label: Some(call.context_label.clone()),
                            progress_updates: vec![ToolProgressUpdate::new(
                                call.tool_use_id,
                                format!("glob complete: {count} matches"),
                            )],
                        },
                    ),
                    permission_denial: None,
                }
            }
            Err(error) => ToolExecutionTrace {
                progress_updates: vec![progress],
                result: ToolCallResult::failed(call, error),
                permission_denial: None,
            },
        }
    }

    fn execute_grep(&self, call: ToolCallInput) -> ToolExecutionTrace {
        let Some(pattern) = call.argument("pattern") else {
            return Self::missing_argument(call, "pattern");
        };
        let pattern = pattern.to_string();
        let search_path = call
            .argument("path")
            .map(ToString::to_string)
            .unwrap_or_else(|| self.context.cwd.clone());
        let progress =
            ToolProgressUpdate::new(call.tool_use_id.clone(), format!("grepping {pattern}"));

        let mut grep_cmd = format!(
            "grep -rn --include='*' {} {} 2>/dev/null | head -200",
            shell_escape(&pattern),
            shell_escape(&search_path)
        );
        // If include glob is specified, use it.
        if let Some(glob) = call.argument("glob") {
            grep_cmd = format!(
                "grep -rn --include={} {} {} 2>/dev/null | head -200",
                shell_escape(glob),
                shell_escape(&pattern),
                shell_escape(&search_path)
            );
        }

        match self.host.run_command(&self.context.cwd_path(), &grep_cmd) {
            Ok(output) => {
                let lines: Vec<&str> = output
                    .stdout
                    .lines()
                    .filter(|line| !line.is_empty())
                    .collect();
                let count = lines.len();
                let listing = if lines.is_empty() {
                    String::from("no matches found")
                } else {
                    lines.join("\n")
                };
                ToolExecutionTrace {
                    progress_updates: vec![progress],
                    result: ToolPermissionDecision::allow(false).settle(
                        call.clone(),
                        ToolCallOutput {
                            summary: format!("grep found {count} matches for {pattern}"),
                            generated_messages: vec![QueryMessage::assistant(format!(
                                "tool-message: grep {pattern} -> {count} matches\n{listing}"
                            ))],
                            context_label: Some(call.context_label.clone()),
                            progress_updates: vec![ToolProgressUpdate::new(
                                call.tool_use_id,
                                format!("grep complete: {count} matches"),
                            )],
                        },
                    ),
                    permission_denial: None,
                }
            }
            Err(error) => ToolExecutionTrace {
                progress_updates: vec![progress],
                result: ToolCallResult::failed(call, error),
                permission_denial: None,
            },
        }
    }

    fn execute_agent(&self, call: ToolCallInput) -> ToolExecutionTrace {
        let Some(agent_id) = call.argument("agent_id") else {
            return Self::missing_argument(call, "agent_id");
        };
        let Some(prompt) = call.argument("prompt") else {
            return Self::missing_argument(call, "prompt");
        };

        let agent_id = agent_id.to_string();
        let prompt = prompt.to_string();
        let progress = ToolProgressUpdate::new(
            call.tool_use_id.clone(),
            format!("spawning agent: {agent_id}"),
        );

        let agent_config = QueryEngineConfig {
            cwd: self.context.cwd.clone(),
            session_id: format!("agent-{agent_id}"),
            persist_session: false,
            persist_history: false,
            file_history_enabled: false,
            tools: vec![
                String::from("Read"),
                String::from("Bash"),
                String::from("Glob"),
                String::from("Grep"),
            ],
            tool_runtime_mode: ToolRuntimeMode::Standard,
            tool_permission_context: Default::default(),
            commands: Vec::new(),
            mcp_clients: Vec::new(),
            agents: Vec::new(),
            initial_messages: Vec::new(),
            read_file_cache_entries: 0,
            custom_system_prompt: None,
            append_system_prompt: None,
            model_provider: self.context.model_provider,
            user_specified_model: self.context.model_name.clone(),
            fallback_model: self.context.model_name.clone(),
            model_reasoning_effort: None,
            thinking_mode: ThinkingMode::Disabled,
            max_turns: Some(2),
            max_budget_usd: None,
            task_budget: None,
            json_schema: None,
            verbose: false,
            replay_user_messages: false,
            include_partial_messages: false,
            stream_model_responses: false,
        };

        // Register worker in global registry
        let worker_id = format!("agent-{agent_id}");
        {
            let registry = global_worker_registry();
            let mut guard = registry.lock().expect("worker registry lock");
            guard.create(&worker_id);
        }

        // Spawn agent on background thread for parallel execution
        let worker_id_clone = worker_id.clone();
        std::thread::spawn(move || {
            // Transition to Running
            {
                let registry = global_worker_registry();
                let mut guard = registry.lock().expect("worker registry lock");
                if let Some(w) = guard.get_mut(&worker_id_clone) {
                    w.emit_event(WorkerEventKind::Running);
                }
            }

            let mut host = InProcessAgentHost::new(agent_config);
            let task_id = TaskId::new("a", 1);
            let _result = host.run_agent(&task_id, &worker_id_clone, &prompt);

            // Mark worker as finished
            let registry = global_worker_registry();
            let mut guard = registry.lock().expect("worker registry lock");
            if let Some(w) = guard.get_mut(&worker_id_clone) {
                w.emit_event(WorkerEventKind::Finished);
            }
        });

        // Return immediately with spawn confirmation
        let summary = format!("agent {agent_id} spawned (worker: {worker_id})");
        ToolExecutionTrace {
            progress_updates: vec![progress],
            result: ToolPermissionDecision::allow(false).settle(
                call.clone(),
                ToolCallOutput {
                    summary,
                    generated_messages: vec![QueryMessage::assistant(format!(
                        "agent {agent_id} spawned in background"
                    ))],
                    context_label: Some(call.context_label.clone()),
                    progress_updates: vec![ToolProgressUpdate::new(
                        call.tool_use_id,
                        format!("agent {agent_id} running"),
                    )],
                },
            ),
            permission_denial: None,
        }
    }

    fn execute_web_fetch(&self, call: ToolCallInput) -> ToolExecutionTrace {
        let Some(url) = call.argument("url") else {
            return Self::missing_argument(call, "url");
        };
        let url = url.to_string();
        let progress = ToolProgressUpdate::new(call.tool_use_id.clone(), format!("fetching {url}"));

        let curl_cmd = format!(
            "curl -sL -m 30 --max-filesize 1048576 {}",
            shell_escape(&url)
        );
        match self.host.run_command(&self.context.cwd_path(), &curl_cmd) {
            Ok(output) if output.exit_code == 0 => {
                let body = output.stdout;
                let len = body.len();
                let preview = Self::preview(&body, 2000);
                ToolExecutionTrace {
                    progress_updates: vec![progress],
                    result: ToolPermissionDecision::allow(false).settle(
                        call.clone(),
                        ToolCallOutput {
                            summary: format!("fetched {len} bytes from {url}"),
                            generated_messages: vec![QueryMessage::assistant(format!(
                                "tool-message: fetched {url} ({len} bytes)\n{preview}"
                            ))],
                            context_label: Some(call.context_label.clone()),
                            progress_updates: vec![ToolProgressUpdate::new(
                                call.tool_use_id,
                                format!("fetch complete: {url}"),
                            )],
                        },
                    ),
                    permission_denial: None,
                }
            }
            Ok(output) => ToolExecutionTrace {
                progress_updates: vec![progress],
                result: ToolCallResult::failed(
                    call,
                    format!(
                        "curl exited {}: {}",
                        output.exit_code,
                        Self::preview(&output.stderr, 160)
                    ),
                ),
                permission_denial: None,
            },
            Err(error) => ToolExecutionTrace {
                progress_updates: vec![progress],
                result: ToolCallResult::failed(call, error),
                permission_denial: None,
            },
        }
    }

    fn execute_web_search(&self, call: ToolCallInput) -> ToolExecutionTrace {
        let Some(query) = call.argument("query") else {
            return Self::missing_argument(call, "query");
        };
        let query = query.to_string();
        let progress =
            ToolProgressUpdate::new(call.tool_use_id.clone(), format!("searching: {query}"));

        // Use DuckDuckGo lite as a zero-dependency web search.
        let encoded = query.replace(' ', "+");
        let search_cmd = format!(
            "curl -sL -m 15 'https://lite.duckduckgo.com/lite/?q={}' | grep -oP '(?<=<a rel=\"nofollow\" href=\")[^\"]+' | head -10",
            shell_escape(&encoded)
        );
        match self.host.run_command(&self.context.cwd_path(), &search_cmd) {
            Ok(output) => {
                let results = output.stdout.trim().to_string();
                let count = results.lines().count();
                let listing = if results.is_empty() {
                    String::from("no results found")
                } else {
                    results
                };
                ToolExecutionTrace {
                    progress_updates: vec![progress],
                    result: ToolPermissionDecision::allow(false).settle(
                        call.clone(),
                        ToolCallOutput {
                            summary: format!("web search found {count} results for: {query}"),
                            generated_messages: vec![QueryMessage::assistant(format!(
                                "tool-message: search '{query}' -> {count} results\n{listing}"
                            ))],
                            context_label: Some(call.context_label.clone()),
                            progress_updates: vec![ToolProgressUpdate::new(
                                call.tool_use_id,
                                format!("search complete: {count} results"),
                            )],
                        },
                    ),
                    permission_denial: None,
                }
            }
            Err(error) => ToolExecutionTrace {
                progress_updates: vec![progress],
                result: ToolCallResult::failed(call, error),
                permission_denial: None,
            },
        }
    }

    fn execute_mcp_tool(&self, call: ToolCallInput) -> ToolExecutionTrace {
        let tool_name = call.tool_name.as_str();
        let progress = ToolProgressUpdate::new(
            call.tool_use_id.clone(),
            format!("executing MCP tool: {tool_name}"),
        );

        let _args: Vec<(String, String)> = call
            .arguments
            .iter()
            .map(|arg| (arg.key.clone(), arg.value.clone()))
            .collect();

        ToolExecutionTrace {
            progress_updates: vec![progress],
            result: ToolCallResult::failed(call, "MCP client not available in executor context"),
            permission_denial: None,
        }
    }
}

impl<H: ToolHost> ToolExecutor for DefaultToolExecutor<H> {
    fn execute(&self, request: ToolExecutionRequest) -> ToolExecutionTrace {
        if !request.can_execute {
            let reason = request
                .deny_reason
                .unwrap_or_else(|| String::from("tool execution denied"));
            return ToolExecutionTrace {
                progress_updates: Vec::new(),
                result: ToolPermissionDecision::deny(reason.clone())
                    .settle(request.call, ToolCallOutput::default()),
                permission_denial: Some(reason),
            };
        }

        // Validate tool input against JSON Schema before dispatch.
        let arg_pairs: Vec<(String, String)> = request
            .call
            .arguments
            .iter()
            .map(|a| (a.key.clone(), a.value.clone()))
            .collect();
        if let Err(error) =
            crate::tool_validation::validate_tool_input(&request.call.tool_name, &arg_pairs)
        {
            return ToolExecutionTrace {
                progress_updates: Vec::new(),
                result: ToolCallResult::failed(request.call, error),
                permission_denial: None,
            };
        }

        // Permission enforcement: verify tool is allowed under active mode.
        let active_mode = self.context.permission_mode;
        let perm_check = crate::permission_enforcer::check_tool_permission(
            request.call.tool_name.as_str(),
            active_mode,
        );
        if let crate::permission_enforcer::PermissionCheckResult::Denied { reason, .. } = perm_check
        {
            return ToolExecutionTrace {
                progress_updates: Vec::new(),
                result: ToolCallResult::failed(
                    request.call,
                    format!("permission denied: {reason}"),
                ),
                permission_denial: Some(reason),
            };
        }

        match request.call.tool_name.as_str() {
            "Read" => self.execute_read(request.call),
            "Edit" => self.execute_edit(request.call),
            "Bash" => self.execute_bash(request.call),
            "Write" => self.execute_write(request.call),
            "Glob" => self.execute_glob(request.call),
            "Grep" => self.execute_grep(request.call),
            "WebFetch" => self.execute_web_fetch(request.call),
            "WebSearch" => self.execute_web_search(request.call),
            "Agent" => self.execute_agent(request.call),
            "CronCreate" => super::cron_tools::execute_cron_create(request.call),
            "CronDelete" => super::cron_tools::execute_cron_delete(request.call),
            "CronList" => super::cron_tools::execute_cron_list(request.call),
            "TeamCreate" => super::team_tools::execute_team_create(request.call),
            "TeamDelete" => super::team_tools::execute_team_delete(request.call),
            "TaskCreate" => super::task_tools::execute_task_create(request.call),
            "TaskGet" => super::task_tools::execute_task_get(request.call),
            "TaskList" => super::task_tools::execute_task_list(request.call),
            "TaskUpdate" => super::task_tools::execute_task_update(request.call),
            "TaskStop" => super::task_tools::execute_task_stop(request.call),
            "TaskOutput" => super::task_tools::execute_task_output(request.call),
            "ToolSearch" => super::tool_search::execute_tool_search(request.call),
            "Lsp" => super::lsp_tools::execute_lsp(request.call),
            "MemorySave" => super::memory_tools::execute_memory_save(request.call),
            "MemoryList" => super::memory_tools::execute_memory_list(request.call),
            "MemorySearch" => super::memory_tools::execute_memory_search(request.call),
            "MemoryDelete" => super::memory_tools::execute_memory_delete(request.call),
            tool_name if tool_name.starts_with("mcp:") => self.execute_mcp_tool(request.call),
            _ => ToolExecutionTrace {
                progress_updates: Vec::new(),
                result: ToolCallResult::failed(
                    request.call,
                    "no default executor behavior for requested tool",
                ),
                permission_denial: None,
            },
        }
    }
}

/// Escape a string for safe use in a shell command.
fn shell_escape(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Check whether any absolute path tokens in a command fall outside the workspace.
fn check_command_paths(command: &str, cwd: &str) -> Option<String> {
    for token in command.split_whitespace() {
        if token.starts_with('/') && !token.starts_with(cwd) && !is_safe_system_path(token) {
            return Some(format!("path {token} is outside workspace {cwd}"));
        }
    }
    None
}

/// Return `true` for system paths that are safe to reference from sandboxed commands.
fn is_safe_system_path(path: &str) -> bool {
    path.starts_with("/dev/null")
        || path.starts_with("/tmp")
        || path.starts_with("/usr/bin")
        || path.starts_with("/usr/local/bin")
        || path.starts_with("/bin")
        || path.starts_with("/proc")
}

/// Build a simple +/- diff preview from old and new strings.
/// Shows up to `max_context` lines of context around changes.
fn build_edit_diff_preview(old: &str, new: &str, max_lines: usize) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let mut diff = Vec::new();

    // Show removed lines (prefixed with -)
    for (i, line) in old_lines.iter().enumerate() {
        if i >= max_lines {
            let remaining = old_lines.len() - max_lines;
            diff.push(format!("- ... {remaining} more removed"));
            break;
        }
        diff.push(format!("- {line}"));
    }
    // Show added lines (prefixed with +)
    for (i, line) in new_lines.iter().enumerate() {
        if i >= max_lines {
            let remaining = new_lines.len() - max_lines;
            diff.push(format!("+ ... {remaining} more added"));
            break;
        }
        diff.push(format!("+ {line}"));
    }
    diff.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct MemoryToolHost {
        files: Arc<Mutex<HashMap<String, String>>>,
        commands: Arc<Mutex<HashMap<String, ToolCommandOutput>>>,
    }

    impl MemoryToolHost {
        fn with_file(self, path: impl Into<String>, content: impl Into<String>) -> Self {
            self.files
                .lock()
                .expect("lock poisoned")
                .insert(path.into(), content.into());
            self
        }

        fn with_command(self, command: impl Into<String>, output: ToolCommandOutput) -> Self {
            self.commands
                .lock()
                .expect("lock poisoned")
                .insert(command.into(), output);
            self
        }

        fn file_content(&self, path: &str) -> Option<String> {
            self.files.lock().expect("lock poisoned").get(path).cloned()
        }
    }

    impl ToolHost for MemoryToolHost {
        fn read_to_string(&self, path: &Path) -> Result<String, String> {
            self.file_content(path.to_string_lossy().as_ref())
                .ok_or_else(|| format!("missing file {}", path.display()))
        }

        fn write_string(&self, path: &Path, content: &str) -> Result<(), String> {
            self.files
                .lock()
                .expect("lock poisoned")
                .insert(path.to_string_lossy().into_owned(), content.to_string());
            Ok(())
        }

        fn run_command(&self, _cwd: &Path, command: &str) -> Result<ToolCommandOutput, String> {
            self.commands
                .lock()
                .expect("lock poisoned")
                .get(command)
                .cloned()
                .ok_or_else(|| format!("missing command {command}"))
        }
    }

    fn sample_context() -> ToolExecutionContext {
        let cwd = std::env::temp_dir().join("nocode-tool-execution-tests");
        std::fs::create_dir_all(cwd.join("src")).expect("test cwd should exist");
        ToolExecutionContext::new(cwd.to_string_lossy().into_owned())
    }

    #[test]
    fn default_executor_reads_file_through_host() {
        let context = sample_context();
        let host = MemoryToolHost::default().with_file(
            context.cwd_path().join("src/query.ts").to_string_lossy(),
            "export const query = 'seed';",
        );
        let executor = DefaultToolExecutor::with_host(context, host);
        let trace = executor.execute(ToolExecutionRequest::allowed(
            ToolCallInput::new("Read", "toolu-4")
                .with_argument("file_path", "src/query.ts")
                .with_context_label("sdk-bootstrap"),
        ));

        assert_eq!(trace.progress_updates.len(), 1);
        assert_eq!(trace.result.status_label(), "completed");
        assert!(trace.result.message().contains("read 28 bytes"));
    }

    #[test]
    fn default_executor_edits_file_through_host() {
        let context = sample_context();
        let file_path = context.cwd_path().join("file.txt");
        let host = MemoryToolHost::default().with_file(file_path.to_string_lossy(), "alpha beta");
        let snapshot = host.clone();
        let executor = DefaultToolExecutor::with_host(context, host);
        let trace = executor.execute(ToolExecutionRequest::allowed(
            ToolCallInput::new("Edit", "toolu-5")
                .with_argument("file_path", "file.txt")
                .with_argument("old_string", "beta")
                .with_argument("new_string", "gamma")
                .with_context_label("sdk-bootstrap"),
        ));

        assert_eq!(trace.result.status_label(), "completed");
        assert_eq!(
            snapshot
                .file_content(file_path.to_string_lossy().as_ref())
                .as_deref(),
            Some("alpha gamma")
        );
    }

    #[test]
    fn default_executor_runs_bash_through_host() {
        let host = MemoryToolHost::default().with_command(
            "printf hello",
            ToolCommandOutput {
                exit_code: 0,
                stdout: String::from("hello"),
                stderr: String::new(),
            },
        );
        let executor = DefaultToolExecutor::with_host(sample_context(), host);
        let trace = executor.execute(ToolExecutionRequest::allowed(
            ToolCallInput::new("Bash", "toolu-6")
                .with_argument("command", "printf hello")
                .with_context_label("sdk-bootstrap"),
        ));

        assert_eq!(trace.result.status_label(), "completed");
        assert!(trace.result.message().contains("bash exited 0"));
    }

    #[test]
    fn default_executor_returns_denied_trace() {
        let executor = DefaultToolExecutor::with_host(sample_context(), MemoryToolHost::default());
        let trace = executor.execute(ToolExecutionRequest::denied(
            ToolCallInput::new("Read", "toolu-7"),
            "tool denied by permission context",
        ));

        assert!(trace.progress_updates.is_empty());
        assert_eq!(trace.result.status_label(), "denied");
        assert_eq!(
            trace.permission_denial.as_deref(),
            Some("tool denied by permission context")
        );
    }

    #[test]
    fn bash_sandbox_blocks_rm_rf_root() {
        use crate::bash_validation::validate_bash_command;
        use crate::tool_registry::PermissionMode;
        assert!(!validate_bash_command("rm -rf /", PermissionMode::WorkspaceWrite, "/tmp").allowed);
        assert!(
            !validate_bash_command("rm -rf /*", PermissionMode::WorkspaceWrite, "/tmp").allowed
        );
        assert!(!validate_bash_command("rm -rf ~", PermissionMode::WorkspaceWrite, "/tmp").allowed);
    }

    #[test]
    fn bash_sandbox_blocks_dangerous_commands() {
        use crate::bash_validation::validate_bash_command;
        use crate::tool_registry::PermissionMode;
        let m = PermissionMode::WorkspaceWrite;
        assert!(!validate_bash_command("mkfs.ext4 /dev/sda1", m, "/tmp").allowed);
        assert!(!validate_bash_command("dd if=/dev/zero of=/dev/sda", m, "/tmp").allowed);
        assert!(!validate_bash_command("shutdown -h now", m, "/tmp").allowed);
        assert!(!validate_bash_command("reboot", m, "/tmp").allowed);
        assert!(!validate_bash_command("halt", m, "/tmp").allowed);
        assert!(!validate_bash_command("poweroff", m, "/tmp").allowed);
    }

    #[test]
    fn bash_sandbox_blocks_db_destructive() {
        use crate::bash_validation::validate_bash_command;
        use crate::tool_registry::PermissionMode;
        let m = PermissionMode::WorkspaceWrite;
        assert!(!validate_bash_command("psql -c 'DROP DATABASE prod'", m, "/tmp").allowed);
        assert!(!validate_bash_command("mysql -e 'TRUNCATE TABLE users'", m, "/tmp").allowed);
    }

    #[test]
    fn bash_sandbox_blocks_system_path_modification() {
        use crate::bash_validation::validate_bash_command;
        use crate::tool_registry::PermissionMode;
        let m = PermissionMode::WorkspaceWrite;
        assert!(!validate_bash_command("rm /etc/passwd", m, "/tmp").allowed);
        assert!(!validate_bash_command("mv /boot/vmlinuz /tmp/", m, "/tmp").allowed);
    }

    #[test]
    fn bash_sandbox_allows_safe_commands() {
        use crate::bash_validation::validate_bash_command;
        use crate::tool_registry::PermissionMode;
        let m = PermissionMode::WorkspaceWrite;
        let cwd = "/home/user/project";
        assert!(validate_bash_command("ls -la", m, cwd).allowed);
        assert!(validate_bash_command("cat README.md", m, cwd).allowed);
        assert!(validate_bash_command("cargo test", m, cwd).allowed);
        assert!(validate_bash_command("git status", m, cwd).allowed);
        assert!(validate_bash_command("grep -rn pattern .", m, cwd).allowed);
        assert!(validate_bash_command("rm src/temp.txt", m, cwd).allowed);
    }

    // -----------------------------------------------------------------------
    // PermissionPrompter tests
    // -----------------------------------------------------------------------

    #[test]
    fn auto_approve_prompter_allows_all() {
        let p = AutoApprovePrompter;
        assert_eq!(p.name(), "auto-approve");
        let d = p.check("Bash", "rm -rf /tmp/test");
        assert!(matches!(d, ToolPermissionDecision::Allow { .. }));
    }

    #[test]
    fn auto_deny_prompter_denies_all() {
        let p = AutoDenyPrompter::new();
        assert_eq!(p.name(), "auto-deny");
        let d = p.check("Read", "file_path=foo.rs");
        assert!(matches!(d, ToolPermissionDecision::Deny { .. }));
        if let ToolPermissionDecision::Deny { reason } = d {
            assert!(reason.contains("Read"));
        }
    }

    #[test]
    fn interactive_prompter_prompts_dangerous_tools() {
        let p = InteractivePrompter::default_dangerous();
        assert_eq!(p.name(), "interactive");

        let d = p.check("Bash", "command=ls");
        assert!(matches!(d, ToolPermissionDecision::Prompt { .. }));

        let d = p.check("Write", "file_path=foo.rs");
        assert!(matches!(d, ToolPermissionDecision::Prompt { .. }));

        let d = p.check("Edit", "old=a new=b");
        assert!(matches!(d, ToolPermissionDecision::Prompt { .. }));
    }

    #[test]
    fn interactive_prompter_allows_safe_tools() {
        let p = InteractivePrompter::default_dangerous();
        let d = p.check("Read", "file_path=foo.rs");
        assert!(matches!(d, ToolPermissionDecision::Allow { .. }));

        let d = p.check("Glob", "pattern=*.rs");
        assert!(matches!(d, ToolPermissionDecision::Allow { .. }));

        let d = p.check("Grep", "pattern=foo");
        assert!(matches!(d, ToolPermissionDecision::Allow { .. }));
    }

    #[test]
    fn interactive_prompter_custom_tools() {
        let p = InteractivePrompter::new(vec!["MyDangerous".into()]);
        let d = p.check("MyDangerous", "args");
        assert!(matches!(d, ToolPermissionDecision::Prompt { .. }));

        let d = p.check("MySafe", "args");
        assert!(matches!(d, ToolPermissionDecision::Allow { .. }));
    }

    #[test]
    fn permission_audit_log_records() {
        let mut log = PermissionAuditLog::default();
        assert!(log.is_empty());

        log.record("Bash", "allow", "auto-approve");
        log.record("Write", "prompt", "interactive");
        assert_eq!(log.len(), 2);
        assert!(!log.is_empty());

        assert_eq!(log.entries[0].tool_name, "Bash");
        assert_eq!(log.entries[0].decision, "allow");
        assert_eq!(log.entries[0].prompter, "auto-approve");
        assert!(log.entries[0].timestamp_ms > 0);

        assert_eq!(log.entries[1].tool_name, "Write");
    }

    #[test]
    fn permission_decision_prompt_settles_to_denied() {
        let call = ToolCallInput::new("Bash", "toolu-99");
        let decision = ToolPermissionDecision::prompt("Bash", "needs approval");
        let result = decision.settle(call, ToolCallOutput::default());
        assert_eq!(result.status_label(), "denied");
        assert!(result.message().contains("awaiting approval"));
    }

    #[test]
    fn auto_deny_prompter_default() {
        let p = AutoDenyPrompter::default();
        let d = p.check("Agent", "prompt=test");
        assert!(matches!(d, ToolPermissionDecision::Deny { .. }));
    }

    // -----------------------------------------------------------------------
    // Shell escape & path check tests
    // -----------------------------------------------------------------------

    #[test]
    fn shell_escape_handles_quotes() {
        assert_eq!(shell_escape("hello"), "'hello'");
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
        assert_eq!(shell_escape(""), "''");
    }

    #[test]
    fn check_command_paths_detects_outside_workspace() {
        let result = check_command_paths("cat /etc/passwd", "/home/user/project");
        assert!(result.is_some());
        assert!(result.unwrap().contains("/etc/passwd"));
    }

    #[test]
    fn check_command_paths_allows_workspace_paths() {
        let result =
            check_command_paths("cat /home/user/project/src/main.rs", "/home/user/project");
        assert!(result.is_none());
    }

    #[test]
    fn check_command_paths_allows_safe_system_paths() {
        assert!(check_command_paths("echo > /dev/null", "/home/user").is_none());
        assert!(check_command_paths("ls /tmp/test", "/home/user").is_none());
        assert!(check_command_paths("which /usr/bin/git", "/home/user").is_none());
        assert!(check_command_paths("cat /proc/cpuinfo", "/home/user").is_none());
    }

    #[test]
    fn check_command_paths_no_absolute_paths() {
        assert!(check_command_paths("ls -la src/", "/home/user").is_none());
        assert!(check_command_paths("cargo test", "/home/user").is_none());
    }

    #[test]
    fn is_safe_system_path_coverage() {
        assert!(is_safe_system_path("/dev/null"));
        assert!(is_safe_system_path("/tmp/anything"));
        assert!(is_safe_system_path("/usr/bin/git"));
        assert!(is_safe_system_path("/usr/local/bin/node"));
        assert!(is_safe_system_path("/bin/sh"));
        assert!(is_safe_system_path("/proc/self/status"));
        assert!(!is_safe_system_path("/etc/passwd"));
        assert!(!is_safe_system_path("/var/log/syslog"));
        assert!(!is_safe_system_path("/root/.ssh/id_rsa"));
    }
}
