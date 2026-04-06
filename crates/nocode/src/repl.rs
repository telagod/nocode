use nocode_core::{
    DefaultDreamHost, LiveTaskRuntimeDriver, LiveTaskShellHost, ModelProvider, ModelStreamEvent,
    ModelStreamSink, ProcessAgentBackoffProfile, ProcessAgentBackoffStrategy, ProcessTaskAgentHost,
    QueryEngine, QueryEngineConfig, QueryLoopContinueReason, QueryLoopOutcome, QueryLoopTerminal,
    QuerySubmissionPlan, SubmitMessageOptions, TaskCoordinator, TaskDriveReport, TaskId, TaskType,
    TranscriptEntry, stop_task,
};
use std::collections::{HashMap, VecDeque};
use std::io::{self, BufRead, Write};
use std::sync::mpsc;

use crate::status_hud::StatusHud;
use crate::task_panel;

const TASK_ACTIVITY_HISTORY_LIMIT: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplInputMode {
    Prompt,
    SlashCommand,
}

impl ReplInputMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Prompt => "prompt",
            Self::SlashCommand => "slash",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplInputOrigin {
    Local,
    Queued,
}

impl ReplInputOrigin {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Queued => "queued",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplInputRecord {
    pub value: String,
    pub mode: ReplInputMode,
    pub origin: ReplInputOrigin,
}

impl ReplInputRecord {
    pub fn new(value: impl Into<String>, mode: ReplInputMode, origin: ReplInputOrigin) -> Self {
        Self {
            value: value.into(),
            mode,
            origin,
        }
    }

    pub fn render(&self) -> String {
        format!(
            "[{}:{}] {}",
            self.origin.as_str(),
            self.mode.as_str(),
            self.value
        )
    }
}

/// Tracks the prompts and slash commands the user has entered.
#[derive(Debug, Default)]
pub struct ReplHistory {
    entries: Vec<ReplInputRecord>,
}

impl ReplHistory {
    pub fn record(&mut self, entry: ReplInputRecord) {
        self.entries.push(entry);
    }

    #[cfg(test)]
    pub fn entries(&self) -> &[ReplInputRecord] {
        self.entries.as_slice()
    }

    #[allow(dead_code)]
    pub fn last(&self) -> Option<&ReplInputRecord> {
        self.entries.last()
    }

    fn prompt_records(&self) -> Vec<&ReplInputRecord> {
        self.entries
            .iter()
            .filter(|entry| entry.mode == ReplInputMode::Prompt)
            .collect()
    }

    fn render(&self) -> String {
        if self.entries.is_empty() {
            return String::from("input history pending / run a prompt first");
        }
        self.entries
            .iter()
            .map(ReplInputRecord::render)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionViewLine {
    pub turn: u32,
    pub role: String,
    pub content: String,
}

impl SessionViewLine {
    fn from_entry(entry: &TranscriptEntry) -> Self {
        Self {
            turn: entry.turn,
            role: entry.role.as_str().to_string(),
            content: normalize_session_content(entry.content.as_str()),
        }
    }

    fn render(&self) -> String {
        format!("[t{}:{}] {}", self.turn, self.role, self.content)
    }

    fn tui_role_badge(&self) -> &'static str {
        match self.role.as_str() {
            "conversation" => "MSG",
            "tool_request" => "CALL",
            "tool_progress" => "PROG",
            "tool_result" => "DONE",
            "tool_message" => "NOTE",
            "response-result" => "RESULT",
            _ => "EVENT",
        }
    }

    fn render_tui(&self, prompt_origin: Option<ReplInputOrigin>) -> String {
        match self.role.as_str() {
            "conversation" => {
                if let Some((speaker, body)) = split_session_speaker(self.content.as_str()) {
                    let badge = match speaker {
                        "system" => "SYS",
                        "user" => "USR",
                        "assistant" => "AST",
                        _ => "MSG",
                    };
                    if speaker == "user" {
                        let origin = prompt_origin
                            .map(|value| format!(":{}", value.as_str()))
                            .unwrap_or_default();
                        format!("  [{badge}{origin}] {body}")
                    } else {
                        format!("  [{badge}] {body}")
                    }
                } else {
                    format!("  [{}] {}", self.tui_role_badge(), self.content)
                }
            }
            "response-result" => format!(
                "  [{}] {}",
                self.tui_role_badge(),
                self.content
                    .strip_prefix("result=")
                    .unwrap_or(self.content.as_str())
            ),
            _ => format!("  [{}] {}", self.tui_role_badge(), self.content),
        }
    }
}

fn normalize_session_content(content: &str) -> String {
    const MAX_CHARS: usize = 180;
    let single_line = content.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = single_line.chars();
    let preview = chars.by_ref().take(MAX_CHARS).collect::<String>();
    if chars.next().is_some() {
        format!("{preview}...")
    } else {
        preview
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionView {
    max_lines: usize,
    lines: Vec<SessionViewLine>,
}

impl Default for SessionView {
    fn default() -> Self {
        Self::new(64)
    }
}

impl SessionView {
    pub fn new(max_lines: usize) -> Self {
        Self {
            max_lines: max_lines.max(1),
            lines: Vec::new(),
        }
    }

    pub fn append_entries(&mut self, entries: &[TranscriptEntry]) -> Vec<SessionViewLine> {
        let new_lines = entries
            .iter()
            .map(SessionViewLine::from_entry)
            .collect::<Vec<_>>();
        self.append_lines(new_lines)
    }

    pub fn append_lines(&mut self, new_lines: Vec<SessionViewLine>) -> Vec<SessionViewLine> {
        self.lines.extend(new_lines.iter().cloned());
        if self.lines.len() > self.max_lines {
            let overflow = self.lines.len() - self.max_lines;
            self.lines.drain(0..overflow);
        }
        new_lines
    }

    pub fn render_lines(lines: &[SessionViewLine]) -> String {
        lines
            .iter()
            .map(SessionViewLine::render)
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn render_scrollback(&self) -> String {
        if self.lines.is_empty() {
            return String::from("session view pending / run a prompt first");
        }
        Self::render_lines(self.lines.as_slice())
    }

    pub fn render_tui_timeline(&self, prompt_origins: &[ReplInputOrigin]) -> String {
        if self.lines.is_empty() {
            return String::from("session timeline pending / run a prompt first");
        }

        let mut rendered = vec![format!(
            "timeline turns={} lines={}",
            self.lines
                .iter()
                .map(|line| line.turn)
                .max()
                .unwrap_or_default(),
            self.lines.len()
        )];
        let mut current_turn = None;
        let mut prompt_index = 0usize;
        for line in &self.lines {
            if current_turn != Some(line.turn) {
                if current_turn.is_some() {
                    rendered.push(String::new());
                }
                current_turn = Some(line.turn);
                rendered.push(format!("turn {}", line.turn));
            }

            let prompt_origin = if line.role == "conversation"
                && split_session_speaker(line.content.as_str())
                    .is_some_and(|(speaker, _)| speaker == "user")
            {
                let origin = prompt_origins.get(prompt_index).copied();
                prompt_index = prompt_index.saturating_add(1);
                origin
            } else {
                None
            };
            rendered.push(line.render_tui(prompt_origin));
        }
        rendered.join("\n")
    }

    pub fn lines(&self) -> &[SessionViewLine] {
        self.lines.as_slice()
    }
}

fn split_session_speaker(content: &str) -> Option<(&str, &str)> {
    content
        .split_once(": ")
        .map(|(speaker, body)| (speaker.trim(), body))
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReplEditorState {
    draft: String,
    history_cursor: Option<usize>,
    history_stash: Option<String>,
}

impl ReplEditorState {
    pub fn set_draft(&mut self, value: impl Into<String>) {
        self.draft = value.into();
        self.history_cursor = None;
        self.history_stash = None;
    }

    pub fn append(&mut self, value: &str) {
        if !self.draft.is_empty() && !value.is_empty() {
            self.draft.push(' ');
        }
        self.draft.push_str(value);
        self.history_cursor = None;
        self.history_stash = None;
    }

    pub fn draft(&self) -> Option<&str> {
        if self.draft.is_empty() {
            None
        } else {
            Some(self.draft.as_str())
        }
    }

    pub fn take_draft(&mut self) -> String {
        self.history_cursor = None;
        self.history_stash = None;
        std::mem::take(&mut self.draft)
    }

    pub fn set_from_history(&mut self, cursor: Option<usize>, draft: impl Into<String>) {
        self.history_cursor = cursor;
        self.draft = draft.into();
    }

    pub fn stash_current_draft(&mut self) {
        if self.history_cursor.is_none() {
            self.history_stash = (!self.draft.is_empty()).then(|| self.draft.clone());
        }
    }

    pub fn restore_stashed_draft(&mut self) -> bool {
        if let Some(draft) = self.history_stash.take() {
            self.history_cursor = None;
            self.draft = draft;
            true
        } else {
            false
        }
    }

    pub fn clear(&mut self) {
        self.history_cursor = None;
        self.history_stash = None;
        self.draft.clear();
    }
}

/// Represents a concrete intent to hand to the query engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplIntent {
    pub prompt: String,
    pub options: SubmitMessageOptions,
}

impl ReplIntent {
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            options: SubmitMessageOptions::default(),
        }
    }

    #[allow(dead_code)]
    pub fn with_options(mut self, options: SubmitMessageOptions) -> Self {
        self.options = options;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReplCommandEnvelope {
    raw_input: String,
    mode: ReplInputMode,
    origin: ReplInputOrigin,
    command: ReplCommand,
}

struct ReplTaskRuntime {
    coordinator: TaskCoordinator,
    driver: ReplTaskDriver,
    info: ReplTaskRuntimeInfo,
}

type ProcessReplTaskDriver =
    LiveTaskRuntimeDriver<LiveTaskShellHost, ProcessTaskAgentHost, DefaultDreamHost>;

#[derive(Debug)]
enum ReplTaskDriver {
    InProcess(LiveTaskRuntimeDriver),
    Process(ProcessReplTaskDriver),
}

impl ReplTaskDriver {
    fn drive_next(
        &mut self,
        coordinator: &mut TaskCoordinator,
    ) -> Result<Option<nocode_core::TaskDriveReport>, nocode_core::TaskDriveError> {
        match self {
            Self::InProcess(driver) => coordinator.drive_next(driver),
            Self::Process(driver) => coordinator.drive_next(driver),
        }
    }

    fn drive_all(
        &mut self,
        coordinator: &mut TaskCoordinator,
    ) -> Result<Vec<nocode_core::TaskDriveReport>, nocode_core::TaskDriveError> {
        match self {
            Self::InProcess(driver) => coordinator.drive_all(driver),
            Self::Process(driver) => coordinator.drive_all(driver),
        }
    }

    fn drive_until_idle(
        &mut self,
        coordinator: &mut TaskCoordinator,
        max_iterations: usize,
    ) -> Result<Vec<nocode_core::TaskDriveReport>, nocode_core::TaskDriveError> {
        match self {
            Self::InProcess(driver) => coordinator.drive_until_idle(driver, max_iterations),
            Self::Process(driver) => coordinator.drive_until_idle(driver, max_iterations),
        }
    }
}

#[derive(Debug, Clone)]
struct ReplTaskRuntimeInfo {
    shell_host: &'static str,
    agent_host: &'static str,
    command: Option<String>,
    args: Vec<String>,
    cwd: String,
    dream_host: &'static str,
}

impl ReplTaskRuntime {
    fn new(task_config: QueryEngineConfig) -> Self {
        let (driver, info) = build_task_runtime_driver(&task_config);
        Self {
            coordinator: TaskCoordinator::new(),
            driver,
            info,
        }
    }

    fn render_runtime_footer(&self) -> String {
        let command = self.info.command.as_deref().unwrap_or("embedded");
        let args = if self.info.args.is_empty() {
            String::from("-")
        } else {
            self.info.args.join(" ")
        };
        let mut lines = vec![
            String::from("task runtime:"),
            format!("shell={}", self.info.shell_host),
            format!("agent={}", self.info.agent_host),
            format!("agent.command={command}"),
            format!("agent.args={args}"),
            format!("cwd={}", self.info.cwd),
            format!("dream={}", self.info.dream_host),
        ];
        if let ReplTaskDriver::Process(driver) = &self.driver {
            let host = &driver.agent_host;
            lines.push(format!(
                "agent.health=running:{} requests:{} spawns:{} restarts:{}/{} failures:{}/{} backoff={} base_backoff_ms={} jitter_pct={} last_backoff_profile={} last_backoff_ms={}",
                host.daemon_running(),
                host.request_count(),
                host.spawn_count(),
                host.restart_count(),
                host.max_restart_attempts(),
                host.consecutive_failure_count(),
                host.max_consecutive_failures(),
                host.restart_backoff_strategy().as_str(),
                host.restart_backoff_ms(),
                host.restart_backoff_jitter_percent(),
                host.last_backoff_profile_kind()
                    .map(|value| value.as_str())
                    .unwrap_or("none"),
                host.last_backoff_ms()
            ));
            lines.push(format!(
                "agent.last_exit={}",
                host.last_exit_code()
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| String::from("none"))
            ));
            lines.push(format!(
                "agent.last_failure_kind={}",
                host.last_failure_kind()
                    .map(|value| value.as_str())
                    .unwrap_or("none")
            ));
            lines.push(format!(
                "agent.last_error={}",
                host.last_error().unwrap_or("none")
            ));
        }
        lines.join("\n")
    }

    fn append_runtime_footer(&self, rendered: String) -> String {
        format!("{rendered}\n{}", self.render_runtime_footer())
    }

    fn render_runtime_status_line(&self) -> String {
        match &self.driver {
            ReplTaskDriver::InProcess(_) => String::from("runtime agent=in-process"),
            ReplTaskDriver::Process(driver) => {
                let host = &driver.agent_host;
                format!(
                    "runtime agent={} running={} requests={} spawns={} restarts={}/{} failures:{}/{} backoff={} base_backoff_ms={} jitter_pct={} last_backoff_profile={} last_backoff_ms={} last_exit={} last_failure={} last_error={}",
                    host.mode_label(),
                    host.daemon_running(),
                    host.request_count(),
                    host.spawn_count(),
                    host.restart_count(),
                    host.max_restart_attempts(),
                    host.consecutive_failure_count(),
                    host.max_consecutive_failures(),
                    host.restart_backoff_strategy().as_str(),
                    host.restart_backoff_ms(),
                    host.restart_backoff_jitter_percent(),
                    host.last_backoff_profile_kind()
                        .map(|value| value.as_str())
                        .unwrap_or("none"),
                    host.last_backoff_ms(),
                    host.last_exit_code()
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| String::from("none")),
                    host.last_failure_kind()
                        .map(|value| value.as_str())
                        .unwrap_or("none"),
                    host.last_error().unwrap_or("none")
                )
            }
        }
    }

    fn info_for_process_host(
        task_config: &QueryEngineConfig,
        daemon_mode: bool,
        command: String,
        args: Vec<String>,
    ) -> ReplTaskRuntimeInfo {
        ReplTaskRuntimeInfo {
            shell_host: "live-shell",
            agent_host: if daemon_mode {
                "process-daemon"
            } else {
                "process-host"
            },
            command: Some(command),
            args,
            cwd: task_config.cwd.clone(),
            dream_host: "default",
        }
    }
}

impl std::fmt::Debug for ReplTaskRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplTaskRuntime")
            .field("coordinator", &self.coordinator)
            .field("driver", &self.driver)
            .field("info", &self.info)
            .finish()
    }
}

fn render_runtime_report(runtime: &ReplTaskRuntime) -> String {
    format!(
        "runtime report:\n{}\n{}",
        runtime.render_runtime_status_line(),
        runtime.render_runtime_footer()
    )
}

fn build_task_runtime_driver(
    task_config: &QueryEngineConfig,
) -> (ReplTaskDriver, ReplTaskRuntimeInfo) {
    match process_task_agent_host(task_config) {
        Some((agent_host, info)) => (
            ReplTaskDriver::Process(LiveTaskRuntimeDriver::with_hosts(
                LiveTaskShellHost,
                agent_host,
                DefaultDreamHost,
            )),
            info,
        ),
        None => (
            ReplTaskDriver::InProcess(LiveTaskRuntimeDriver::new(task_config.clone())),
            ReplTaskRuntimeInfo {
                shell_host: "live-shell",
                agent_host: "in-process",
                command: None,
                args: Vec::new(),
                cwd: task_config.cwd.clone(),
                dream_host: "default",
            },
        ),
    }
}

fn process_task_agent_host(
    task_config: &QueryEngineConfig,
) -> Option<(ProcessTaskAgentHost, ReplTaskRuntimeInfo)> {
    process_task_agent_host_from_lookup(task_config, |name| std::env::var(name).ok())
}

fn process_task_agent_host_from_lookup<F>(
    task_config: &QueryEngineConfig,
    get_var: F,
) -> Option<(ProcessTaskAgentHost, ReplTaskRuntimeInfo)>
where
    F: Fn(&str) -> Option<String>,
{
    let command_override = get_var("NOCODE_TASK_AGENT_COMMAND");
    let host_mode = get_var("NOCODE_TASK_AGENT_HOST");
    let restart_budget =
        get_var("NOCODE_TASK_AGENT_DAEMON_RESTARTS").and_then(|value| value.parse::<u8>().ok());
    let max_consecutive_failures = get_var("NOCODE_TASK_AGENT_DAEMON_MAX_CONSECUTIVE_FAILURES")
        .and_then(|value| value.parse::<u8>().ok());
    let io_backoff_profile = get_var("NOCODE_TASK_AGENT_DAEMON_IO_BACKOFF")
        .and_then(|value| parse_backoff_profile_value(value.as_str()));
    let decode_backoff_profile = get_var("NOCODE_TASK_AGENT_DAEMON_DECODE_BACKOFF")
        .and_then(|value| parse_backoff_profile_value(value.as_str()));
    let exit_backoff_profile = get_var("NOCODE_TASK_AGENT_DAEMON_EXIT_BACKOFF")
        .and_then(|value| parse_backoff_profile_value(value.as_str()));
    let restart_backoff_strategy = get_var("NOCODE_TASK_AGENT_DAEMON_BACKOFF_STRATEGY")
        .and_then(|value| parse_backoff_strategy_value(value.as_str()));
    let restart_backoff_jitter_percent = get_var("NOCODE_TASK_AGENT_DAEMON_BACKOFF_JITTER_PERCENT")
        .and_then(|value| value.parse::<u8>().ok());
    let restart_backoff_ms =
        get_var("NOCODE_TASK_AGENT_DAEMON_BACKOFF_MS").and_then(|value| value.parse::<u64>().ok());
    let restart_on_io_error = get_var("NOCODE_TASK_AGENT_DAEMON_RESTART_ON_IO_ERROR")
        .and_then(|value| parse_env_bool_value(value.as_str()));
    let restart_on_decode_error = get_var("NOCODE_TASK_AGENT_DAEMON_RESTART_ON_DECODE_ERROR")
        .and_then(|value| parse_env_bool_value(value.as_str()));
    let restart_on_clean_exit = get_var("NOCODE_TASK_AGENT_DAEMON_RESTART_ON_CLEAN_EXIT")
        .and_then(|value| parse_env_bool_value(value.as_str()));
    let daemon_mode = matches!(host_mode.as_deref(), Some("daemon"));
    let use_process_host = command_override.is_some()
        || matches!(
            host_mode.as_deref(),
            Some("process" | "external" | "daemon")
        );
    if !use_process_host {
        return None;
    }

    let (command, args) = if let Some(command) = command_override {
        (
            command,
            split_process_agent_args(get_var("NOCODE_TASK_AGENT_ARGS").as_deref()),
        )
    } else {
        let exe = std::env::current_exe().ok()?;
        (
            exe.to_string_lossy().into_owned(),
            vec![if daemon_mode {
                String::from("--process-agent-daemon")
            } else {
                String::from("--process-agent-host")
            }],
        )
    };
    let info = ReplTaskRuntime::info_for_process_host(
        task_config,
        daemon_mode,
        command.clone(),
        args.clone(),
    );

    let mut host = ProcessTaskAgentHost::with_args(command, args).with_cwd(task_config.cwd.clone());
    if let Some(attempts) = restart_budget {
        host = host.with_daemon_restart_budget(attempts);
    }
    if let Some(failures) = max_consecutive_failures {
        host = host.with_daemon_max_consecutive_failures(failures);
    }
    if let Some(profile) = io_backoff_profile {
        host = host.with_daemon_io_backoff_profile(profile);
    }
    if let Some(profile) = decode_backoff_profile {
        host = host.with_daemon_decode_backoff_profile(profile);
    }
    if let Some(profile) = exit_backoff_profile {
        host = host.with_daemon_exit_backoff_profile(profile);
    }
    if let Some(strategy) = restart_backoff_strategy {
        host = host.with_daemon_restart_backoff_strategy(strategy);
    }
    if let Some(jitter_percent) = restart_backoff_jitter_percent {
        host = host.with_daemon_restart_backoff_jitter_percent(jitter_percent);
    }
    if let Some(backoff_ms) = restart_backoff_ms {
        host = host.with_daemon_restart_backoff_ms(backoff_ms);
    }
    if let Some(enabled) = restart_on_io_error {
        host = host.with_daemon_restart_on_io_error(enabled);
    }
    if let Some(enabled) = restart_on_decode_error {
        host = host.with_daemon_restart_on_decode_error(enabled);
    }
    if let Some(enabled) = restart_on_clean_exit {
        host = host.with_daemon_restart_on_clean_exit(enabled);
    }
    let host = if daemon_mode {
        host.with_daemon_mode()
    } else {
        host
    };
    Some((host, info))
}

fn split_process_agent_args(raw: Option<&str>) -> Vec<String> {
    raw.map(|value| {
        value
            .split_whitespace()
            .filter(|segment| !segment.is_empty())
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    })
    .unwrap_or_default()
}

fn parse_env_bool_value(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[allow(dead_code)]
fn parse_backoff_strategy(name: &str) -> Option<ProcessAgentBackoffStrategy> {
    let raw = std::env::var(name).ok()?;
    parse_backoff_strategy_value(raw.as_str())
}

#[allow(dead_code)]
fn parse_backoff_profile(name: &str) -> Option<ProcessAgentBackoffProfile> {
    let raw = std::env::var(name).ok()?;
    parse_backoff_profile_value(raw.as_str())
}

fn parse_backoff_profile_value(raw: &str) -> Option<ProcessAgentBackoffProfile> {
    let mut parts = raw.trim().split(':');
    let base_delay_ms = parts.next()?.trim().parse::<u64>().ok()?;
    let strategy = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(parse_backoff_strategy_value)
        .unwrap_or(ProcessAgentBackoffStrategy::Linear);
    let jitter_percent = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<u8>().ok())
        .unwrap_or(0);
    Some(ProcessAgentBackoffProfile {
        base_delay_ms,
        strategy,
        jitter_percent,
    })
}

fn parse_backoff_strategy_value(raw: &str) -> Option<ProcessAgentBackoffStrategy> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "linear" => Some(ProcessAgentBackoffStrategy::Linear),
        "exp" | "exponential" => Some(ProcessAgentBackoffStrategy::Exponential),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ReplPaneFocus {
    #[default]
    Transcript,
    TaskList,
    TaskDetail,
}

impl ReplPaneFocus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Transcript => "transcript",
            Self::TaskList => "task_list",
            Self::TaskDetail => "task_detail",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "transcript" | "session" | "history" => Some(Self::Transcript),
            "tasks" | "task-list" | "task_list" | "list" => Some(Self::TaskList),
            "detail" | "task-detail" | "task_detail" | "inspect" => Some(Self::TaskDetail),
            _ => None,
        }
    }
}

