use super::model::{
    ToolCallInput, ToolCallOutput, ToolCallResult, ToolExecutionRequest, ToolExecutionTrace,
    ToolPermissionDecision, ToolProgressUpdate,
};
use crate::message::QueryMessage;
use crate::provider::ModelProvider;
use crate::query_engine::QueryEngineConfig;
use crate::query_engine::ThinkingMode;
use crate::task_runtime::{InProcessAgentHost, TaskAgentHost, TaskId};
use crate::tool_registry::ToolRuntimeMode;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolExecutionContext {
    pub cwd: String,
    pub model_provider: ModelProvider,
    pub model_name: Option<String>,
}

impl ToolExecutionContext {
    pub fn new(cwd: impl Into<String>) -> Self {
        Self {
            cwd: cwd.into(),
            model_provider: ModelProvider::Mock,
            model_name: None,
        }
    }

    pub fn with_provider(mut self, provider: ModelProvider, model: Option<String>) -> Self {
        self.model_provider = provider;
        self.model_name = model;
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
        ToolExecutionTrace {
            progress_updates: vec![progress],
            result: ToolPermissionDecision::allow(false).settle(
                call.clone(),
                ToolCallOutput {
                    summary: format!("edited {file_path} with 1 replacement"),
                    generated_messages: vec![QueryMessage::assistant(format!(
                        "tool-message: edited {file_path}"
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
        if let Some(reason) = check_bash_safety(&command) {
            return ToolExecutionTrace {
                progress_updates: Vec::new(),
                result: ToolCallResult::failed(call, format!("command blocked: {reason}")),
                permission_denial: Some(reason),
            };
        }

        let progress =
            ToolProgressUpdate::new(call.tool_use_id.clone(), format!("running bash: {command}"));
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
                    progress_updates: vec![progress],
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
                progress_updates: vec![progress],
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
                progress_updates: vec![progress],
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

        let mut host = InProcessAgentHost::new(agent_config);
        let task_id = TaskId::new("a", 1);

        match host.run_agent(&task_id, &agent_id, &prompt) {
            Ok(step) => {
                let summary = format!(
                    "agent {agent_id} completed: {} tool uses, {} tokens",
                    step.tool_use_delta, step.token_delta
                );
                ToolExecutionTrace {
                    progress_updates: vec![progress],
                    result: ToolPermissionDecision::allow(false).settle(
                        call.clone(),
                        ToolCallOutput {
                            summary,
                            generated_messages: vec![QueryMessage::assistant(format!(
                                "agent {agent_id} result: retrieved={}",
                                step.retrieved
                            ))],
                            context_label: Some(call.context_label.clone()),
                            progress_updates: vec![ToolProgressUpdate::new(
                                call.tool_use_id,
                                "agent complete",
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

/// Check if a bash command is safe to execute. Returns `Some(reason)` if blocked.
fn check_bash_safety(command: &str) -> Option<String> {
    let normalized = command.to_ascii_lowercase();
    let trimmed = normalized.trim();

    // Block destructive filesystem commands.
    static DANGEROUS_PATTERNS: &[(&str, &str)] = &[
        ("rm -rf /", "recursive delete of root filesystem"),
        ("rm -rf /*", "recursive delete of root filesystem"),
        ("rm -rf ~", "recursive delete of home directory"),
        ("mkfs", "filesystem format command"),
        ("dd if=", "raw disk write command"),
        (":(){:|:&};:", "fork bomb"),
        ("chmod -r 777 /", "recursive permission change on root"),
        ("chown -r", "recursive ownership change"),
        ("> /dev/sda", "raw device write"),
        ("mv / ", "move root filesystem"),
    ];

    for (pattern, reason) in DANGEROUS_PATTERNS {
        if trimmed.contains(pattern) {
            return Some((*reason).to_string());
        }
    }

    // Block dangerous database commands without explicit backup context.
    static DB_DANGEROUS: &[(&str, &str)] = &[
        ("drop database", "database drop without backup"),
        ("drop table", "table drop without backup"),
        ("truncate table", "table truncate without backup"),
        ("delete from", "bulk delete without where clause check"),
    ];

    for (pattern, reason) in DB_DANGEROUS {
        if trimmed.contains(pattern) && !trimmed.contains("backup") && !trimmed.contains("--dry") {
            return Some((*reason).to_string());
        }
    }

    // Block commands that modify system-critical paths.
    if (trimmed.starts_with("rm ") || trimmed.starts_with("mv "))
        && (trimmed.contains("/etc/") || trimmed.contains("/boot/") || trimmed.contains("/usr/"))
    {
        return Some("modification of system-critical path".to_string());
    }

    // Block shutdown/reboot.
    if trimmed.starts_with("shutdown")
        || trimmed.starts_with("reboot")
        || trimmed.starts_with("init 0")
        || trimmed.starts_with("init 6")
        || trimmed.starts_with("halt")
        || trimmed.starts_with("poweroff")
    {
        return Some("system shutdown/reboot command".to_string());
    }

    None
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
        assert!(check_bash_safety("rm -rf /").is_some());
        assert!(check_bash_safety("rm -rf /*").is_some());
        assert!(check_bash_safety("rm -rf ~").is_some());
    }

    #[test]
    fn bash_sandbox_blocks_dangerous_commands() {
        assert!(check_bash_safety("mkfs.ext4 /dev/sda1").is_some());
        assert!(check_bash_safety("dd if=/dev/zero of=/dev/sda").is_some());
        assert!(check_bash_safety("shutdown -h now").is_some());
        assert!(check_bash_safety("reboot").is_some());
        assert!(check_bash_safety("halt").is_some());
        assert!(check_bash_safety("poweroff").is_some());
    }

    #[test]
    fn bash_sandbox_blocks_db_destructive() {
        assert!(check_bash_safety("psql -c 'DROP DATABASE prod'").is_some());
        assert!(check_bash_safety("mysql -e 'TRUNCATE TABLE users'").is_some());
    }

    #[test]
    fn bash_sandbox_blocks_system_path_modification() {
        assert!(check_bash_safety("rm /etc/passwd").is_some());
        assert!(check_bash_safety("mv /boot/vmlinuz /tmp/").is_some());
    }

    #[test]
    fn bash_sandbox_allows_safe_commands() {
        assert!(check_bash_safety("ls -la").is_none());
        assert!(check_bash_safety("cat README.md").is_none());
        assert!(check_bash_safety("cargo test").is_none());
        assert!(check_bash_safety("git status").is_none());
        assert!(check_bash_safety("grep -rn pattern .").is_none());
        assert!(check_bash_safety("rm src/temp.txt").is_none());
    }
}