#[derive(Debug, Default)]
struct ReplTaskPanelState {
    active_filter: Option<String>,
    inspected_task_id: Option<TaskId>,
    selected_task_id: Option<TaskId>,
    last_drive_report: Option<TaskDriveReport>,
    activity_history: HashMap<TaskId, VecDeque<TaskDriveReport>>,
    focus: ReplPaneFocus,
}

/// User actions produced by the REPL loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplCommand {
    Quit,
    Intent(ReplIntent),
    Status,
    Runtime,
    History,
    Help,
    InputHistory,
    QueueShow,
    Tasks(Option<String>),
    TaskQueue,
    ShowDraft,
    SetDraft(String),
    AppendDraft(String),
    SubmitDraft,
    HistoryPrev,
    HistoryNext,
    QueuePrompt(String),
    QueueSlash(String),
    TaskSpawnShell(String),
    TaskSpawnAgent {
        agent_id: String,
        prompt: String,
    },
    TaskSpawnDream {
        sessions_reviewing: usize,
        description: Option<String>,
    },
    TaskShow(String),
    TasksNext,
    TasksPrev,
    TaskOpen,
    Focus(String),
    PaneNext,
    PanePrev,
    PaneActivate,
    TaskRunNext,
    TaskRunAll,
    TaskStop(String),
    Print(String),
    GitCommit(String),
    GitDiff(Option<String>),
    GitBranch(Option<String>),
    // P2
    Login(Option<String>),
    Logout,
    Doctor,
    Ide,
    Plugin(Option<String>),
    // Team agent
    TeamCreate(String),
    TeamStatus,
}

// ---------------------------------------------------------------------------
// W1: Pending async submission state for TUI live streaming
// ---------------------------------------------------------------------------

/// Tracks an in-flight model call running on a background thread.
pub struct PendingSubmission {
    pub stream_rx: mpsc::Receiver<ModelStreamEvent>,
    pub result_rx: mpsc::Receiver<(QuerySubmissionPlan, QueryEngine)>,
    pub accumulated_text: String,
    pub started: bool,
}

impl std::fmt::Debug for PendingSubmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingSubmission")
            .field("accumulated_len", &self.accumulated_text.len())
            .field("started", &self.started)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// W2: TUI permission request types
// ---------------------------------------------------------------------------

/// All recognized slash command names for tab completion.
#[allow(dead_code)]
const SLASH_COMMAND_NAMES: &[&str] = &[
    "append",
    "branch",
    "commit",
    "diff",
    "doctor",
    "down",
    "draft",
    "edit",
    "enter",
    "focus",
    "help",
    "history",
    "history-next",
    "history-prev",
    "ide",
    "inputs",
    "j",
    "k",
    "login",
    "logout",
    "plugin",
    "plugins",
    "queue",
    "queue-show",
    "queue-slash",
    "quit",
    "runtime",
    "send",
    "status",
    "task-agent",
    "task-dream",
    "task-open",
    "task-queue",
    "task-run-all",
    "task-run-next",
    "task-shell",
    "task-show",
    "task-stop",
    "tasks",
    "tasks-next",
    "tasks-prev",
    "team-create",
    "team-status",
    "up",
];

/// Minimal REPL session metadata plus state for history, queue, and draft editing.
#[derive(Debug)]
pub struct ReplSession {
    prompt_label: String,
    history: ReplHistory,
    queued_commands: VecDeque<ReplCommandEnvelope>,
    editor: ReplEditorState,
    last_plan_summary: Option<String>,
    last_plan_diagnostics: Option<String>,
    last_plan_response_result_pretty: Option<String>,
    last_plan_stream_lines: Vec<String>,
    session_view: SessionView,
    task_runtime: Option<ReplTaskRuntime>,
    task_panel: ReplTaskPanelState,
    // W1: async streaming
    pending_submission: Option<PendingSubmission>,
    pending_intent: Option<ReplIntent>,
    tui_mode: bool,
    // W2: permission prompts
    permission_rx: Option<mpsc::Receiver<crate::tui_permission::TuiPermissionBridgeRequest>>,
    pending_permissions: Vec<crate::tui_permission::TuiPermissionBridgeRequest>,
    permission_cursor: usize,
    tui_prompter: Option<crate::tui_permission::TuiPermissionPrompter>,
    // T7: status bar HUD
    status_hud: StatusHud,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplTuiSnapshot {
    pub focus: String,
    pub status_line: String,
    pub diagnostics_line: String,
    pub queue_line: String,
    pub editor_line: String,
    pub footer_line: String,
    pub transcript: String,
    pub task_list: String,
    pub task_detail: String,
    pub hud_line: String,
}

impl ReplSession {
    pub fn new(prompt_label: impl Into<String>) -> Self {
        Self {
            prompt_label: prompt_label.into(),
            history: ReplHistory::default(),
            queued_commands: VecDeque::new(),
            editor: ReplEditorState::default(),
            last_plan_summary: None,
            last_plan_diagnostics: None,
            last_plan_response_result_pretty: None,
            last_plan_stream_lines: Vec::new(),
            session_view: SessionView::default(),
            task_runtime: None,
            task_panel: ReplTaskPanelState::default(),
            pending_submission: None,
            pending_intent: None,
            tui_mode: false,
            permission_rx: None,
            pending_permissions: Vec::new(),
            permission_cursor: 0,
            tui_prompter: None,
            status_hud: StatusHud::new("pending", "pending"),
        }
    }

    pub fn set_tui_prompter(&mut self, prompter: crate::tui_permission::TuiPermissionPrompter) {
        self.tui_prompter = Some(prompter);
    }

    pub fn prompt_text(&self) -> String {
        format!("{}> ", self.prompt_label)
    }

    /// Tab-complete a slash command prefix.
    ///
    /// Returns `Ok(completed)` when exactly one command matches, or
    /// `Err(candidates)` when zero or multiple commands match.
    #[allow(dead_code)]
    pub fn complete_command(&self, prefix: &str) -> Result<String, Vec<String>> {
        let needle = prefix
            .strip_prefix('/')
            .unwrap_or(prefix)
            .to_ascii_lowercase();
        let matches: Vec<&str> = SLASH_COMMAND_NAMES
            .iter()
            .copied()
            .filter(|name| name.starts_with(needle.as_str()))
            .collect();
        match matches.len() {
            1 => Ok(format!("/{}", matches[0])),
            _ => Err(matches.iter().map(|name| format!("/{name}")).collect()),
        }
    }

    pub fn tui_draft(&self) -> &str {
        self.editor.draft().unwrap_or("")
    }

    pub fn set_tui_draft(&mut self, value: impl Into<String>) {
        self.editor.set_draft(value);
    }

    pub fn clear_tui_draft(&mut self) {
        self.editor.clear();
    }

    pub fn process_local_line<W: Write>(
        &mut self,
        engine: &mut QueryEngine,
        writer: &mut W,
        line: &str,
    ) -> io::Result<bool> {
        let command = self.parse_input(line, ReplInputOrigin::Local);
        self.execute_command(engine, writer, command)
    }

    pub fn tick_tasks<W: Write>(&mut self, engine: &QueryEngine, writer: &mut W) -> io::Result<()> {
        self.maybe_tick_task_updates(engine, writer)
    }

    // -----------------------------------------------------------------------
    // W1: Async streaming support
    // -----------------------------------------------------------------------

    /// Returns true when a background model call is in flight.
    pub fn is_streaming(&self) -> bool {
        self.pending_submission.is_some()
    }

    /// Drain pending stream events from the background thread.
    /// Returns `(lines, maybe_engine)` — engine is returned when the submission completes.
    pub fn poll_pending_stream(&mut self) -> (Vec<String>, Option<QueryEngine>) {
        let Some(pending) = self.pending_submission.as_mut() else {
            return (Vec::new(), None);
        };
        let mut lines = Vec::new();

        // Drain all available stream events.
        while let Ok(event) = pending.stream_rx.try_recv() {
            match &event {
                ModelStreamEvent::Start { provider, model } => {
                    if !pending.started {
                        pending.started = true;
                        self.status_hud.model_name = model.clone();
                        self.status_hud.start_turn();
                        lines.push(format!(
                            "stream start: provider={} model={}",
                            provider.as_str(),
                            model
                        ));
                    }
                }
                ModelStreamEvent::Delta { text, .. } => {
                    pending.accumulated_text.push_str(text);
                    // Approximate streaming output tokens: 1 token ~ 4 chars.
                    let approx_tokens = (text.len() as u64).div_ceil(4);
                    self.status_hud.record_tokens(0, approx_tokens);
                    lines.push(format!(
                        "stream delta: {}",
                        normalize_session_content(text.as_str())
                    ));
                }
                ModelStreamEvent::Complete { message } => {
                    lines.push(format!(
                        "stream complete: {}",
                        normalize_session_content(message.content.as_str())
                    ));
                }
                ModelStreamEvent::StreamError { message, .. } => {
                    lines.push(format!(
                        "stream error: {}",
                        normalize_session_content(message.as_str())
                    ));
                }
            }
        }

        // Check if the final result has arrived.
        let mut returned_engine = None;
        if let Ok((plan, engine)) = pending.result_rx.try_recv() {
            // Surface model errors prominently before the full submission render.
            if let Some(error) = &plan.model_error {
                lines.push(format!(
                    "stream error: {} (kind={}, retryable={})",
                    error.message,
                    error.kind.as_str(),
                    error.retryable
                ));
            }
            let rendered = self.render_submission(&plan);
            lines.push(rendered);
            self.last_plan_summary = Some(render_plan_status(&plan));
            self.last_plan_diagnostics = Some(render_plan_diagnostics(&plan));
            self.last_plan_response_result_pretty = plan.response_result_pretty();
            // T7: finalize HUD with real usage from the plan.
            self.finalize_hud_from_plan(&plan);
            self.pending_submission = None;
            returned_engine = Some(engine);
        }

        (lines, returned_engine)
    }

    // -----------------------------------------------------------------------
    // W2: Permission prompt support
    // -----------------------------------------------------------------------

    /// Install the receiving end of the permission channel.
    #[allow(dead_code)]
    pub fn set_permission_rx(
        &mut self,
        rx: mpsc::Receiver<crate::tui_permission::TuiPermissionBridgeRequest>,
    ) {
        self.permission_rx = Some(rx);
    }

    /// Drain incoming permission requests from the channel.
    /// Returns true if new requests arrived.
    pub fn poll_permissions(&mut self) -> bool {
        let Some(rx) = &self.permission_rx else {
            return false;
        };
        let mut got_new = false;
        while let Ok(req) = rx.try_recv() {
            self.pending_permissions.push(req);
            got_new = true;
        }
        got_new
    }

    /// Approve or deny the currently selected permission request.
    pub fn resolve_permission(&mut self, approved: bool) {
        if self.pending_permissions.is_empty() {
            return;
        }
        let idx = self
            .permission_cursor
            .min(self.pending_permissions.len().saturating_sub(1));
        let req = self.pending_permissions.remove(idx);
        let _ = req.response_tx.send(approved);
        if self.permission_cursor > 0 && self.permission_cursor >= self.pending_permissions.len() {
            self.permission_cursor = self.pending_permissions.len().saturating_sub(1);
        }
    }

    /// Move permission cursor up.
    pub fn permission_cursor_up(&mut self) {
        if self.permission_cursor > 0 {
            self.permission_cursor -= 1;
        }
    }

    /// Move permission cursor down.
    pub fn permission_cursor_down(&mut self) {
        if !self.pending_permissions.is_empty() {
            self.permission_cursor =
                (self.permission_cursor + 1).min(self.pending_permissions.len() - 1);
        }
    }

    /// True if there are unresolved permission requests.
    pub fn has_pending_permissions(&self) -> bool {
        !self.pending_permissions.is_empty()
    }

    /// Enable TUI async mode — Intent commands will be deferred instead of blocking.
    pub fn set_tui_mode(&mut self, enabled: bool) {
        self.tui_mode = enabled;
    }

    /// Take a pending intent that was deferred in TUI mode.
    pub fn take_pending_intent(&mut self) -> Option<ReplIntent> {
        self.pending_intent.take()
    }

    /// Set the pending async submission state.
    pub fn set_pending_submission(&mut self, pending: PendingSubmission) {
        self.pending_submission = Some(pending);
    }

    pub fn focus_label(&self) -> &'static str {
        self.task_panel.focus.as_str()
    }

    #[allow(dead_code)]
    pub fn render_active_pane(&self) -> String {
        match self.task_panel.focus {
            ReplPaneFocus::Transcript => self.render_transcript_panel(),
            ReplPaneFocus::TaskList | ReplPaneFocus::TaskDetail => {
                if let Some(runtime) = &self.task_runtime {
                    Self::render_task_panel_state(
                        &runtime.coordinator,
                        self.task_panel.active_filter.as_deref(),
                        self.task_panel.selected_task_id.as_ref(),
                        self.task_panel.inspected_task_id.as_ref(),
                        self.task_panel.focus,
                        Some(runtime.render_runtime_footer().as_str()),
                    )
                } else {
                    let coordinator = TaskCoordinator::new();
                    Self::render_task_panel_state(
                        &coordinator,
                        self.task_panel.active_filter.as_deref(),
                        self.task_panel.selected_task_id.as_ref(),
                        self.task_panel.inspected_task_id.as_ref(),
                        self.task_panel.focus,
                        None,
                    )
                }
            }
        }
    }

    pub fn render_tui_snapshot(&self) -> ReplTuiSnapshot {
        let panel = self.build_task_panel_view();
        let transcript_title = if self.task_panel.focus == ReplPaneFocus::Transcript {
            "transcript pane [active]"
        } else {
            "transcript pane"
        };
        let task_list_title = if self.task_panel.focus == ReplPaneFocus::TaskList {
            "task list pane [active]"
        } else {
            "task list pane"
        };
        let task_detail_title = if self.task_panel.focus == ReplPaneFocus::TaskDetail {
            "task detail pane [active]"
        } else {
            "task detail pane"
        };
        let task_detail = if panel.has_detail() {
            panel.render_detail()
        } else {
            String::from("task detail pending / use /task-open, /task-show, or /enter first")
        };
        let task_list = self.append_runtime_footer_if_present(panel.render_list_summary());
        let task_detail = self.append_runtime_footer_if_present(task_detail);
        let task_detail = self.append_response_result_section(task_detail);
        let task_detail = self.append_task_activity_section(task_detail);
        let task_detail = self.append_task_activity_history_section(task_detail);
        ReplTuiSnapshot {
            focus: self.focus_label().to_string(),
            status_line: self.render_tui_status_line(),
            diagnostics_line: self.render_tui_diagnostics_line(),
            queue_line: self.render_tui_queue_line(),
            editor_line: self.render_tui_editor_line(),
            footer_line: self.render_tui_footer_line(),
            transcript: format!("{transcript_title}\n{}", self.render_tui_transcript_body()),
            task_list: format!("{task_list_title}\n{task_list}"),
            task_detail: format!("{task_detail_title}\n{task_detail}"),
            hud_line: self.render_tui_hud_line(),
        }
    }

    pub fn render_tui_help_overlay(&self) -> String {
        let mut lines = vec![
            String::from("help overlay:"),
            String::from("keys:"),
            String::from("Esc close-overlay/quit | Tab/Shift-Tab switch pane | 1-4 focus pane"),
            String::from(
                "Enter submit | Up/Down move or scroll | PgUp/PgDn/Home/End scroll | Ctrl-P/N history",
            ),
            String::from("Ctrl-A/E cursor home/end | Ctrl-U clear input | Ctrl-L clear events"),
            String::from(
                "? or F1 help | F2 inspector | F3 permissions | f toggle detail-follow/events-filter",
            ),
            String::new(),
            render_help(),
        ];
        if let Some(filter) = self.task_panel.active_filter.as_deref() {
            lines.push(String::new());
            lines.push(format!("active task filter: {filter}"));
        }
        lines.join("\n")
    }

    pub fn render_tui_inspector_overlay(&self) -> String {
        let mut sections = vec![
            String::from("inspector overlay:"),
            format!("status: {}", self.render_tui_status_line()),
            format!("diagnostics: {}", self.render_tui_diagnostics_line()),
            format!("editor: {}", self.render_tui_editor_line()),
            format!("queue: {}", self.render_tui_queue_line()),
            String::new(),
            String::from("input history:"),
            self.history.render(),
            String::new(),
            String::from("queued commands:"),
            self.render_queue(),
        ];
        sections.push(String::new());
        if let Some(runtime) = &self.task_runtime {
            sections.push(render_runtime_report(runtime));
        } else {
            sections.push(String::from(
                "runtime report:\nruntime pending / no task host initialized",
            ));
        }
        sections.join("\n")
    }

    pub fn render_tui_permission_overlay(&self) -> String {
        if self.pending_permissions.is_empty() {
            return [
                "permission overlay:",
                "",
                "no pending permission requests.",
                "",
                "permissions will appear here when tools request access.",
            ]
            .join("\n");
        }

        let mut lines = vec![format!(
            "permission requests: {} pending",
            self.pending_permissions.len()
        )];
        lines.push(String::new());

        for (i, req) in self.pending_permissions.iter().enumerate() {
            let marker = if i == self.permission_cursor {
                ">"
            } else {
                " "
            };
            lines.push(format!("{marker} {tool}", tool = req.tool_name));
            if !req.arguments_summary.is_empty() {
                lines.push(format!("    {}", req.arguments_summary));
            }
        }

        lines.push(String::new());
        lines.push(String::from(
            "[a] approve  [d] deny  [Up/Down] select  [Esc] close",
        ));
        lines.join("\n")
    }

    fn append_response_result_section(&self, rendered: String) -> String {
        if rendered.contains("response-result.pretty:")
            || rendered.contains("response result:")
            || rendered.contains("result.pretty:")
        {
            return rendered;
        }
        self.last_plan_response_result_pretty
            .as_deref()
            .map_or(rendered.clone(), |pretty| {
                format!("{rendered}\nresponse result:\n{pretty}")
            })
    }

    fn append_task_activity_section(&self, rendered: String) -> String {
        if rendered.contains("live activity:") {
            return rendered;
        }
        let Some(report) = self.task_panel.last_drive_report.as_ref() else {
            return rendered;
        };
        let Some(activity) = report.activity.as_deref() else {
            return rendered;
        };
        let current_task = self
            .task_panel
            .inspected_task_id
            .as_ref()
            .or(self.task_panel.selected_task_id.as_ref());
        if current_task != Some(&report.task_id) {
            return rendered;
        }
        format!(
            "{rendered}\nlive activity:\ntask={} status={} {}\n",
            report.task_id.as_str(),
            render_task_status_label(report.status),
            activity
        )
    }

    fn append_task_activity_history_section(&self, rendered: String) -> String {
        if rendered.contains("activity history:") {
            return rendered;
        }
        let current_task = self
            .task_panel
            .inspected_task_id
            .as_ref()
            .or(self.task_panel.selected_task_id.as_ref());
        let Some(task_id) = current_task else {
            return rendered;
        };
        let Some(history) = self.task_panel.activity_history.get(task_id) else {
            return rendered;
        };
        if history.is_empty() {
            return rendered;
        }
        let lines = history
            .iter()
            .enumerate()
            .map(|(index, report)| render_task_activity_history_line(index, report))
            .collect::<Vec<_>>()
            .join("\n");
        format!("{rendered}\nactivity history:\n{lines}\n")
    }

    #[cfg(test)]
    pub fn history(&self) -> &[ReplInputRecord] {
        self.history.entries()
    }

    #[allow(dead_code)]
    pub fn session_view(&self) -> &[SessionViewLine] {
        self.session_view.lines()
    }

    fn parse_input(&self, line: &str, origin: ReplInputOrigin) -> ReplCommandEnvelope {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return ReplCommandEnvelope {
                raw_input: String::new(),
                mode: ReplInputMode::Prompt,
                origin,
                command: ReplCommand::Intent(ReplIntent::new(String::new())),
            };
        }

        if !trimmed.starts_with('/') {
            return ReplCommandEnvelope {
                raw_input: trimmed.to_string(),
                mode: ReplInputMode::Prompt,
                origin,
                command: ReplCommand::Intent(ReplIntent::new(trimmed)),
            };
        }

        let without_slash = trimmed.trim_start_matches('/');
        let mut parts = without_slash.splitn(2, char::is_whitespace);
        let command_name = parts.next().unwrap_or_default().to_ascii_lowercase();
        let args = parts.next().unwrap_or_default().trim();
        let command = match command_name.as_str() {
            "quit" => ReplCommand::Quit,
            "status" => ReplCommand::Status,
            "runtime" => ReplCommand::Runtime,
            "history" => ReplCommand::History,
            "help" => ReplCommand::Help,
            "inputs" => ReplCommand::InputHistory,
            "queue-show" => ReplCommand::QueueShow,
            "tasks" => ReplCommand::Tasks((!args.is_empty()).then(|| args.to_string())),
            "task-queue" => ReplCommand::TaskQueue,
            "draft" | "edit" => {
                if args.is_empty() {
                    ReplCommand::ShowDraft
                } else {
                    ReplCommand::SetDraft(args.to_string())
                }
            }
            "append" => {
                if args.is_empty() {
                    ReplCommand::Print(String::from("usage: /append <text>"))
                } else {
                    ReplCommand::AppendDraft(args.to_string())
                }
            }
            "send" => ReplCommand::SubmitDraft,
            "history-prev" => ReplCommand::HistoryPrev,
            "history-next" => ReplCommand::HistoryNext,
            "queue" => {
                if args.is_empty() {
                    ReplCommand::Print(String::from("usage: /queue <prompt>"))
                } else {
                    ReplCommand::QueuePrompt(args.to_string())
                }
            }
            "queue-slash" => {
                if args.is_empty() {
                    ReplCommand::Print(String::from("usage: /queue-slash </command>"))
                } else {
                    ReplCommand::QueueSlash(args.to_string())
                }
            }
            "task-shell" => {
                if args.is_empty() {
                    ReplCommand::Print(String::from("usage: /task-shell <command>"))
                } else {
                    ReplCommand::TaskSpawnShell(args.to_string())
                }
            }
            "task-agent" => match parse_task_agent_args(args) {
                Ok((agent_id, prompt)) => ReplCommand::TaskSpawnAgent { agent_id, prompt },
                Err(message) => ReplCommand::Print(message),
            },
            "task-dream" => match parse_task_dream_args(args) {
                Ok((sessions_reviewing, description)) => ReplCommand::TaskSpawnDream {
                    sessions_reviewing,
                    description,
                },
                Err(message) => ReplCommand::Print(message),
            },
            "task-show" => {
                if args.is_empty() {
                    ReplCommand::Print(String::from(
                        "usage: /task-show <task-id|first|last|latest|prev|next>",
                    ))
                } else {
                    ReplCommand::TaskShow(args.to_string())
                }
            }
            "focus" => {
                if args.is_empty() {
                    ReplCommand::Print(String::from("usage: /focus <transcript|tasks|detail>"))
                } else {
                    ReplCommand::Focus(args.to_string())
                }
            }
            "tasks-next" => ReplCommand::TasksNext,
            "tasks-prev" => ReplCommand::TasksPrev,
            "task-open" => ReplCommand::TaskOpen,
            "j" | "down" => ReplCommand::PaneNext,
            "k" | "up" => ReplCommand::PanePrev,
            "enter" => ReplCommand::PaneActivate,
            "task-run-next" => ReplCommand::TaskRunNext,
            "task-run-all" => ReplCommand::TaskRunAll,
            "task-stop" => {
                if args.is_empty() {
                    ReplCommand::Print(String::from("usage: /task-stop <task-id>"))
                } else {
                    ReplCommand::TaskStop(args.to_string())
                }
            }
            "commit" => {
                if args.is_empty() {
                    ReplCommand::Print(String::from("usage: /commit <message>"))
                } else {
                    ReplCommand::GitCommit(args.to_string())
                }
            }
            "diff" => ReplCommand::GitDiff((!args.is_empty()).then(|| args.to_string())),
            "branch" => ReplCommand::GitBranch((!args.is_empty()).then(|| args.to_string())),
            "login" => ReplCommand::Login((!args.is_empty()).then(|| args.to_string())),
            "logout" => ReplCommand::Logout,
            "doctor" => ReplCommand::Doctor,
            "ide" => ReplCommand::Ide,
            "plugin" | "plugins" => {
                ReplCommand::Plugin((!args.is_empty()).then(|| args.to_string()))
            }
            "team-create" | "team" => {
                if args.is_empty() {
                    ReplCommand::Print(String::from(
                        "usage: /team-create <task-description>\n\
                         spawns parallel agents to work on subtasks",
                    ))
                } else {
                    ReplCommand::TeamCreate(args.to_string())
                }
            }
            "team-status" => ReplCommand::TeamStatus,
            _ => ReplCommand::Print(format!("unknown command: /{command_name} (run /help)")),
        };
        ReplCommandEnvelope {
            raw_input: trimmed.to_string(),
            mode: ReplInputMode::SlashCommand,
            origin,
            command,
        }
    }

    /// Runs the REPL loop against the given engine until EOF or `/quit`.
    ///
    /// Supports multiline input: when a line ends with `\`, the next line is
    /// appended (separated by newline) and reading continues until a line
    /// without a trailing backslash is entered.
    pub fn run_loop<R: BufRead, W: Write>(
        &mut self,
        engine: &mut QueryEngine,
        reader: &mut R,
        writer: &mut W,
    ) -> io::Result<()> {
        let mut buffer = String::new();
        let mut accumulated = String::new();
        loop {
            self.maybe_auto_drive_tasks(engine, writer)?;

            if let Some(queued) = self.queued_commands.pop_front() {
                if !self.execute_command(engine, writer, queued)? {
                    break;
                }
                continue;
            }

            buffer.clear();
            if accumulated.is_empty() {
                writer.write_all(self.prompt_text().as_bytes())?;
            } else {
                writer.write_all(b"... ")?;
            }
            writer.flush()?;

            let read = reader.read_line(&mut buffer)?;
            if read == 0 {
                if !accumulated.is_empty() {
                    let final_input = std::mem::take(&mut accumulated);
                    let command = self.parse_input(final_input.as_str(), ReplInputOrigin::Local);
                    self.execute_command(engine, writer, command)?;
                }
                break;
            }

            let raw_line = buffer.trim_end_matches(['\n', '\r']);
            if raw_line.is_empty() && accumulated.is_empty() {
                continue;
            }

            if let Some(continued) = raw_line.strip_suffix('\\') {
                if !accumulated.is_empty() {
                    accumulated.push('\n');
                }
                accumulated.push_str(continued);
                continue;
            }

            let full_input = if accumulated.is_empty() {
                raw_line.to_string()
            } else {
                accumulated.push('\n');
                accumulated.push_str(raw_line);
                std::mem::take(&mut accumulated)
            };

            let command = self.parse_input(full_input.as_str(), ReplInputOrigin::Local);
            if !self.execute_command(engine, writer, command)? {
                break;
            }
        }
        Ok(())
    }

    fn execute_command<W: Write>(
        &mut self,
        engine: &mut QueryEngine,
        writer: &mut W,
        envelope: ReplCommandEnvelope,
    ) -> io::Result<bool> {
        self.history.record(ReplInputRecord::new(
            envelope.raw_input.clone(),
            envelope.mode,
            envelope.origin,
        ));

        match envelope.command {
            ReplCommand::Quit => Ok(false),
            ReplCommand::Intent(intent) => {
                if self.tui_mode {
                    // Defer to TUI main loop for async execution.
                    self.pending_intent = Some(intent);
                    self.write_line(writer, "submitting...")?;
                    return Ok(true);
                }
                let mut live_stream = ReplLiveStreamCapture::new(writer);
                let plan = submit_intent_with_stream(engine, intent, &mut live_stream);
                self.last_plan_stream_lines = live_stream.finish()?;
                let rendered = self.render_submission(&plan);
                self.write_line(writer, rendered.as_str())?;
                self.last_plan_summary = Some(render_plan_status(&plan));
                self.last_plan_diagnostics = Some(render_plan_diagnostics(&plan));
                self.last_plan_response_result_pretty = plan.response_result_pretty();
                // T7: update HUD from synchronous submission.
                self.status_hud.start_turn();
                self.finalize_hud_from_plan(&plan);
                Ok(true)
            }
            ReplCommand::Status => {
                let status = self
                    .last_plan_summary
                    .as_deref()
                    .unwrap_or("status summary pending / run a prompt first");
                self.write_line(writer, status)?;
                Ok(true)
            }
            ReplCommand::Runtime => {
                let rendered = {
                    let runtime = self.ensure_task_runtime(engine);
                    render_runtime_report(runtime)
                };
                self.write_line(writer, rendered.as_str())?;
                Ok(true)
            }
            ReplCommand::History => {
                self.task_panel.focus = ReplPaneFocus::Transcript;
                let scrollback = self.render_transcript_panel();
                self.write_line(writer, scrollback.as_str())?;
                Ok(true)
            }
            ReplCommand::Help => {
                self.write_line(writer, render_help().as_str())?;
                Ok(true)
            }
            ReplCommand::InputHistory => {
                self.write_line(writer, self.history.render().as_str())?;
                Ok(true)
            }
            ReplCommand::QueueShow => {
                self.write_line(writer, self.render_queue().as_str())?;
                Ok(true)
            }
            ReplCommand::Tasks(filter) => {
                self.update_task_filter(filter);
                let active_filter = self.task_panel.active_filter.clone();
                let current_selected = self.task_panel.selected_task_id.clone();
                let current_task = self.task_panel.inspected_task_id.clone();
                let (selected_task, rendered) = {
                    let runtime = self.ensure_task_runtime(engine);
                    let footer = runtime.render_runtime_footer();
                    let selected_task = task_panel::align_selected_task_id(
                        &runtime.coordinator,
                        active_filter.as_deref(),
                        current_selected.as_ref(),
                    );
                    let rendered = Self::render_task_panel_state(
                        &runtime.coordinator,
                        active_filter.as_deref(),
                        selected_task.as_ref(),
                        current_task.as_ref(),
                        ReplPaneFocus::TaskList,
                        Some(footer.as_str()),
                    );
                    (selected_task, rendered)
                };
                self.task_panel.focus = ReplPaneFocus::TaskList;
                self.task_panel.selected_task_id = selected_task;
                self.write_line(writer, rendered.as_str())?;
                Ok(true)
            }
            ReplCommand::TaskQueue => {
                let rendered = {
                    let runtime = self.ensure_task_runtime(engine);
                    task_panel::render_task_queue(&runtime.coordinator)
                };
                self.write_line(writer, rendered.as_str())?;
                Ok(true)
            }
            ReplCommand::TaskShow(raw_id) => {
                let current_selected = self.task_panel.selected_task_id.clone();
                let current_task = self.task_panel.inspected_task_id.clone();
                let active_filter = self.task_panel.active_filter.clone();
                let (next_selected, next_inspected, rendered) = {
                    let runtime = self.ensure_task_runtime(engine);
                    match task_panel::resolve_task_id(
                        &runtime.coordinator,
                        raw_id.as_str(),
                        current_selected.as_ref().or(current_task.as_ref()),
                        active_filter.as_deref(),
                    ) {
                        Some(task_id) => (
                            Some(task_id.clone()),
                            Some(task_id.clone()),
                            runtime.append_runtime_footer(
                                task_panel::TaskPanelView::from_state(
                                    &runtime.coordinator,
                                    active_filter.as_deref(),
                                    Some(&task_id),
                                    Some(&task_id),
                                )
                                .render_layout_with_focus(ReplPaneFocus::TaskDetail.as_str()),
                            ),
                        ),
                        None => (None, None, String::from("task show: task not found")),
                    }
                };
                if let Some(task_id) = next_selected {
                    self.task_panel.selected_task_id = Some(task_id);
                }
                if let Some(task_id) = next_inspected {
                    self.task_panel.inspected_task_id = Some(task_id);
                    self.task_panel.focus = ReplPaneFocus::TaskDetail;
                }
                self.write_line(writer, rendered.as_str())?;
                Ok(true)
            }
            ReplCommand::TasksNext => {
                let active_filter = self.task_panel.active_filter.clone();
                let current_selected = self.task_panel.selected_task_id.clone();
                let current_task = self.task_panel.inspected_task_id.clone();
                let rendered = {
                    let runtime = self.ensure_task_runtime(engine);
                    let footer = runtime.render_runtime_footer();
                    let next_selected = task_panel::step_selected_task_id(
                        &runtime.coordinator,
                        active_filter.as_deref(),
                        current_selected.as_ref(),
                        1,
                    )
                    .or(current_selected.clone());
                    Self::render_task_panel_state(
                        &runtime.coordinator,
                        active_filter.as_deref(),
                        next_selected.as_ref(),
                        current_task.as_ref(),
                        ReplPaneFocus::TaskList,
                        Some(footer.as_str()),
                    )
                };
                self.task_panel.focus = ReplPaneFocus::TaskList;
                self.task_panel.selected_task_id = {
                    let runtime = self.ensure_task_runtime(engine);
                    task_panel::step_selected_task_id(
                        &runtime.coordinator,
                        active_filter.as_deref(),
                        current_selected.as_ref(),
                        1,
                    )
                    .or(current_selected)
                };
                self.write_line(writer, rendered.as_str())?;
                Ok(true)
            }
            ReplCommand::TasksPrev => {
                let active_filter = self.task_panel.active_filter.clone();
                let current_selected = self.task_panel.selected_task_id.clone();
                let current_task = self.task_panel.inspected_task_id.clone();
                let rendered = {
                    let runtime = self.ensure_task_runtime(engine);
                    let footer = runtime.render_runtime_footer();
                    let next_selected = task_panel::step_selected_task_id(
                        &runtime.coordinator,
                        active_filter.as_deref(),
                        current_selected.as_ref(),
                        -1,
                    )
                    .or(current_selected.clone());
                    Self::render_task_panel_state(
                        &runtime.coordinator,
                        active_filter.as_deref(),
                        next_selected.as_ref(),
                        current_task.as_ref(),
                        ReplPaneFocus::TaskList,
                        Some(footer.as_str()),
                    )
                };
                self.task_panel.focus = ReplPaneFocus::TaskList;
                self.task_panel.selected_task_id = {
                    let runtime = self.ensure_task_runtime(engine);
                    task_panel::step_selected_task_id(
                        &runtime.coordinator,
                        active_filter.as_deref(),
                        current_selected.as_ref(),
                        -1,
                    )
                    .or(current_selected)
                };
                self.write_line(writer, rendered.as_str())?;
                Ok(true)
            }
            ReplCommand::TaskOpen => {
                let active_filter = self.task_panel.active_filter.clone();
                let current_selected = self
                    .task_panel
                    .selected_task_id
                    .clone()
                    .or(self.task_panel.inspected_task_id.clone());
                let (inspected_task, rendered) = {
                    let runtime = self.ensure_task_runtime(engine);
                    let footer = runtime.render_runtime_footer();
                    let selected_task = task_panel::align_selected_task_id(
                        &runtime.coordinator,
                        active_filter.as_deref(),
                        current_selected.as_ref(),
                    );
                    let rendered = selected_task.as_ref().map_or_else(
                        || String::from("task open: no task selected"),
                        |task_id| {
                            Self::render_task_panel_state(
                                &runtime.coordinator,
                                active_filter.as_deref(),
                                selected_task.as_ref(),
                                Some(task_id),
                                ReplPaneFocus::TaskDetail,
                                Some(footer.as_str()),
                            )
                        },
                    );
                    (selected_task, rendered)
                };
                if let Some(task_id) = inspected_task.clone() {
                    self.task_panel.selected_task_id = Some(task_id.clone());
                    self.task_panel.inspected_task_id = Some(task_id);
                    self.task_panel.focus = ReplPaneFocus::TaskDetail;
                }
                self.write_line(writer, rendered.as_str())?;
                Ok(true)
            }
            ReplCommand::Focus(target) => {
                let Some(focus) = ReplPaneFocus::parse(target.as_str()) else {
                    self.write_line(writer, "focus: unknown pane (use transcript|tasks|detail)")?;
                    return Ok(true);
                };
                let rendered = match focus {
                    ReplPaneFocus::Transcript => {
                        self.task_panel.focus = ReplPaneFocus::Transcript;
                        self.render_transcript_panel()
                    }
                    ReplPaneFocus::TaskList => {
                        let active_filter = self.task_panel.active_filter.clone();
                        let current_selected = self.task_panel.selected_task_id.clone();
                        let current_task = self.task_panel.inspected_task_id.clone();
                        let (selected_task, rendered) = {
                            let runtime = self.ensure_task_runtime(engine);
                            let footer = runtime.render_runtime_footer();
                            let selected_task = task_panel::align_selected_task_id(
                                &runtime.coordinator,
                                active_filter.as_deref(),
                                current_selected.as_ref(),
                            );
                            let rendered = Self::render_task_panel_state(
                                &runtime.coordinator,
                                active_filter.as_deref(),
                                selected_task.as_ref(),
                                current_task.as_ref(),
                                ReplPaneFocus::TaskList,
                                Some(footer.as_str()),
                            );
                            (selected_task, rendered)
                        };
                        self.task_panel.focus = ReplPaneFocus::TaskList;
                        self.task_panel.selected_task_id = selected_task;
                        rendered
                    }
                    ReplPaneFocus::TaskDetail => {
                        let active_filter = self.task_panel.active_filter.clone();
                        let current_selected = self
                            .task_panel
                            .selected_task_id
                            .clone()
                            .or(self.task_panel.inspected_task_id.clone());
                        let (opened, rendered) = {
                            let runtime = self.ensure_task_runtime(engine);
                            let footer = runtime.render_runtime_footer();
                            let selected_task = task_panel::align_selected_task_id(
                                &runtime.coordinator,
                                active_filter.as_deref(),
                                current_selected.as_ref(),
                            );
                            let rendered = selected_task.as_ref().map_or_else(
                                || String::from("task open: no task selected"),
                                |task_id| {
                                    Self::render_task_panel_state(
                                        &runtime.coordinator,
                                        active_filter.as_deref(),
                                        selected_task.as_ref(),
                                        Some(task_id),
                                        ReplPaneFocus::TaskDetail,
                                        Some(footer.as_str()),
                                    )
                                },
                            );
                            (selected_task, rendered)
                        };
                        if let Some(task_id) = opened {
                            self.task_panel.selected_task_id = Some(task_id.clone());
                            self.task_panel.inspected_task_id = Some(task_id);
                            self.task_panel.focus = ReplPaneFocus::TaskDetail;
                        }
                        rendered
                    }
                };
                self.write_line(writer, rendered.as_str())?;
                Ok(true)
            }
            ReplCommand::PaneNext => {
                let active_filter = self.task_panel.active_filter.clone();
                let current_selected = self.task_panel.selected_task_id.clone();
                let current_task = self.task_panel.inspected_task_id.clone();
                let rendered = match self.task_panel.focus {
                    ReplPaneFocus::Transcript => self.render_transcript_panel(),
                    ReplPaneFocus::TaskList => {
                        let (next_selected, rendered) = {
                            let runtime = self.ensure_task_runtime(engine);
                            let footer = runtime.render_runtime_footer();
                            let next_selected = task_panel::step_selected_task_id(
                                &runtime.coordinator,
                                active_filter.as_deref(),
                                current_selected.as_ref(),
                                1,
                            )
                            .or(current_selected.clone());
                            let rendered = Self::render_task_panel_state(
                                &runtime.coordinator,
                                active_filter.as_deref(),
                                next_selected.as_ref(),
                                current_task.as_ref(),
                                ReplPaneFocus::TaskList,
                                Some(footer.as_str()),
                            );
                            (next_selected, rendered)
                        };
                        self.task_panel.selected_task_id = next_selected;
                        rendered
                    }
                    ReplPaneFocus::TaskDetail => {
                        let (next_selected, rendered) = {
                            let runtime = self.ensure_task_runtime(engine);
                            let footer = runtime.render_runtime_footer();
                            let next_selected = task_panel::step_selected_task_id(
                                &runtime.coordinator,
                                active_filter.as_deref(),
                                current_selected.as_ref(),
                                1,
                            )
                            .or(current_selected.clone());
                            let rendered = Self::render_task_panel_state(
                                &runtime.coordinator,
                                active_filter.as_deref(),
                                next_selected.as_ref(),
                                next_selected.as_ref(),
                                ReplPaneFocus::TaskDetail,
                                Some(footer.as_str()),
                            );
                            (next_selected, rendered)
                        };
                        self.task_panel.selected_task_id = next_selected.clone();
                        self.task_panel.inspected_task_id = next_selected;
                        rendered
                    }
                };
                self.write_line(writer, rendered.as_str())?;
                Ok(true)
            }
            ReplCommand::PanePrev => {
                let active_filter = self.task_panel.active_filter.clone();
                let current_selected = self.task_panel.selected_task_id.clone();
                let current_task = self.task_panel.inspected_task_id.clone();
                let rendered = match self.task_panel.focus {
                    ReplPaneFocus::Transcript => self.render_transcript_panel(),
                    ReplPaneFocus::TaskList => {
                        let (next_selected, rendered) = {
                            let runtime = self.ensure_task_runtime(engine);
                            let footer = runtime.render_runtime_footer();
                            let next_selected = task_panel::step_selected_task_id(
                                &runtime.coordinator,
                                active_filter.as_deref(),
                                current_selected.as_ref(),
                                -1,
                            )
                            .or(current_selected.clone());
                            let rendered = Self::render_task_panel_state(
                                &runtime.coordinator,
                                active_filter.as_deref(),
                                next_selected.as_ref(),
                                current_task.as_ref(),
                                ReplPaneFocus::TaskList,
                                Some(footer.as_str()),
                            );
                            (next_selected, rendered)
                        };
                        self.task_panel.selected_task_id = next_selected;
                        rendered
                    }
                    ReplPaneFocus::TaskDetail => {
                        let (next_selected, rendered) = {
                            let runtime = self.ensure_task_runtime(engine);
                            let footer = runtime.render_runtime_footer();
                            let next_selected = task_panel::step_selected_task_id(
                                &runtime.coordinator,
                                active_filter.as_deref(),
                                current_selected.as_ref(),
                                -1,
                            )
                            .or(current_selected.clone());
                            let rendered = Self::render_task_panel_state(
                                &runtime.coordinator,
                                active_filter.as_deref(),
                                next_selected.as_ref(),
                                next_selected.as_ref(),
                                ReplPaneFocus::TaskDetail,
                                Some(footer.as_str()),
                            );
                            (next_selected, rendered)
                        };
                        self.task_panel.selected_task_id = next_selected.clone();
                        self.task_panel.inspected_task_id = next_selected;
                        rendered
                    }
                };
                self.write_line(writer, rendered.as_str())?;
                Ok(true)
            }
            ReplCommand::PaneActivate => {
                let rendered = match self.task_panel.focus {
                    ReplPaneFocus::Transcript => self.render_transcript_panel(),
                    ReplPaneFocus::TaskList | ReplPaneFocus::TaskDetail => {
                        let active_filter = self.task_panel.active_filter.clone();
                        let current_selected = self
                            .task_panel
                            .selected_task_id
                            .clone()
                            .or(self.task_panel.inspected_task_id.clone());
                        let (opened, rendered) = {
                            let runtime = self.ensure_task_runtime(engine);
                            let footer = runtime.render_runtime_footer();
                            let selected_task = task_panel::align_selected_task_id(
                                &runtime.coordinator,
                                active_filter.as_deref(),
                                current_selected.as_ref(),
                            );
                            let rendered = selected_task.as_ref().map_or_else(
                                || String::from("task open: no task selected"),
                                |task_id| {
                                    Self::render_task_panel_state(
                                        &runtime.coordinator,
                                        active_filter.as_deref(),
                                        selected_task.as_ref(),
                                        Some(task_id),
                                        ReplPaneFocus::TaskDetail,
                                        Some(footer.as_str()),
                                    )
                                },
                            );
                            (selected_task, rendered)
                        };
                        if let Some(task_id) = opened {
                            self.task_panel.selected_task_id = Some(task_id.clone());
                            self.task_panel.inspected_task_id = Some(task_id);
                            self.task_panel.focus = ReplPaneFocus::TaskDetail;
                        }
                        rendered
                    }
                };
                self.write_line(writer, rendered.as_str())?;
                Ok(true)
            }
            ReplCommand::ShowDraft => {
                self.write_line(writer, self.render_draft().as_str())?;
                Ok(true)
            }
            ReplCommand::SetDraft(text) => {
                self.editor.set_draft(text);
                self.write_line(writer, self.render_draft().as_str())?;
                Ok(true)
            }
            ReplCommand::AppendDraft(text) => {
                self.editor.append(text.as_str());
                self.write_line(writer, self.render_draft().as_str())?;
                Ok(true)
            }
            ReplCommand::SubmitDraft => {
                let draft = self.editor.take_draft();
                if draft.trim().is_empty() {
                    self.write_line(writer, "draft pending / use /draft <text> first")?;
                    return Ok(true);
                }
                let queued = self.parse_input(draft.as_str(), ReplInputOrigin::Local);
                self.execute_command(engine, writer, queued)
            }
            ReplCommand::HistoryPrev => {
                let output = self.navigate_history(-1);
                self.write_line(writer, output.as_str())?;
                Ok(true)
            }
            ReplCommand::HistoryNext => {
                let output = self.navigate_history(1);
                self.write_line(writer, output.as_str())?;
                Ok(true)
            }
            ReplCommand::QueuePrompt(prompt) => {
                let queued = self.parse_input(prompt.as_str(), ReplInputOrigin::Queued);
                self.queued_commands.push_back(queued);
                self.write_line(writer, format!("queued prompt: {prompt}").as_str())?;
                Ok(true)
            }
            ReplCommand::QueueSlash(command) => {
                let slash = if command.trim_start().starts_with('/') {
                    command
                } else {
                    format!("/{}", command.trim())
                };
                let queued = self.parse_input(slash.as_str(), ReplInputOrigin::Queued);
                self.queued_commands.push_back(queued);
                self.write_line(writer, format!("queued slash: {slash}").as_str())?;
                Ok(true)
            }
            ReplCommand::TaskSpawnShell(command) => {
                let (task_id, rendered) = {
                    let runtime = self.ensure_task_runtime(engine);
                    let task_id =
                        runtime
                            .coordinator
                            .spawn_local_shell(command.clone(), None, None);
                    let rendered = task_panel::render_task_spawned(
                        task_id.clone(),
                        TaskType::LocalShell,
                        command.as_str(),
                    );
                    (task_id, rendered)
                };
                self.task_panel.selected_task_id = Some(task_id.clone());
                self.task_panel.inspected_task_id = Some(task_id);
                self.task_panel.focus = ReplPaneFocus::TaskDetail;
                self.write_line(writer, rendered.as_str())?;
                Ok(true)
            }
            ReplCommand::TaskSpawnAgent { agent_id, prompt } => {
                let (task_id, rendered) = {
                    let runtime = self.ensure_task_runtime(engine);
                    let task_id = runtime
                        .coordinator
                        .spawn_local_agent(agent_id.clone(), prompt.clone());
                    let rendered = task_panel::render_task_spawned(
                        task_id.clone(),
                        TaskType::LocalAgent,
                        format!("agent={} prompt={}", agent_id, prompt).as_str(),
                    );
                    (task_id, rendered)
                };
                self.task_panel.selected_task_id = Some(task_id.clone());
                self.task_panel.inspected_task_id = Some(task_id);
                self.task_panel.focus = ReplPaneFocus::TaskDetail;
                self.write_line(writer, rendered.as_str())?;
                Ok(true)
            }
            ReplCommand::TaskSpawnDream {
                sessions_reviewing,
                description,
            } => {
                let (task_id, rendered) = {
                    let runtime = self.ensure_task_runtime(engine);
                    let task_id = runtime
                        .coordinator
                        .spawn_dream(sessions_reviewing, description.clone());
                    let detail = description.as_deref().map_or_else(
                        || format!("sessions={sessions_reviewing}"),
                        |value| format!("sessions={sessions_reviewing} description={value}"),
                    );
                    let rendered = task_panel::render_task_spawned(
                        task_id.clone(),
                        TaskType::Dream,
                        detail.as_str(),
                    );
                    (task_id, rendered)
                };
                self.task_panel.selected_task_id = Some(task_id.clone());
                self.task_panel.inspected_task_id = Some(task_id);
                self.task_panel.focus = ReplPaneFocus::TaskDetail;
                self.write_line(writer, rendered.as_str())?;
                Ok(true)
            }
            ReplCommand::TaskRunNext => {
                let mut latest_report = None;
                let rendered = {
                    let runtime = self.ensure_task_runtime(engine);
                    match runtime.driver.drive_next(&mut runtime.coordinator) {
                        Ok(Some(report)) => {
                            latest_report = Some(report.clone());
                            task_panel::render_task_drive_report(&report)
                        }
                        Ok(None) => String::from("task drive: idle"),
                        Err(error) => task_panel::render_task_drive_error(&error),
                    }
                };
                if let Some(report) = latest_report.as_ref() {
                    self.record_task_drive_report(report);
                }
                self.write_line(writer, rendered.as_str())?;
                Ok(true)
            }
            ReplCommand::TaskRunAll => {
                let mut latest_report = None;
                let rendered = {
                    let runtime = self.ensure_task_runtime(engine);
                    match runtime.driver.drive_all(&mut runtime.coordinator) {
                        Ok(reports) => {
                            latest_report = reports.last().cloned();
                            task_panel::render_task_drive_reports(reports.as_slice())
                        }
                        Err(error) => task_panel::render_task_drive_error(&error),
                    }
                };
                if let Some(report) = latest_report.as_ref() {
                    self.record_task_drive_report(report);
                }
                self.write_line(writer, rendered.as_str())?;
                Ok(true)
            }
            ReplCommand::TaskStop(raw_id) => {
                let current_task = self.task_panel.inspected_task_id.clone();
                let active_filter = self.task_panel.active_filter.clone();
                let rendered = {
                    let runtime = self.ensure_task_runtime(engine);
                    match task_panel::resolve_task_id(
                        &runtime.coordinator,
                        raw_id.as_str(),
                        current_task.as_ref(),
                        active_filter.as_deref(),
                    ) {
                        Some(task_id) => match stop_task(&mut runtime.coordinator, &task_id) {
                            Ok(result) => task_panel::render_task_stop_result(&result),
                            Err(error) => {
                                task_panel::render_task_stop_error(raw_id.as_str(), &error)
                            }
                        },
                        None => String::from("task stop: task not found"),
                    }
                };
                self.write_line(writer, rendered.as_str())?;
                Ok(true)
            }
            ReplCommand::Print(message) => {
                self.write_line(writer, message.as_str())?;
                Ok(true)
            }
            ReplCommand::GitCommit(message) => {
                let escaped = message.replace('"', "\\\"");
                let command = format!("git add -A && git commit -m \"{escaped}\"");
                let rendered = self.spawn_shell_task(engine, &command);
                self.write_line(writer, rendered.as_str())?;
                Ok(true)
            }
            ReplCommand::GitDiff(args) => {
                let command = match args {
                    Some(a) => format!("git diff {a}"),
                    None => String::from("git diff"),
                };
                let rendered = self.spawn_shell_task(engine, &command);
                self.write_line(writer, rendered.as_str())?;
                Ok(true)
            }
            ReplCommand::GitBranch(args) => {
                let command = match args {
                    Some(a) => format!("git branch {a}"),
                    None => String::from("git branch"),
                };
                let rendered = self.spawn_shell_task(engine, &command);
                self.write_line(writer, rendered.as_str())?;
                Ok(true)
            }
            ReplCommand::Login(key) => {
                let result = handle_login(key.as_deref());
                self.write_line(writer, result.as_str())?;
                Ok(true)
            }
            ReplCommand::Logout => {
                let result = handle_logout();
                self.write_line(writer, result.as_str())?;
                Ok(true)
            }
            ReplCommand::Doctor => {
                let result = run_doctor(engine);
                self.write_line(writer, result.as_str())?;
                Ok(true)
            }
            ReplCommand::Ide => {
                self.write_line(
                    writer,
                    "ide integration: --ide-server mode available for VS Code/JetBrains.\n\
                     run: nocode --ide-server to start JSON-RPC stdio server.\n\
                     status: stub — protocol not yet implemented.",
                )?;
                Ok(true)
            }
            ReplCommand::Plugin(args) => {
                let result = handle_plugin_command(args.as_deref());
                self.write_line(writer, result.as_str())?;
                Ok(true)
            }
            ReplCommand::TeamCreate(description) => {
                let result = self.handle_team_create(engine, &description);
                self.write_line(writer, result.as_str())?;
                Ok(true)
            }
            ReplCommand::TeamStatus => {
                let result = self.handle_team_status();
                self.write_line(writer, result.as_str())?;
                Ok(true)
            }
        }
    }

    fn render_submission(&mut self, plan: &QuerySubmissionPlan) -> String {
        let new_lines = if self.session_view.lines().is_empty() {
            let mut seeded = self
                .session_view
                .append_entries(plan.transcript.entries.as_slice());
            if let Some(response_result_line) = response_result_session_line(plan) {
                seeded.extend(self.session_view.append_lines(vec![response_result_line]));
            }
            seeded
        } else {
            self.session_view
                .append_lines(self.collect_submission_lines(plan))
        };
        if new_lines.is_empty() {
            render_plan_output(plan)
        } else {
            SessionView::render_lines(new_lines.as_slice())
        }
    }

    fn collect_submission_lines(&self, plan: &QuerySubmissionPlan) -> Vec<SessionViewLine> {
        let seeded_len = plan.loop_params.system_prompt.len() + plan.loop_params.messages.len();
        let mut lines = Vec::new();
        if let Some(prompt_entry) = seeded_len
            .checked_sub(1)
            .and_then(|index| plan.transcript.entries.get(index))
        {
            lines.push(SessionViewLine::from_entry(prompt_entry));
        }
        lines.extend(
            plan.transcript
                .entries
                .iter()
                .skip(seeded_len)
                .map(SessionViewLine::from_entry),
        );
        if let Some(response_result_line) = response_result_session_line(plan) {
            lines.push(response_result_line);
        }
        lines
    }

    fn render_queue(&self) -> String {
        if self.queued_commands.is_empty() {
            return String::from("queued commands: none");
        }
        self.queued_commands
            .iter()
            .map(|command| {
                format!(
                    "[{}:{}] {}",
                    command.origin.as_str(),
                    command.mode.as_str(),
                    command.raw_input
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn render_draft(&self) -> String {
        self.editor.draft().map_or_else(
            || String::from("draft pending"),
            |draft| format!("draft: {draft}"),
        )
    }

    fn navigate_history(&mut self, direction: i8) -> String {
        let prompt_records = self.history.prompt_records();
        if prompt_records.is_empty() {
            return String::from("prompt history pending / run a prompt first");
        }

        if direction < 0 {
            self.editor.stash_current_draft();
            let next_cursor = self
                .editor
                .history_cursor
                .map_or(prompt_records.len().saturating_sub(1), |cursor| {
                    cursor.saturating_sub(1)
                });
            let record = prompt_records[next_cursor];
            self.editor
                .set_from_history(Some(next_cursor), record.value.clone());
            self.render_history_record(record, next_cursor, prompt_records.len())
        } else if let Some(cursor) = self.editor.history_cursor {
            if cursor + 1 < prompt_records.len() {
                let next_cursor = cursor + 1;
                let record = prompt_records[next_cursor];
                self.editor
                    .set_from_history(Some(next_cursor), record.value.clone());
                self.render_history_record(record, next_cursor, prompt_records.len())
            } else if self.editor.restore_stashed_draft() {
                self.render_draft()
            } else {
                self.editor.clear();
                String::from("draft cleared")
            }
        } else {
            self.render_draft()
        }
    }

    fn render_history_record(
        &self,
        record: &ReplInputRecord,
        cursor: usize,
        total: usize,
    ) -> String {
        format!(
            "draft[{}/{} {}]: {}",
            cursor + 1,
            total,
            record.origin.as_str(),
            record.value
        )
    }

    fn ensure_task_runtime(&mut self, engine: &QueryEngine) -> &mut ReplTaskRuntime {
        self.task_runtime
            .get_or_insert_with(|| ReplTaskRuntime::new(engine.config().clone()))
    }

    /// Spawn a shell task and return the rendered output. Used by /commit, /diff, /branch.
    fn spawn_shell_task(&mut self, engine: &mut QueryEngine, command: &str) -> String {
        let runtime = self.ensure_task_runtime(engine);
        let task_id = runtime
            .coordinator
            .spawn_local_shell(command.to_string(), None, None);
        let rendered =
            task_panel::render_task_spawned(task_id.clone(), TaskType::LocalShell, command);
        self.task_panel.selected_task_id = Some(task_id.clone());
        self.task_panel.inspected_task_id = Some(task_id);
        self.task_panel.focus = ReplPaneFocus::TaskDetail;
        rendered
    }

    /// Create a team of parallel agents working on subtasks.
    fn handle_team_create(&mut self, engine: &mut QueryEngine, description: &str) -> String {
        let runtime = self.ensure_task_runtime(engine);

        // Split description into subtasks by newline or semicolon.
        let subtasks: Vec<&str> = description
            .split([';', '\n'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();

        if subtasks.is_empty() {
            return String::from("no subtasks found — separate with ; or newlines");
        }

        let mut lines = vec![format!("team-create: spawning {} agents", subtasks.len())];
        let mut task_ids = Vec::new();

        for (i, subtask) in subtasks.iter().enumerate() {
            let agent_id = format!("team-{}", i + 1);
            let task_id = runtime
                .coordinator
                .spawn_local_agent(agent_id.clone(), (*subtask).to_string());
            lines.push(format!(
                "  [{id}] agent={agent_id} task={subtask}",
                id = task_id.as_str()
            ));
            task_ids.push(task_id);
        }

        if let Some(first) = task_ids.first() {
            self.task_panel.selected_task_id = Some(first.clone());
            self.task_panel.inspected_task_id = Some(first.clone());
        }
        self.task_panel.focus = ReplPaneFocus::TaskList;

        lines.push(format!(
            "team ready: {} agents queued — use /tasks to monitor",
            task_ids.len()
        ));
        lines.join("\n")
    }

    /// Show status of all team agent tasks.
    fn handle_team_status(&self) -> String {
        let Some(runtime) = &self.task_runtime else {
            return String::from("no task runtime — spawn a team first");
        };
        let tasks = runtime.coordinator.list_tasks();
        let agents: Vec<_> = tasks
            .iter()
            .filter(|t| t.base.task_type == TaskType::LocalAgent)
            .collect();
        if agents.is_empty() {
            return String::from("no agent tasks found");
        }
        let mut lines = vec![format!("team status: {} agents", agents.len())];
        for task in &agents {
            lines.push(format!(
                "  [{}] status={:?} type={:?}",
                task.base.id.as_str(),
                task.base.status,
                task.base.task_type,
            ));
        }
        let completed = agents
            .iter()
            .filter(|t| t.base.status.is_terminal())
            .count();
        lines.push(format!("progress: {completed}/{} complete", agents.len()));
        lines.join("\n")
    }

    fn update_task_filter(&mut self, filter: Option<String>) {
        if filter.is_some() {
            self.task_panel.active_filter = task_panel::normalize_task_filter(filter);
        }
    }

    fn task_focus_label(&self) -> &'static str {
        self.task_panel.focus.as_str()
    }

    fn render_transcript_panel(&self) -> String {
        format!(
            "transcript pane: focus={}\n{}",
            ReplPaneFocus::Transcript.as_str(),
            self.render_transcript_body()
        )
    }

    fn render_tui_status_line(&self) -> String {
        let plan = self
            .last_plan_summary
            .as_deref()
            .unwrap_or("status summary pending / run a prompt first");
        if let Some(runtime) = &self.task_runtime {
            format!("{plan} | {}", runtime.render_runtime_status_line())
        } else {
            plan.to_string()
        }
    }

    #[allow(dead_code)]
    fn render_tui_hud_line(&self) -> String {
        if self.is_streaming() {
            self.status_hud.render_line_streaming()
        } else {
            self.status_hud.render_line()
        }
    }

    /// Overwrite HUD turn/cumulative tokens with real values from the plan,
    /// update model name and session ID, then end the turn timer.
    fn finalize_hud_from_plan(&mut self, plan: &QuerySubmissionPlan) {
        if let Some(inv) = plan.model_invocation.as_ref() {
            self.status_hud.model_name = inv.model.clone();
        }
        self.status_hud.session_id = plan.session_persistence.session_id.clone();
        let snap = &plan.usage_snapshot;
        self.status_hud.turn_input_tokens = snap.input_tokens;
        self.status_hud.turn_output_tokens = snap.output_tokens;
        self.status_hud.cumulative_input_tokens = snap.total_usage.input_tokens;
        self.status_hud.cumulative_output_tokens = snap.total_usage.output_tokens;
        self.status_hud.end_turn();
    }

    fn render_tui_diagnostics_line(&self) -> String {
        let diagnostics = self
            .last_plan_diagnostics
            .as_deref()
            .map(normalize_session_content)
            .unwrap_or_else(|| String::from("provider diagnostics pending / run a prompt first"));
        let filter = self.task_panel.active_filter.as_deref().unwrap_or("all");
        let selected = self
            .task_panel
            .selected_task_id
            .as_ref()
            .map(|task_id| task_id.as_str())
            .unwrap_or("none");
        let detail = self
            .task_panel
            .inspected_task_id
            .as_ref()
            .map(|task_id| task_id.as_str())
            .unwrap_or("none");
        format!("diag={diagnostics} | filter={filter} | selected={selected} | detail={detail}")
    }

    fn render_tui_editor_line(&self) -> String {
        let prompt_total = self.history.prompt_records().len();
        let history = self.editor.history_cursor.map_or_else(
            || {
                if prompt_total == 0 {
                    String::from("history=empty")
                } else {
                    format!("history=ready/{prompt_total}")
                }
            },
            |cursor| format!("history={}/{}", cursor + 1, prompt_total),
        );
        let draft = self.editor.draft().map_or_else(
            || String::from("draft=pending"),
            |draft| format!("draft={}", normalize_session_content(draft)),
        );
        format!(
            "editor focus={} | {} | {}",
            self.focus_label(),
            history,
            draft
        )
    }

    fn render_tui_queue_line(&self) -> String {
        if self.queued_commands.is_empty() {
            return String::from("queue=empty");
        }
        let preview = self
            .queued_commands
            .iter()
            .take(3)
            .map(|command| {
                format!(
                    "[{}:{}] {}",
                    command.origin.as_str(),
                    command.mode.as_str(),
                    normalize_session_content(command.raw_input.as_str())
                )
            })
            .collect::<Vec<_>>()
            .join(" | ");
        format!("queue={} next={preview}", self.queued_commands.len())
    }

    fn render_tui_footer_line(&self) -> String {
        String::from(
            "keys Tab/Shift-Tab panes | 1-4 focus | Enter submit | f detail/events toggle | ? help | F2 inspect | F3 permissions | Ctrl-P/N history | Esc close/quit",
        )
    }

    fn render_tui_transcript_body(&self) -> String {
        let prompt_origins = self
            .history
            .prompt_records()
            .into_iter()
            .map(|record| record.origin)
            .collect::<Vec<_>>();
        let mut sections = vec![
            self.session_view
                .render_tui_timeline(prompt_origins.as_slice()),
        ];
        if !self.last_plan_stream_lines.is_empty() {
            sections.push(String::from("latest stream:"));
            sections.extend(
                self.last_plan_stream_lines
                    .iter()
                    .map(|line| format!("  {line}")),
            );
        }
        if let Some(pretty) = self.last_plan_response_result_pretty.as_deref() {
            sections.push(String::from("latest response result:"));
            sections.extend(pretty.lines().map(|line| format!("  {line}")));
        }
        sections.join("\n\n")
    }

    fn render_transcript_body(&self) -> String {
        let status = self
            .last_plan_summary
            .as_deref()
            .unwrap_or("status summary pending / run a prompt first");
        let diagnostics = self
            .last_plan_diagnostics
            .as_deref()
            .unwrap_or("provider diagnostics pending / run a prompt first");
        let mut sections = vec![status.to_string(), diagnostics.to_string()];
        if !self.last_plan_stream_lines.is_empty() {
            sections.push(String::from("latest stream:"));
            sections.extend(
                self.last_plan_stream_lines
                    .iter()
                    .map(|line| format!("  {line}")),
            );
        }
        sections.push(self.session_view.render_scrollback());
        sections.join("\n")
    }

    fn build_task_panel_view(&self) -> task_panel::TaskPanelView {
        if let Some(runtime) = &self.task_runtime {
            task_panel::TaskPanelView::from_state(
                &runtime.coordinator,
                self.task_panel.active_filter.as_deref(),
                self.task_panel.selected_task_id.as_ref(),
                self.task_panel.inspected_task_id.as_ref(),
            )
        } else {
            let coordinator = TaskCoordinator::new();
            task_panel::TaskPanelView::from_state(
                &coordinator,
                self.task_panel.active_filter.as_deref(),
                self.task_panel.selected_task_id.as_ref(),
                self.task_panel.inspected_task_id.as_ref(),
            )
        }
    }

    fn render_task_panel_state(
        coordinator: &TaskCoordinator,
        filter: Option<&str>,
        selected_id: Option<&TaskId>,
        detail_id: Option<&TaskId>,
        focus: ReplPaneFocus,
        runtime_footer: Option<&str>,
    ) -> String {
        let rendered =
            task_panel::TaskPanelView::from_state(coordinator, filter, selected_id, detail_id)
                .render_layout_with_focus(focus.as_str());
        runtime_footer.map_or(rendered.clone(), |footer| format!("{rendered}\n{footer}"))
    }

    fn append_runtime_footer_if_present(&self, rendered: String) -> String {
        if let Some(runtime) = &self.task_runtime {
            runtime.append_runtime_footer(rendered)
        } else {
            rendered
        }
    }

    fn maybe_auto_drive_tasks<W: Write>(
        &mut self,
        _engine: &QueryEngine,
        writer: &mut W,
    ) -> io::Result<()> {
        let active_filter = self.task_panel.active_filter.clone();
        let current_selected = self.task_panel.selected_task_id.clone();
        let current_detail = self.task_panel.inspected_task_id.clone();
        let focus_label = self.task_focus_label();
        let mut next_selected = current_selected.clone();
        let mut latest_report = None;
        let rendered = match self.task_runtime.as_mut() {
            Some(runtime) if runtime.coordinator.pending_count() > 0 => {
                next_selected = task_panel::align_selected_task_id(
                    &runtime.coordinator,
                    active_filter.as_deref(),
                    current_selected.as_ref(),
                );
                let max_iterations = runtime.coordinator.pending_count().saturating_mul(8).max(1);
                match runtime
                    .driver
                    .drive_until_idle(&mut runtime.coordinator, max_iterations)
                {
                    Ok(reports) if reports.is_empty() => None,
                    Ok(reports) => {
                        latest_report = reports.last().cloned();
                        Some(
                            runtime.append_runtime_footer(task_panel::render_task_auto_drive(
                                reports.as_slice(),
                                &runtime.coordinator,
                                active_filter.as_deref(),
                                next_selected.as_ref().or(current_selected.as_ref()),
                                current_detail.as_ref(),
                                focus_label,
                            )),
                        )
                    }
                    Err(error) => Some(task_panel::render_task_drive_error(&error)),
                }
            }
            _ => None,
        };
        self.task_panel.selected_task_id = next_selected;
        if let Some(report) = latest_report.as_ref() {
            self.record_task_drive_report(report);
        }

        if let Some(rendered) = rendered {
            self.write_line(writer, rendered.as_str())?;
        }
        Ok(())
    }

    fn maybe_tick_task_updates<W: Write>(
        &mut self,
        _engine: &QueryEngine,
        writer: &mut W,
    ) -> io::Result<()> {
        let active_filter = self.task_panel.active_filter.clone();
        let current_selected = self.task_panel.selected_task_id.clone();
        let mut next_selected = current_selected.clone();
        let mut latest_report = None;
        let rendered =
            match self.task_runtime.as_mut() {
                Some(runtime) if runtime.coordinator.pending_count() > 0 => {
                    next_selected = task_panel::align_selected_task_id(
                        &runtime.coordinator,
                        active_filter.as_deref(),
                        current_selected.as_ref(),
                    );
                    match runtime.driver.drive_next(&mut runtime.coordinator) {
                        Ok(Some(report)) => {
                            latest_report = Some(report.clone());
                            Some(runtime.append_runtime_footer(
                                task_panel::render_task_drive_report(&report),
                            ))
                        }
                        Ok(None) => None,
                        Err(error) => Some(task_panel::render_task_drive_error(&error)),
                    }
                }
                _ => None,
            };
        self.task_panel.selected_task_id = next_selected;
        if let Some(report) = latest_report.as_ref() {
            self.record_task_drive_report(report);
        }

        if let Some(rendered) = rendered {
            self.write_line(writer, rendered.as_str())?;
        }
        Ok(())
    }

    fn record_task_drive_report(&mut self, report: &TaskDriveReport) {
        self.task_panel.last_drive_report = Some(report.clone());
        let Some(_) = report.activity.as_ref() else {
            return;
        };
        let history = self
            .task_panel
            .activity_history
            .entry(report.task_id.clone())
            .or_default();
        history.push_back(report.clone());
        if history.len() > TASK_ACTIVITY_HISTORY_LIMIT {
            let overflow = history.len() - TASK_ACTIVITY_HISTORY_LIMIT;
            history.drain(0..overflow);
        }
    }

    fn write_line<W: Write>(&self, writer: &mut W, value: &str) -> io::Result<()> {
        writer.write_all(value.as_bytes())?;
        writer.write_all(b"\n")?;
        writer.flush()
    }
}

fn render_help() -> String {
    [
        "commands:",
        "/help",
        "/status",
        "/runtime",
        "/history",
        "/inputs",
        "/tasks [filter]",
        "/focus <transcript|tasks|detail>",
        "/tasks-next",
        "/tasks-prev",
        "/j",
        "/k",
        "/enter",
        "/task-queue",
        "/task-shell <command>",
        "/task-agent <agent-id> <prompt>",
        "/task-dream [sessions] [description]",
        "/task-show <task-id|first|last|latest|prev|next>",
        "/task-open",
        "/task-run-next",
        "/task-run-all",
        "/task-stop <task-id>",
        "/draft <text>",
        "/edit <text>",
        "/append <text>",
        "/send",
        "/history-prev",
        "/history-next",
        "/queue <prompt>",
        "/queue-slash </command>",
        "/queue-show",
        "/quit",
    ]
    .join("\n")
}

fn parse_task_agent_args(args: &str) -> Result<(String, String), String> {
    let mut parts = args.trim().splitn(2, char::is_whitespace);
    let agent_id = parts.next().unwrap_or_default().trim();
    let prompt = parts.next().unwrap_or_default().trim();
    if agent_id.is_empty() || prompt.is_empty() {
        Err(String::from("usage: /task-agent <agent-id> <prompt>"))
    } else {
        Ok((agent_id.to_string(), prompt.to_string()))
    }
}

fn parse_task_dream_args(args: &str) -> Result<(usize, Option<String>), String> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return Ok((0, None));
    }
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let sessions = parts.next().unwrap_or_default();
    let sessions_reviewing = sessions
        .parse::<usize>()
        .map_err(|_| String::from("usage: /task-dream [sessions] [description]"))?;
    let description = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    Ok((sessions_reviewing, description))
}

/// Pushes the intent into `QueryEngine` so callers can keep intent creation separate from execution.
#[allow(dead_code)]
pub fn submit_intent(engine: &mut QueryEngine, intent: ReplIntent) -> QuerySubmissionPlan {
    engine.submit_message(intent.prompt, intent.options)
}

fn submit_intent_with_stream(
    engine: &mut QueryEngine,
    intent: ReplIntent,
    stream: &mut dyn ModelStreamSink,
) -> QuerySubmissionPlan {
    engine.submit_message_with_stream(intent.prompt, intent.options, stream)
}

struct ReplLiveStreamCapture<'a, W: Write> {
    writer: &'a mut W,
    lines: Vec<String>,
    write_error: Option<String>,
}

impl<'a, W: Write> ReplLiveStreamCapture<'a, W> {
    fn new(writer: &'a mut W) -> Self {
        Self {
            writer,
            lines: Vec::new(),
            write_error: None,
        }
    }

    fn finish(self) -> io::Result<Vec<String>> {
        if let Some(message) = self.write_error {
            Err(io::Error::other(message))
        } else {
            Ok(self.lines)
        }
    }

    fn push_line(&mut self, line: String) {
        self.lines.push(line.clone());
        if self.write_error.is_some() {
            return;
        }
        if let Err(error) = self.writer.write_all(format!("{line}\n").as_bytes()) {
            self.write_error = Some(error.to_string());
            return;
        }
        if let Err(error) = self.writer.flush() {
            self.write_error = Some(error.to_string());
        }
    }
}

impl<W: Write> ModelStreamSink for ReplLiveStreamCapture<'_, W> {
    fn push(&mut self, event: ModelStreamEvent) {
        let line = match event {
            ModelStreamEvent::Start { provider, model } => {
                format!(
                    "stream start: provider={} model={}",
                    provider.as_str(),
                    model
                )
            }
            ModelStreamEvent::Delta { text, .. } => {
                format!("stream delta: {}", normalize_session_content(text.as_str()))
            }
            ModelStreamEvent::Complete { message } => format!(
                "stream complete: {}",
                normalize_session_content(message.content.as_str())
            ),
            ModelStreamEvent::StreamError { message, .. } => {
                format!(
                    "stream error: {}",
                    normalize_session_content(message.as_str())
                )
            }
        };
        self.push_line(line);
    }
}

fn render_plan_output(plan: &QuerySubmissionPlan) -> String {
    plan.model_response
        .final_assistant_message
        .as_ref()
        .map(|message| message.content.clone())
        .unwrap_or_else(|| {
            format!(
                "turn-finished: {}",
                plan.model_response.stop_reason.as_str()
            )
        })
}

fn response_result_session_line(plan: &QuerySubmissionPlan) -> Option<SessionViewLine> {
    plan.response_result
        .as_ref()
        .map(|response_result| SessionViewLine {
            turn: plan
                .transcript
                .entries
                .last()
                .map(|entry| entry.turn)
                .unwrap_or(plan.assistant_turn.sequence),
            role: String::from("response-result"),
            content: normalize_session_content(format!("result={response_result}").as_str()),
        })
}

fn render_plan_status(plan: &QuerySubmissionPlan) -> String {
    let invocation = plan.model_invocation.as_ref();
    let selected_provider = invocation
        .map(|inv| inv.provider)
        .unwrap_or(plan.query_config.model_selection.provider);
    let provider = invocation
        .map(|inv| inv.provider.as_str())
        .unwrap_or(selected_provider.as_str());
    let model = invocation.map(|inv| inv.model.as_str()).unwrap_or("none");
    let transport = invocation
        .map(|inv| inv.transport_request.url.as_str())
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
        "status summary: provider={} caps={} matrix={} model={} transport={} stream={} tools={} turn-count={} response-result={} error={}",
        provider,
        capabilities,
        capability_matrix,
        model,
        transport,
        stream_summary,
        tools,
        turn_count,
        plan.response_result_preview(96),
        error_summary
    )
}

fn render_plan_diagnostics(plan: &QuerySubmissionPlan) -> String {
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
        .unwrap_or(("none", String::from("none")));
    let headers = invocation
        .map(|inv| inv.transport_request.headers.len())
        .unwrap_or(0);
    let body_preview = invocation
        .and_then(|inv| inv.transport_request.body.as_deref())
        .map(normalize_session_content)
        .unwrap_or_else(|| String::from("none"));
    let stream_summary = invocation
        .map(|inv| inv.stream_summary())
        .unwrap_or_else(|| String::from("total=0 delta=0 chars=0 start=no complete=no"));
    let tools = plan.tool_results.len();
    let turn_count = match &plan.loop_outcome {
        QueryLoopOutcome::Continue(state) => state.turn_count,
        QueryLoopOutcome::Terminal(_) => 0,
    };
    let terminal = match &plan.loop_outcome {
        QueryLoopOutcome::Continue(state) => render_continue_reason(state.transition),
        QueryLoopOutcome::Terminal(reason) => render_terminal_reason(reason),
    };
    let model_error = plan
        .model_error
        .as_ref()
        .map(render_model_error)
        .unwrap_or_else(|| String::from("none"));
    let mut lines = vec![
        String::from("provider diagnostics:"),
        format!(
            "provider={} model={} caps={}",
            provider,
            model,
            selected_provider.capability_summary()
        ),
        format!(
            "capability-matrix={}",
            ModelProvider::capability_matrix_summary()
        ),
        format!(
            "transport={}({}) headers={} body={}",
            transport_url, transport_method, headers, body_preview
        ),
        format!(
            "stream={} tools={} turn-count={} terminal={}",
            stream_summary, tools, turn_count, terminal
        ),
        format!("response-result={}", plan.response_result_preview(240)),
        format!("model-error: {model_error}"),
    ];
    if let Some(pretty) = plan.response_result_pretty() {
        lines.push(format!("response-result.pretty:\n{pretty}"));
    }
    lines.join("\n")
}

fn render_continue_reason(reason: Option<QueryLoopContinueReason>) -> String {
    reason
        .map(|value| format!("continue({})", value.as_str()))
        .unwrap_or_else(|| String::from("continue(pending)"))
}

fn render_terminal_reason(reason: &QueryLoopTerminal) -> String {
    match reason {
        QueryLoopTerminal::ModelError { error } => {
            format!("model_error({})", render_model_error(error))
        }
        QueryLoopTerminal::MaxTurns { turn_count } => format!("max_turns({turn_count})"),
        QueryLoopTerminal::Completed => String::from("completed"),
        QueryLoopTerminal::BlockingLimit => String::from("blocking_limit"),
        QueryLoopTerminal::ImageError => String::from("image_error"),
        QueryLoopTerminal::AbortedStreaming => String::from("aborted_streaming"),
        QueryLoopTerminal::PromptTooLong => String::from("prompt_too_long"),
        QueryLoopTerminal::StopHookPrevented => String::from("stop_hook_prevented"),
        QueryLoopTerminal::AbortedTools => String::from("aborted_tools"),
        QueryLoopTerminal::HookStopped => String::from("hook_stopped"),
    }
}

fn render_model_error(error: &nocode_core::ModelError) -> String {
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

fn render_task_status_label(status: nocode_core::TaskStatus) -> &'static str {
    match status {
        nocode_core::TaskStatus::Pending => "pending",
        nocode_core::TaskStatus::Running => "running",
        nocode_core::TaskStatus::Completed => "completed",
        nocode_core::TaskStatus::Failed => "failed",
        nocode_core::TaskStatus::Killed => "killed",
    }
}

fn render_task_activity_history_line(index: usize, report: &TaskDriveReport) -> String {
    format!(
        "[{}] task={} status={} {}",
        index + 1,
        report.task_id.as_str(),
        render_task_status_label(report.status),
        report.activity.as_deref().unwrap_or("none")
    )
}

// ---------------------------------------------------------------------------
// P2: Auth login/logout
// ---------------------------------------------------------------------------

fn credentials_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| String::from("/tmp"));
    std::path::Path::new(&home).join(".nocode/credentials")
}

fn handle_login(key: Option<&str>) -> String {
    let api_key = match key {
        Some(k) => k.to_string(),
        None => {
            return String::from(
                "usage: /login <api-key>\n\
                 or set ANTHROPIC_API_KEY / OPENAI_API_KEY environment variable",
            );
        }
    };
    let path = credentials_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::write(&path, format!("ANTHROPIC_API_KEY={api_key}\n")) {
        Ok(()) => format!("credentials saved to {}", path.display()),
        Err(e) => format!("failed to save credentials: {e}"),
    }
}

fn handle_logout() -> String {
    let path = credentials_path();
    if path.exists() {
        match std::fs::remove_file(&path) {
            Ok(()) => format!("credentials removed from {}", path.display()),
            Err(e) => format!("failed to remove credentials: {e}"),
        }
    } else {
        String::from("no credentials file found")
    }
}

// ---------------------------------------------------------------------------
// P2: Doctor diagnostics
// ---------------------------------------------------------------------------

fn run_doctor(engine: &QueryEngine) -> String {
    let mut lines = vec![String::from("nocode doctor")];
    lines.push(String::from("---"));

    // Rust toolchain
    lines.push(format!(
        "platform: {} / {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    ));

    // Provider
    let provider = engine.config().model_provider;
    let model = engine
        .config()
        .user_specified_model
        .as_deref()
        .unwrap_or("(default)");
    lines.push(format!("provider: {} model: {}", provider.as_str(), model));

    // API key status
    let has_anthropic = std::env::var("ANTHROPIC_API_KEY").is_ok();
    let has_openai = std::env::var("OPENAI_API_KEY").is_ok();
    lines.push(format!(
        "api keys: anthropic={} openai={}",
        if has_anthropic { "set" } else { "missing" },
        if has_openai { "set" } else { "missing" },
    ));

    // Credentials file
    let creds = credentials_path();
    lines.push(format!(
        "credentials: {}",
        if creds.exists() {
            creds.to_string_lossy().to_string()
        } else {
            String::from("none")
        }
    ));

    // Tools
    let tools = &engine.config().tools;
    lines.push(format!(
        "tools: {} registered ({})",
        tools.len(),
        tools.join(", ")
    ));

    // Git
    let git_ok = std::process::Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    lines.push(format!("git: {}", if git_ok { "ok" } else { "not found" }));

    // curl
    let curl_ok = std::process::Command::new("curl")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    lines.push(format!(
        "curl: {}",
        if curl_ok { "ok" } else { "not found" }
    ));

    // CLAUDE.md
    let cwd = std::path::Path::new(&engine.config().cwd);
    let claude_md = cwd.join("CLAUDE.md").exists() || cwd.join(".claude/CLAUDE.md").exists();
    lines.push(format!(
        "CLAUDE.md: {}",
        if claude_md { "found" } else { "not found" }
    ));

    lines.push(String::from("---"));
    if has_anthropic || has_openai {
        lines.push(String::from("status: ready"));
    } else {
        lines.push(String::from(
            "status: no API key — set ANTHROPIC_API_KEY or run /login <key>",
        ));
    }

    lines.join("\n")
}

// ---------------------------------------------------------------------------
// P2: Plugin system skeleton
// ---------------------------------------------------------------------------

fn handle_plugin_command(args: Option<&str>) -> String {
    let plugins_dir = plugin_discovery_path();
    match args {
        None | Some("list") => {
            if !plugins_dir.is_dir() {
                return format!(
                    "no plugins directory found.\n\
                     create {} to get started.",
                    plugins_dir.display()
                );
            }
            let entries: Vec<String> = std::fs::read_dir(&plugins_dir)
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .filter(|e| e.path().is_dir())
                .filter(|e| e.path().join("manifest.json").exists())
                .map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    let manifest = e.path().join("manifest.json");
                    let desc = std::fs::read_to_string(&manifest)
                        .ok()
                        .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
                        .and_then(|v| {
                            v.get("description")
                                .and_then(|d| d.as_str().map(String::from))
                        })
                        .unwrap_or_default();
                    if desc.is_empty() {
                        name
                    } else {
                        format!("{name} — {desc}")
                    }
                })
                .collect();
            if entries.is_empty() {
                format!(
                    "no plugins found in {}.\n\
                     each plugin is a directory with manifest.json.",
                    plugins_dir.display()
                )
            } else {
                format!("plugins ({}):\n{}", entries.len(), entries.join("\n"))
            }
        }
        Some(other) => format!(
            "unknown plugin command: {other}\n\
             usage: /plugin [list]"
        ),
    }
}

fn plugin_discovery_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| String::from("/tmp"));
    std::path::Path::new(&home).join(".nocode/plugins")
}

#[cfg(test)]
mod tests {
    use super::{
        ReplCommand, ReplInputMode, ReplInputOrigin, ReplIntent, ReplSession, ReplTaskDriver,
        ReplTaskRuntime, TASK_ACTIVITY_HISTORY_LIMIT, render_help, render_runtime_report,
    };
    use nocode_core::{
        CallModel, DefaultDreamHost, LiveTaskRuntimeDriver, LiveTaskShellHost, ModelCallOutput,
        ModelError, ModelProvider, ModelRequest, ModelStreamSink, ProcessAgentBackoffProfile,
        ProcessAgentBackoffStrategy, ProcessTaskAgentHost, QueryDeps, QueryEngine,
        QueryEngineConfig, QueryMessage, TaskBudget, TaskDriveError, TaskDriveReport, TaskStatus,
        TaskType, ThinkingMode, ToolPermissionContext, ToolRuntimeMode,
    };
    use serde_json::json;
    use std::collections::HashMap;
    use std::io::Cursor;
    use std::process::Command as OsCommand;

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
                QueryMessage::assistant("{\"ok\":true,\"source\":\"repl\"}"),
            )
            .with_response_result(json!({"ok": true, "source": "repl"})))
        }
    }

    fn process_task_agent_host_with_vars(
        vars: &[(&str, &str)],
    ) -> (ProcessTaskAgentHost, super::ReplTaskRuntimeInfo) {
        let values = vars
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
            .collect::<HashMap<_, _>>();
        super::process_task_agent_host_from_lookup(&test_config(), |name| values.get(name).cloned())
            .expect("process task host should be built")
    }

    fn test_config() -> QueryEngineConfig {
        QueryEngineConfig {
            cwd: String::from("/tmp"),
            session_id: String::from("repl-test"),
            persist_session: false,
            persist_history: false,
            file_history_enabled: false,
            tools: vec![String::from("Read")],
            tool_runtime_mode: ToolRuntimeMode::Standard,
            tool_permission_context: ToolPermissionContext::default(),
            commands: Vec::new(),
            mcp_clients: Vec::new(),
            agents: Vec::new(),
            initial_messages: vec![QueryMessage::system("bootstrap")],
            read_file_cache_entries: 0,
            custom_system_prompt: Some(String::from("system")),
            append_system_prompt: None,
            model_provider: ModelProvider::Mock,
            user_specified_model: Some(String::from("sonnet")),
            fallback_model: Some(String::from("haiku")),
            model_reasoning_effort: None,
            thinking_mode: ThinkingMode::Adaptive,
            max_turns: Some(2),
            max_budget_usd: None,
            task_budget: Some(TaskBudget { total: 1_000 }),
            json_schema: None,
            verbose: false,
            replay_user_messages: false,
            include_partial_messages: false,
            stream_model_responses: true,
        }
    }

    #[test]
    fn prompt_text_and_quit_token_are_stable() {
        let session = ReplSession::new("nocode");
        assert_eq!(session.prompt_text(), "nocode> ");
    }

    #[test]
    fn intent_builder_preserves_prompt() {
        let intent = ReplIntent::new("rewrite");
        assert_eq!(intent.prompt, "rewrite");
    }

    #[test]
    fn intent_builder_accepts_custom_options() {
        let intent = ReplIntent::new("rewrite").with_options(nocode_core::SubmitMessageOptions {
            uuid: Some(String::from("uuid-1")),
            is_meta: true,
        });
        assert_eq!(intent.options.uuid.as_deref(), Some("uuid-1"));
        assert!(intent.options.is_meta);
    }

    #[test]
    fn empty_lines_do_not_quit() {
        let command = ReplCommand::Intent(ReplIntent::new(""));
        assert_eq!(command, ReplCommand::Intent(ReplIntent::new("")));
    }

    #[test]
    fn repl_loop_records_history_and_renders_session_view() {
        let mut engine = QueryEngine::new(test_config());
        let mut session = ReplSession::new("nocode");
        let mut input = Cursor::new(b"hello\n/quit\n");
        let mut output = Vec::new();

        session
            .run_loop(&mut engine, &mut input, &mut output)
            .expect("repl loop should succeed");

        let rendered = String::from_utf8(output).expect("utf8 output");
        assert!(rendered.contains("nocode> "));
        assert!(rendered.contains("[t1:conversation] system: bootstrap"));
        assert!(rendered.contains("[t1:conversation] user: hello"));
        assert!(rendered.contains("nocode response: hello"));
        assert_eq!(
            session.history(),
            &[
                super::ReplInputRecord::new("hello", ReplInputMode::Prompt, ReplInputOrigin::Local,),
                super::ReplInputRecord::new(
                    "/quit",
                    ReplInputMode::SlashCommand,
                    ReplInputOrigin::Local,
                ),
            ]
        );
        assert_eq!(
            session.history.last().map(|entry| entry.value.as_str()),
            Some("/quit")
        );
        assert!(!session.session_view().is_empty());
    }

    #[test]
    fn repl_loop_surfaces_live_stream_events_before_final_transcript() {
        let mut engine = QueryEngine::new(test_config());
        let mut session = ReplSession::new("nocode");
        let mut input = Cursor::new(b"hello\n/quit\n");
        let mut output = Vec::new();

        session
            .run_loop(&mut engine, &mut input, &mut output)
            .expect("repl loop should succeed");

        let rendered = String::from_utf8(output).expect("utf8 output");
        let start_index = rendered
            .find("stream start: provider=mock model=sonnet")
            .expect("start event should render");
        let delta_index = rendered
            .find("stream delta: nocode response: hello")
            .expect("delta event should render");
        let complete_index = rendered
            .find("stream complete: nocode response: hello")
            .expect("complete event should render");
        let final_index = rendered
            .find("[t2:conversation] assistant: nocode response: hello")
            .expect("final transcript should render");
        assert!(start_index < final_index);
        assert!(delta_index < final_index);
        assert!(complete_index < final_index);

        let snapshot = session.render_tui_snapshot();
        assert!(snapshot.transcript.contains("latest stream:"));
        assert!(
            snapshot
                .transcript
                .contains("stream start: provider=mock model=sonnet")
        );
        assert!(
            snapshot
                .transcript
                .contains("stream delta: nocode response: hello")
        );
    }

    #[test]
    fn status_command_reports_summary() {
        let mut engine = QueryEngine::new(test_config());
        let mut session = ReplSession::new("nocode");
        let mut input = Cursor::new(b"hello\n/status\n/quit\n");
        let mut output = Vec::new();

        session
            .run_loop(&mut engine, &mut input, &mut output)
            .expect("repl loop should succeed");

        let rendered = String::from_utf8(output).expect("utf8 output");
        assert!(rendered.contains("status summary:"));
        assert!(rendered.contains("caps=stream(request=yes,live=no,sse=no)"));
        assert!(rendered.contains("matrix=mock[stream(request=yes,live=no,sse=no)"));
        assert!(rendered.contains("transport="));
        assert!(rendered.contains("stream=total=3 delta=1 chars="));
        assert!(rendered.contains("start=yes complete=yes"));
        assert!(rendered.contains("response-result=none"));
        assert!(rendered.contains("error=none"));
    }

    #[test]
    fn runtime_command_reports_runtime_footer() {
        let mut engine = QueryEngine::new(test_config());
        let mut session = ReplSession::new("nocode");
        let mut input = Cursor::new(b"/runtime\n/quit\n");
        let mut output = Vec::new();

        session
            .run_loop(&mut engine, &mut input, &mut output)
            .expect("repl loop should succeed");

        let rendered = String::from_utf8(output).expect("utf8 output");
        assert!(rendered.contains("runtime report:"));
        assert!(rendered.contains("runtime agent=in-process"));
        assert!(rendered.contains("task runtime:"));
        assert!(rendered.contains("agent=in-process"));
    }

    #[test]
    fn parse_backoff_strategy_accepts_known_values() {
        unsafe {
            std::env::set_var("NOCODE_TEST_BACKOFF_STRATEGY", "linear");
        }
        assert_eq!(
            super::parse_backoff_strategy("NOCODE_TEST_BACKOFF_STRATEGY"),
            Some(ProcessAgentBackoffStrategy::Linear)
        );
        unsafe {
            std::env::set_var("NOCODE_TEST_BACKOFF_STRATEGY", "exponential");
        }
        assert_eq!(
            super::parse_backoff_strategy("NOCODE_TEST_BACKOFF_STRATEGY"),
            Some(ProcessAgentBackoffStrategy::Exponential)
        );
        unsafe {
            std::env::remove_var("NOCODE_TEST_BACKOFF_STRATEGY");
        }
    }

    #[test]
    fn parse_backoff_profile_accepts_compact_override_values() {
        unsafe {
            std::env::set_var("NOCODE_TEST_BACKOFF_PROFILE", "25:exponential:15");
        }
        assert_eq!(
            super::parse_backoff_profile("NOCODE_TEST_BACKOFF_PROFILE"),
            Some(ProcessAgentBackoffProfile {
                base_delay_ms: 25,
                strategy: ProcessAgentBackoffStrategy::Exponential,
                jitter_percent: 15,
            })
        );
        unsafe {
            std::env::remove_var("NOCODE_TEST_BACKOFF_PROFILE");
        }
    }

    #[test]
    fn process_task_agent_host_wires_default_backoff_envs() {
        let (host, info) = process_task_agent_host_with_vars(&[
            ("NOCODE_TASK_AGENT_HOST", "daemon"),
            ("NOCODE_TASK_AGENT_COMMAND", "python3"),
            ("NOCODE_TASK_AGENT_ARGS", "-u -c stub"),
            ("NOCODE_TASK_AGENT_DAEMON_RESTARTS", "4"),
            ("NOCODE_TASK_AGENT_DAEMON_MAX_CONSECUTIVE_FAILURES", "3"),
            ("NOCODE_TASK_AGENT_DAEMON_BACKOFF_STRATEGY", "exponential"),
            ("NOCODE_TASK_AGENT_DAEMON_BACKOFF_JITTER_PERCENT", "12"),
            ("NOCODE_TASK_AGENT_DAEMON_BACKOFF_MS", "40"),
            ("NOCODE_TASK_AGENT_DAEMON_RESTART_ON_IO_ERROR", "false"),
            ("NOCODE_TASK_AGENT_DAEMON_RESTART_ON_DECODE_ERROR", "false"),
            ("NOCODE_TASK_AGENT_DAEMON_RESTART_ON_CLEAN_EXIT", "false"),
        ]);
        let policy = host.supervisor().policy();

        assert_eq!(host.mode_label(), "process-daemon");
        assert_eq!(info.agent_host, "process-daemon");
        assert_eq!(info.command.as_deref(), Some("python3"));
        assert_eq!(info.args, vec!["-u", "-c", "stub"]);
        assert_eq!(policy.restart.max_restart_attempts, 4);
        assert_eq!(policy.restart.max_consecutive_failures, 3);
        assert_eq!(
            policy.backoff.default_profile,
            ProcessAgentBackoffProfile {
                base_delay_ms: 40,
                strategy: ProcessAgentBackoffStrategy::Exponential,
                jitter_percent: 12,
            }
        );
        assert!(!policy.restart.restart_on_io_error);
        assert!(!policy.restart.restart_on_decode_error);
        assert!(!policy.restart.restart_on_clean_exit);
    }

    #[test]
    fn process_task_agent_host_wires_failure_specific_backoff_overrides() {
        let (host, _) = process_task_agent_host_with_vars(&[
            ("NOCODE_TASK_AGENT_HOST", "daemon"),
            ("NOCODE_TASK_AGENT_COMMAND", "python3"),
            ("NOCODE_TASK_AGENT_DAEMON_BACKOFF_STRATEGY", "exponential"),
            ("NOCODE_TASK_AGENT_DAEMON_BACKOFF_JITTER_PERCENT", "15"),
            ("NOCODE_TASK_AGENT_DAEMON_BACKOFF_MS", "50"),
            ("NOCODE_TASK_AGENT_DAEMON_IO_BACKOFF", "25"),
            (
                "NOCODE_TASK_AGENT_DAEMON_DECODE_BACKOFF",
                "80:exponential:0",
            ),
            ("NOCODE_TASK_AGENT_DAEMON_EXIT_BACKOFF", "10::7"),
        ]);
        let backoff = host.supervisor().policy().backoff;

        assert_eq!(
            backoff.default_profile,
            ProcessAgentBackoffProfile {
                base_delay_ms: 50,
                strategy: ProcessAgentBackoffStrategy::Exponential,
                jitter_percent: 15,
            }
        );
        assert_eq!(
            backoff.io_profile,
            Some(ProcessAgentBackoffProfile {
                base_delay_ms: 25,
                strategy: ProcessAgentBackoffStrategy::Linear,
                jitter_percent: 0,
            })
        );
        assert_eq!(
            backoff.decode_profile,
            Some(ProcessAgentBackoffProfile {
                base_delay_ms: 80,
                strategy: ProcessAgentBackoffStrategy::Exponential,
                jitter_percent: 0,
            })
        );
        assert_eq!(
            backoff.exit_profile,
            Some(ProcessAgentBackoffProfile {
                base_delay_ms: 10,
                strategy: ProcessAgentBackoffStrategy::Linear,
                jitter_percent: 7,
            })
        );
    }

    #[test]
    fn history_command_renders_scrollback_view() {
        let mut engine = QueryEngine::new(test_config());
        let mut session = ReplSession::new("nocode");
        let mut input = Cursor::new(b"hello\n/history\n/quit\n");
        let mut output = Vec::new();

        session
            .run_loop(&mut engine, &mut input, &mut output)
            .expect("repl loop should succeed");

        let rendered = String::from_utf8(output).expect("utf8 output");
        assert!(rendered.contains("transcript pane: focus=transcript"));
        assert!(rendered.contains("provider diagnostics:"));
        assert!(rendered.contains("capability-matrix=mock[stream(request=yes,live=no,sse=no)"));
        assert!(rendered.contains("terminal=completed"));
        assert!(rendered.contains("response-result=none"));
        assert!(rendered.contains("model-error: none"));
        assert!(rendered.contains("latest stream:"));
        assert!(rendered.contains("stream start: provider=mock model=sonnet"));
        assert!(rendered.contains("[t1:conversation] system: bootstrap"));
        assert!(rendered.contains("[t1:conversation] user: hello"));
        assert!(rendered.contains("[t2:conversation] assistant: nocode response: hello"));
    }

    #[test]
    fn transcript_panel_surfaces_provider_error_diagnostics() {
        let mut config = test_config();
        config.json_schema = Some(String::from("{\"type\":\"object\"}"));
        let mut engine = QueryEngine::new(config);
        let mut session = ReplSession::new("nocode");
        let mut input = Cursor::new(b"hello\n/history\n/quit\n");
        let mut output = Vec::new();

        session
            .run_loop(&mut engine, &mut input, &mut output)
            .expect("repl loop should succeed");

        let rendered = String::from_utf8(output).expect("utf8 output");
        assert!(rendered.contains("provider diagnostics:"));
        assert!(rendered.contains("capability-matrix=mock[stream(request=yes,live=no,sse=no)"));
        assert!(rendered.contains("terminal=model_error("));
        assert!(rendered.contains("response-result=none"));
        assert!(rendered.contains("model-error: surface=configuration kind=configuration"));
        assert!(rendered.contains("message=provider mock does not support json schema requests"));
    }

    #[test]
    fn transcript_panel_surfaces_response_result_line() {
        let deps = QueryDeps::builder()
            .with_call_model(StructuredCallModel)
            .build();
        let mut engine = QueryEngine::with_deps(test_config(), deps);
        let mut session = ReplSession::new("nocode");
        let mut input = Cursor::new(b"hello\n/history\n/quit\n");
        let mut output = Vec::new();

        session
            .run_loop(&mut engine, &mut input, &mut output)
            .expect("repl loop should succeed");

        let rendered = String::from_utf8(output).expect("utf8 output");
        assert!(
            session.session_view().iter().any(|line| {
                line.role == "response-result"
                    && line.content == "result={\"ok\":true,\"source\":\"repl\"}"
            }),
            "{:?}",
            session.session_view()
        );
        let snapshot = session.render_tui_snapshot();
        assert!(
            snapshot
                .transcript
                .contains("[RESULT] {\"ok\":true,\"source\":\"repl\"}"),
            "{}",
            snapshot.transcript
        );
        assert!(snapshot.transcript.contains("latest response result:"));
        assert!(snapshot.transcript.contains("turn 2"));
        assert!(snapshot.task_detail.contains("response result:"));
        assert!(snapshot.task_detail.contains("\"source\": \"repl\""));
        assert!(rendered.contains("response-result={\"ok\":true,\"source\":\"repl\"}"));
        assert!(rendered.contains("response-result.pretty:"));
        assert!(rendered.contains("\"source\": \"repl\""));
    }

    #[test]
    fn help_command_lists_new_input_routing_controls() {
        let help = render_help();
        assert!(help.contains("/tasks"));
        assert!(help.contains("/runtime"));
        assert!(help.contains("/focus <transcript|tasks|detail>"));
        assert!(help.contains("/tasks-next"));
        assert!(help.contains("/tasks-prev"));
        assert!(help.contains("/j"));
        assert!(help.contains("/k"));
        assert!(help.contains("/enter"));
        assert!(help.contains("/task-shell <command>"));
        assert!(help.contains("/task-show <task-id|first|last|latest|prev|next>"));
        assert!(help.contains("/task-open"));
        assert!(help.contains("/task-stop <task-id>"));
        assert!(help.contains("/queue <prompt>"));
        assert!(help.contains("/queue-slash </command>"));
        assert!(help.contains("/history-prev"));
        assert!(help.contains("/edit <text>"));
        assert!(help.contains("/send"));
    }

    #[test]
    fn queue_command_executes_and_tracks_queued_origin() {
        let mut engine = QueryEngine::new(test_config());
        let mut session = ReplSession::new("nocode");
        let mut input = Cursor::new(b"/queue queued hello\n/quit\n");
        let mut output = Vec::new();

        session
            .run_loop(&mut engine, &mut input, &mut output)
            .expect("repl loop should succeed");

        let rendered = String::from_utf8(output).expect("utf8 output");
        assert!(rendered.contains("queued prompt: queued hello"));
        assert!(rendered.contains("[t1:conversation] user: queued hello"));
        assert!(rendered.contains("nocode response: queued hello"));
        assert_eq!(
            session.history(),
            &[
                super::ReplInputRecord::new(
                    "/queue queued hello",
                    ReplInputMode::SlashCommand,
                    ReplInputOrigin::Local,
                ),
                super::ReplInputRecord::new(
                    "queued hello",
                    ReplInputMode::Prompt,
                    ReplInputOrigin::Queued,
                ),
                super::ReplInputRecord::new(
                    "/quit",
                    ReplInputMode::SlashCommand,
                    ReplInputOrigin::Local,
                ),
            ]
        );
        let snapshot = session.render_tui_snapshot();
        assert!(snapshot.transcript.contains("[USR:queued] queued hello"));
    }

    #[test]
    fn task_shell_commands_auto_drive_and_task_show_render_detail() {
        let mut engine = QueryEngine::new(test_config());
        let mut session = ReplSession::new("nocode");
        let mut input = Cursor::new(
            b"/task-shell printf ready\n/tasks\n/task-queue\n/task-show b0000000000000001\n/task-run-next\n/task-stop b0000000000000001\n/quit\n",
        );
        let mut output = Vec::new();

        session
            .run_loop(&mut engine, &mut input, &mut output)
            .expect("repl loop should succeed");

        let rendered = String::from_utf8(output).expect("utf8 output");
        eprintln!("{rendered}");
        assert!(rendered.contains("task spawned: b0000000000000001 type=local_shell"));
        assert!(rendered.contains("task auto-drive:"));
        assert!(
            rendered.contains("task drive: b0000000000000001 type=local_shell status=completed")
        );
        assert!(rendered.contains("b0000000000000001 type=local_shell status=completed queue=-"));
        assert!(
            rendered.contains(
                "task summary: filter=all total=1 queued=0 pending=0 running=0 completed=1 failed=0 killed=0"
            )
        );
        assert!(rendered.contains("task queue: empty"));
        assert!(rendered.contains("task b0000000000000001"));
        assert!(rendered.contains("payload=local_shell"));
        assert!(rendered.contains("command=printf ready"));
        assert!(rendered.contains("result.code=0"));
        assert!(rendered.contains("result={\"code\":0,\"interrupted\":false,\"kind\":\"shell\"}"));
        assert!(rendered.contains("task runtime:"));
        assert!(rendered.contains("agent=in-process"));
        assert!(rendered.contains("task drive: idle"));
        assert!(rendered.contains("task stop error: b0000000000000001 not_running"));
    }

    #[test]
    fn task_agent_and_dream_commands_auto_refresh_and_task_show_progress() {
        let mut engine = QueryEngine::new(test_config());
        let mut session = ReplSession::new("nocode");
        let mut input = Cursor::new(
            b"/task-agent agent-a done\n/task-dream 2 review-pass\n/tasks\n/task-show d0000000000000002\n/task-run-all\n/quit\n",
        );
        let mut output = Vec::new();

        session
            .run_loop(&mut engine, &mut input, &mut output)
            .expect("repl loop should succeed");

        let rendered = String::from_utf8(output).expect("utf8 output");
        assert!(rendered.contains("task spawned: a0000000000000001 type=local_agent"));
        assert!(rendered.contains("task spawned: d0000000000000002 type=dream"));
        assert!(rendered.contains("task auto-drive:"));
        assert!(
            rendered.contains("task drive: a0000000000000001 type=local_agent status=completed")
        );
        assert!(rendered.contains("task drive: d0000000000000002 type=dream status=completed"));
        assert!(rendered.contains("agent=agent-a"));
        assert!(rendered.contains("retrieved=true"));
        assert!(rendered.contains("phase=updating sessions=2 files=0 turns=1"));
        assert!(rendered.contains("task d0000000000000002"));
        assert!(rendered.contains("payload=dream"));
        assert!(rendered.contains("sessions_reviewing=2"));
        assert!(rendered.contains("turn[0]=dream review 2 tools=0"));
        assert!(rendered.contains("task drive: idle"));
    }

    #[test]
    fn task_show_latest_and_filtered_tasks_work() {
        let mut engine = QueryEngine::new(test_config());
        let mut session = ReplSession::new("nocode");
        let mut input = Cursor::new(
            b"/task-shell printf ready\n/task-agent agent-a done\n/tasks completed\n/task-show latest\n/tasks shell\n/quit\n",
        );
        let mut output = Vec::new();

        session
            .run_loop(&mut engine, &mut input, &mut output)
            .expect("repl loop should succeed");

        let rendered = String::from_utf8(output).expect("utf8 output");
        assert!(
            rendered.contains(
                "task summary: filter=completed total=2 queued=0 pending=0 running=0 completed=2 failed=0 killed=0"
            )
        );
        assert!(rendered.contains("task a0000000000000002"));
        assert!(rendered.contains("payload=local_agent"));
        assert!(rendered.contains("agent_id=agent-a"));
        assert!(
            rendered.contains(
                "task summary: filter=shell total=1 queued=0 pending=0 running=0 completed=1 failed=0 killed=0"
            )
        );
        assert!(rendered.contains("b0000000000000001 type=local_shell status=completed"));
    }

    #[test]
    fn task_show_prev_next_and_remembered_filter_work() {
        let mut engine = QueryEngine::new(test_config());
        let mut session = ReplSession::new("nocode");
        let mut input = Cursor::new(
            b"/task-shell printf ready\n/task-agent agent-a done\n/task-dream 1 drift\n/tasks agent\n/tasks\n/tasks all\n/task-show latest\n/task-show prev\n/task-show prev\n/task-show next\n/quit\n",
        );
        let mut output = Vec::new();

        session
            .run_loop(&mut engine, &mut input, &mut output)
            .expect("repl loop should succeed");

        let rendered = String::from_utf8(output).expect("utf8 output");
        assert!(
            rendered.contains(
                "task summary: filter=agent total=1 queued=0 pending=0 running=0 completed=1 failed=0 killed=0"
            )
        );
        assert!(rendered.contains("d0000000000000003 type=dream status=completed"));
        assert!(rendered.contains("payload=dream"));
        assert!(rendered.contains("task a0000000000000002"));
        assert!(rendered.contains("payload=local_agent"));
        assert!(rendered.contains("task b0000000000000001"));
        assert!(rendered.contains("payload=local_shell"));
        assert!(
            rendered.contains(
                "task summary: filter=all total=3 queued=0 pending=0 running=0 completed=3 failed=0 killed=0"
            )
        );
    }

    #[test]
    fn task_show_first_last_and_scoped_filters_work() {
        let mut engine = QueryEngine::new(test_config());
        let mut session = ReplSession::new("nocode");
        let mut input = Cursor::new(
            b"/task-shell printf ready\n/task-agent agent-a done\n/task-dream 1 drift\n/tasks status:completed type:agent\n/task-show first\n/task-show last\n/quit\n",
        );
        let mut output = Vec::new();

        session
            .run_loop(&mut engine, &mut input, &mut output)
            .expect("repl loop should succeed");

        let rendered = String::from_utf8(output).expect("utf8 output");
        assert!(
            rendered.contains(
                "task summary: filter=status:completed type:agent total=1 queued=0 pending=0 running=0 completed=1 failed=0 killed=0"
            )
        );
        assert!(rendered.contains(
            "task list:\n> a0000000000000002 type=local_agent status=completed queue=- summary=agent agent-a"
        ));
        assert!(rendered.contains("task a0000000000000002"));
        assert!(rendered.contains("payload=local_agent"));
    }

    #[test]
    fn task_navigation_commands_move_selection_and_open_detail() {
        let mut engine = QueryEngine::new(test_config());
        let mut session = ReplSession::new("nocode");
        let mut input = Cursor::new(
            b"/task-shell printf ready\n/task-agent agent-a done\n/tasks all\n/tasks-prev\n/task-open\n/tasks-next\n/task-open\n/quit\n",
        );
        let mut output = Vec::new();

        session
            .run_loop(&mut engine, &mut input, &mut output)
            .expect("repl loop should succeed");

        let rendered = String::from_utf8(output).expect("utf8 output");
        assert!(rendered.contains("task pane: focus=task_list"));
        assert!(rendered.contains("task pane: focus=task_detail"));
        assert!(rendered.contains("task list:\n> a0000000000000002"));
        assert!(rendered.contains("task detail:\ntask a0000000000000002"));
        assert!(rendered.contains("  b0000000000000001 type=local_shell"));
        assert!(rendered.contains("> b0000000000000001 type=local_shell"));
        assert!(rendered.contains("task detail:\ntask b0000000000000001"));
    }

    #[test]
    fn draft_commands_and_input_history_are_rendered() {
        let mut engine = QueryEngine::new(test_config());
        let mut session = ReplSession::new("nocode");
        let mut input = Cursor::new(
            b"/draft hello\n/append rewrite\n/send\n/inputs\n/history-prev\n/history-next\n/quit\n",
        );
        let mut output = Vec::new();

        session
            .run_loop(&mut engine, &mut input, &mut output)
            .expect("repl loop should succeed");

        let rendered = String::from_utf8(output).expect("utf8 output");
        assert!(rendered.contains("draft: hello"));
        assert!(rendered.contains("draft: hello rewrite"));
        assert!(rendered.contains("[local:slash] /draft hello"));
        assert!(rendered.contains("[local:prompt] hello rewrite"));
        assert!(rendered.contains("draft[1/1 local]: hello rewrite"));
        assert!(rendered.contains("draft cleared"));
    }

    #[test]
    fn focus_and_alias_commands_drive_task_panes() {
        let mut engine = QueryEngine::new(test_config());
        let mut session = ReplSession::new("nocode");
        let mut input = Cursor::new(
            b"/task-shell printf ready\n/task-agent agent-a done\n/focus tasks\n/k\n/enter\n/j\n/enter\n/focus transcript\n/quit\n",
        );
        let mut output = Vec::new();

        session
            .run_loop(&mut engine, &mut input, &mut output)
            .expect("repl loop should succeed");

        let rendered = String::from_utf8(output).expect("utf8 output");
        assert!(rendered.contains("task pane: focus=task_list"));
        assert!(rendered.contains("task pane: focus=task_detail"));
        assert!(rendered.contains("task list:\n> a0000000000000002"));
        assert!(rendered.contains("task detail:\ntask a0000000000000002"));
        assert!(rendered.contains("> b0000000000000001 type=local_shell"));
        assert!(rendered.contains("task detail:\ntask b0000000000000001"));
        assert!(rendered.contains("transcript pane: focus=transcript"));
    }

    #[test]
    fn tui_driver_helpers_process_lines_tick_tasks_and_render_active_pane() {
        let mut engine = QueryEngine::new(test_config());
        let mut session = ReplSession::new("nocode");
        let mut output = Vec::new();

        assert!(
            session
                .process_local_line(&mut engine, &mut output, "/task-shell printf ready")
                .expect("task shell should succeed")
        );
        assert!(
            session
                .process_local_line(&mut engine, &mut output, "/focus tasks")
                .expect("focus tasks should succeed")
        );
        session
            .tick_tasks(&engine, &mut output)
            .expect("task tick should succeed");

        let rendered = session.render_active_pane();
        assert!(rendered.contains("task pane: focus=task_list"));
        assert!(rendered.contains("b0000000000000001 type=local_shell status=completed"));

        let snapshot = session.render_tui_snapshot();
        assert_eq!(snapshot.focus, "task_list");
        assert!(snapshot.status_line.contains("status summary pending"));
        assert!(snapshot.status_line.contains("runtime agent=in-process"));
        assert!(snapshot.transcript.contains("transcript pane"));
        assert!(snapshot.diagnostics_line.contains("provider diagnostics"));
        assert!(snapshot.queue_line.contains("queue=empty"));
        assert!(snapshot.editor_line.contains("editor focus=task_list"));
        assert!(snapshot.footer_line.contains("Tab/Shift-Tab panes"));
        assert!(snapshot.task_list.contains("task list pane [active]"));
        assert!(snapshot.task_list.contains("task runtime:"));
        assert!(snapshot.task_list.contains("agent=in-process"));
        assert!(snapshot.task_detail.contains("task detail pane"));
        assert!(snapshot.task_detail.contains("task b0000000000000001"));
        assert!(snapshot.task_detail.contains("task runtime:"));
    }

    #[test]
    fn tui_task_ticks_drive_agent_stream_incrementally() {
        let mut engine = QueryEngine::new(test_config());
        let mut session = ReplSession::new("nocode");
        let mut output = Vec::new();

        assert!(
            session
                .process_local_line(
                    &mut engine,
                    &mut output,
                    "/task-agent agent-a bridge prompt"
                )
                .expect("task agent should succeed")
        );
        assert!(
            session
                .process_local_line(&mut engine, &mut output, "/focus detail")
                .expect("focus detail should succeed")
        );

        output.clear();
        session
            .tick_tasks(&engine, &mut output)
            .expect("first task tick should succeed");
        let first_tick = String::from_utf8(output.clone()).expect("utf8 output");
        assert!(first_tick.contains("task drive: a0000000000000001"));
        assert!(first_tick.contains("status=running"));
        assert!(first_tick.contains("stream=start:mock/sonnet"));

        let first_snapshot = session.render_tui_snapshot();
        assert!(first_snapshot.task_detail.contains("status=running"));
        assert!(first_snapshot.task_detail.contains("stream_events.count=1"));
        assert!(first_snapshot.task_detail.contains("live activity:"));
        assert!(first_snapshot.task_detail.contains("activity history:"));
        assert!(
            first_snapshot
                .task_detail
                .contains("task=a0000000000000001 status=running stream=start:mock/sonnet")
        );
        assert!(
            first_snapshot
                .task_detail
                .contains("[1] task=a0000000000000001 status=running stream=start:mock/sonnet")
        );
        assert!(
            first_snapshot
                .task_detail
                .contains("stream_events.last=start provider=mock model=sonnet")
        );

        output.clear();
        session
            .tick_tasks(&engine, &mut output)
            .expect("second task tick should succeed");
        let second_tick = String::from_utf8(output.clone()).expect("utf8 output");
        assert!(second_tick.contains("status=running"));
        assert!(second_tick.contains("stream=delta:"));

        output.clear();
        session
            .tick_tasks(&engine, &mut output)
            .expect("third task tick should succeed");
        let third_tick = String::from_utf8(output.clone()).expect("utf8 output");
        assert!(third_tick.contains("status=running"));
        assert!(third_tick.contains("stream=complete:assistant:"));

        output.clear();
        session
            .tick_tasks(&engine, &mut output)
            .expect("final task tick should succeed");
        let final_tick = String::from_utf8(output).expect("utf8 output");
        assert!(final_tick.contains("status=completed"));
        assert!(final_tick.contains("retrieved=true"));

        let final_snapshot = session.render_tui_snapshot();
        assert!(final_snapshot.task_detail.contains("status=completed"));
        assert!(final_snapshot.task_detail.contains("stream_events.count=3"));
        assert!(final_snapshot.task_detail.contains("live activity:"));
        assert!(final_snapshot.task_detail.contains("activity history:"));
        assert!(
            final_snapshot
                .task_detail
                .contains("task=a0000000000000001 status=completed")
        );
        assert!(
            final_snapshot
                .task_detail
                .contains("[4] task=a0000000000000001 status=completed")
        );
        assert!(
            final_snapshot
                .task_detail
                .contains("stream_events.last=complete role=assistant")
        );
        assert!(final_snapshot.task_detail.contains("result.pretty:"));
    }

    #[test]
    fn task_activity_history_keeps_recent_entries_per_task() {
        let mut session = ReplSession::new("nocode");
        let engine = QueryEngine::new(test_config());
        let task_id = session
            .ensure_task_runtime(&engine)
            .coordinator
            .spawn_local_agent(String::from("agent-history"), String::from("prompt"));
        session.task_panel.selected_task_id = Some(task_id.clone());
        session.task_panel.inspected_task_id = Some(task_id.clone());

        for index in 0..(TASK_ACTIVITY_HISTORY_LIMIT + 2) {
            session.record_task_drive_report(&TaskDriveReport {
                task_id: task_id.clone(),
                task_type: TaskType::LocalAgent,
                status: nocode_core::TaskStatus::Running,
                summary: String::from("agent recent"),
                activity: Some(format!("stream=delta:{index}")),
                result_preview: None,
            });
        }

        let snapshot = session.render_tui_snapshot();
        assert!(snapshot.task_detail.contains("activity history:"));
        assert!(!snapshot.task_detail.contains("stream=delta:0"));
        assert!(!snapshot.task_detail.contains("stream=delta:1"));
        assert!(snapshot.task_detail.contains("stream=delta:2"));
        assert!(
            snapshot
                .task_detail
                .contains(format!("stream=delta:{}", TASK_ACTIVITY_HISTORY_LIMIT + 1).as_str())
        );
    }

    #[test]
    fn process_runtime_status_line_surfaces_failure_kind() {
        if OsCommand::new("python3").arg("--version").output().is_err() {
            return;
        }

        let config = test_config();
        let script = r#"import sys
sys.stderr.write("daemon denied")
sys.exit(7)
"#;
        let mut runtime = ReplTaskRuntime {
            coordinator: nocode_core::TaskCoordinator::new(),
            driver: ReplTaskDriver::Process(LiveTaskRuntimeDriver::with_hosts(
                LiveTaskShellHost,
                ProcessTaskAgentHost::with_args(
                    "python3",
                    vec![String::from("-c"), script.to_string()],
                ),
                DefaultDreamHost,
            )),
            info: ReplTaskRuntime::info_for_process_host(
                &config,
                false,
                String::from("python3"),
                vec![String::from("-c"), script.to_string()],
            ),
        };
        let task_id = runtime
            .coordinator
            .spawn_local_agent(String::from("agent-fail"), String::from("prompt"));
        let error = runtime
            .driver
            .drive_next(&mut runtime.coordinator)
            .expect_err("process runtime should surface host failure");

        match error {
            TaskDriveError::DriverFailure {
                task_id: failed_task,
                message,
            } => {
                assert_eq!(failed_task, task_id);
                assert!(message.contains("code 7"));
            }
            other => panic!("unexpected task drive error: {other:?}"),
        }

        let record = runtime
            .coordinator
            .record(&task_id)
            .expect("failed task should remain visible");
        assert_eq!(record.base.status, TaskStatus::Failed);
        let status_line = runtime.render_runtime_status_line();
        let footer = runtime.render_runtime_footer();
        assert!(status_line.contains("last_exit=7"));
        assert!(status_line.contains("last_failure=process_exit"));
        assert!(status_line.contains("failures:1/255"));
        assert!(status_line.contains("backoff=linear"));
        assert!(status_line.contains("last_backoff_profile=none"));
        assert!(footer.contains("agent.last_exit=7"));
        assert!(footer.contains("agent.last_failure_kind=process_exit"));
        assert!(footer.contains("failures:1/255"));
        assert!(footer.contains("backoff=linear"));
        assert!(footer.contains("last_backoff_profile=none"));
        assert!(render_runtime_report(&runtime).contains("runtime report:"));
    }

    #[test]
    fn history_next_restores_stashed_edit_draft() {
        let mut engine = QueryEngine::new(test_config());
        let mut session = ReplSession::new("nocode");
        let mut input =
            Cursor::new(b"first\n/draft scratch\n/history-prev\n/history-next\n/quit\n");
        let mut output = Vec::new();

        session
            .run_loop(&mut engine, &mut input, &mut output)
            .expect("repl loop should succeed");

        let rendered = String::from_utf8(output).expect("utf8 output");
        assert!(rendered.contains("draft[1/1 local]: first"));
        assert!(rendered.contains("draft: scratch"));
    }

    #[test]
    fn history_navigation_marks_queued_prompt_origin() {
        let mut engine = QueryEngine::new(test_config());
        let mut session = ReplSession::new("nocode");
        let mut input = Cursor::new(b"/queue queued hello\n/history-prev\n/quit\n");
        let mut output = Vec::new();

        session
            .run_loop(&mut engine, &mut input, &mut output)
            .expect("repl loop should succeed");

        let rendered = String::from_utf8(output).expect("utf8 output");
        assert!(rendered.contains("draft[1/1 queued]: queued hello"));
    }

    #[test]
    fn tab_complete_single_match_returns_full_command() {
        let session = ReplSession::new("nocode");
        assert_eq!(
            session.complete_command("/ru"),
            Ok(String::from("/runtime"))
        );
        assert_eq!(session.complete_command("/qui"), Ok(String::from("/quit")));
        assert_eq!(session.complete_command("/sen"), Ok(String::from("/send")));
        // Without leading slash.
        assert_eq!(session.complete_command("ru"), Ok(String::from("/runtime")));
    }

    #[test]
    fn tab_complete_multiple_matches_returns_candidates() {
        let session = ReplSession::new("nocode");
        let result = session.complete_command("/task-s");
        assert!(result.is_err());
        let candidates = result.unwrap_err();
        assert!(candidates.contains(&String::from("/task-shell")));
        assert!(candidates.contains(&String::from("/task-show")));
        assert!(candidates.contains(&String::from("/task-stop")));
        assert_eq!(candidates.len(), 3);
    }

    #[test]
    fn multiline_input_merges_backslash_continued_lines() {
        let mut engine = QueryEngine::new(test_config());
        let mut session = ReplSession::new("nocode");
        let mut input = Cursor::new(b"hello \\\nworld\n/quit\n");
        let mut output = Vec::new();

        session
            .run_loop(&mut engine, &mut input, &mut output)
            .expect("repl loop should succeed");

        let rendered = String::from_utf8(output).expect("utf8 output");
        // The two lines should be merged and sent as a single prompt.
        assert!(rendered.contains("[t1:conversation] user: hello"));
        assert!(rendered.contains("world"));
        // Continuation prompt should appear.
        assert!(rendered.contains("... "));
    }
}
