use crate::assistant_turn::AssistantTurnStatus;
use crate::provider::{ModelErrorWire, ModelStreamEventWire, RecordingModelStream};
use crate::query_engine::{QueryEngine, QueryEngineConfig, SubmitMessageOptions};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

static TASK_COORDINATOR: OnceLock<Arc<Mutex<TaskCoordinator>>> = OnceLock::new();

pub fn global_task_coordinator() -> Arc<Mutex<TaskCoordinator>> {
    TASK_COORDINATOR
        .get_or_init(|| Arc::new(Mutex::new(TaskCoordinator::new())))
        .clone()
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TaskId(String);

impl TaskId {
    pub fn new(prefix: &'static str, sequence: u64) -> Self {
        Self(format!("{}{:016x}", prefix, sequence))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn from_string(s: String) -> Self {
        Self(s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskType {
    LocalShell,
    LocalAgent,
    Dream,
}

impl TaskType {
    const fn prefix(self) -> &'static str {
        match self {
            Self::LocalShell => "b",
            Self::LocalAgent => "a",
            Self::Dream => "d",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Killed,
}

impl TaskStatus {
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Killed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskStateBase {
    pub id: TaskId,
    pub task_type: TaskType,
    pub status: TaskStatus,
    pub description: String,
    pub start_time: u64,
    pub end_time: Option<u64>,
    pub notified: bool,
}

impl TaskStateBase {
    fn new(id: TaskId, task_type: TaskType, description: String) -> Self {
        Self {
            id,
            task_type,
            status: TaskStatus::Pending,
            description,
            start_time: current_time_millis(),
            end_time: None,
            notified: false,
        }
    }

    fn mark_running(&mut self) {
        self.status = TaskStatus::Running;
    }

    fn mark_terminal(&mut self, status: TaskStatus) {
        self.status = status;
        self.end_time = Some(current_time_millis());
    }

    fn mark_completed(&mut self) {
        self.mark_terminal(TaskStatus::Completed);
    }

    fn mark_failed(&mut self) {
        self.mark_terminal(TaskStatus::Failed);
    }

    fn mark_killed(&mut self) {
        self.mark_terminal(TaskStatus::Killed);
        self.notified = true;
    }
}

fn current_time_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock went backwards")
        .as_millis() as u64
}

fn truncate_preview(value: String, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value;
    }
    let keep = max_chars.saturating_sub(3);
    let preview = value.chars().take(keep).collect::<String>();
    format!("{preview}...")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BashTaskKind {
    Bash,
    Monitor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DreamPhase {
    Starting,
    Updating,
}

impl DreamPhase {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Updating => "updating",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DreamTurn {
    pub text: String,
    pub tool_use_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalShellTaskData {
    pub command: String,
    pub kind: BashTaskKind,
    pub result: Option<CommandResult>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    pub code: i32,
    pub interrupted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentTaskResult {
    pub retrieved: bool,
    pub progress: AgentProgress,
    pub stream_events: Vec<ModelStreamEventWire>,
    pub response_result: Option<Value>,
    pub model_error: Option<ModelErrorWire>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DreamTaskResult {
    pub phase: DreamPhase,
    pub files_touched: Vec<String>,
    pub turns: Vec<DreamTurn>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskResult {
    Shell(CommandResult),
    Agent(AgentTaskResult),
    Dream(DreamTaskResult),
}

impl TaskResult {
    pub const fn kind_label(&self) -> &'static str {
        match self {
            Self::Shell(_) => "shell",
            Self::Agent(_) => "agent",
            Self::Dream(_) => "dream",
        }
    }

    pub const fn has_response_result_payload(&self) -> bool {
        match self {
            Self::Agent(result) => result.response_result.is_some(),
            Self::Shell(_) | Self::Dream(_) => false,
        }
    }

    pub fn preview(&self, max_chars: usize) -> String {
        truncate_preview(self.to_value().to_string(), max_chars)
    }

    pub fn pretty(&self) -> Option<String> {
        serde_json::to_string_pretty(&self.to_value()).ok()
    }

    pub fn to_value(&self) -> Value {
        match self {
            Self::Shell(result) => serde_json::json!({
                "kind": "shell",
                "code": result.code,
                "interrupted": result.interrupted,
            }),
            Self::Agent(result) => serde_json::json!({
                "kind": "agent",
                "retrieved": result.retrieved,
                "progress": {
                    "tool_use_count": result.progress.tool_use_count,
                    "token_count": result.progress.token_count,
                },
                "response_result": result.response_result,
                "model_error": result.model_error,
                "stream_events": result.stream_events,
            }),
            Self::Dream(result) => serde_json::json!({
                "kind": "dream",
                "phase": result.phase.as_str(),
                "files_touched": result.files_touched,
                "turns": result.turns.iter().map(|turn| serde_json::json!({
                    "text": turn.text,
                    "tool_use_count": turn.tool_use_count,
                })).collect::<Vec<_>>(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalAgentTaskData {
    pub agent_id: String,
    pub prompt: String,
    pub progress: AgentProgress,
    pub retrieved: bool,
    pub stream_events: Vec<ModelStreamEventWire>,
    pub response_result: Option<Value>,
    pub model_error: Option<ModelErrorWire>,
}

impl LocalAgentTaskData {
    pub fn response_result_preview(&self, max_chars: usize) -> String {
        self.response_result
            .as_ref()
            .map(|value| truncate_preview(value.to_string(), max_chars))
            .unwrap_or_else(|| String::from("none"))
    }

    pub fn response_result_pretty(&self) -> Option<String> {
        self.response_result
            .as_ref()
            .and_then(|value| serde_json::to_string_pretty(value).ok())
    }

    pub fn result(&self) -> Option<TaskResult> {
        TaskPayload::LocalAgent(self.clone()).result()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentProgress {
    pub tool_use_count: u32,
    pub token_count: u32,
}

impl Default for AgentProgress {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentProgress {
    pub fn new() -> Self {
        Self {
            tool_use_count: 0,
            token_count: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DreamTaskData {
    pub phase: DreamPhase,
    pub sessions_reviewing: usize,
    pub files_touched: Vec<String>,
    pub turns: Vec<DreamTurn>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskPayload {
    LocalShell(LocalShellTaskData),
    LocalAgent(LocalAgentTaskData),
    Dream(DreamTaskData),
}

impl TaskPayload {
    fn task_type(&self) -> TaskType {
        match self {
            Self::LocalShell(_) => TaskType::LocalShell,
            Self::LocalAgent(_) => TaskType::LocalAgent,
            Self::Dream(_) => TaskType::Dream,
        }
    }

    pub fn result(&self) -> Option<TaskResult> {
        match self {
            Self::LocalShell(shell) => shell.result.clone().map(TaskResult::Shell),
            Self::LocalAgent(agent)
                if agent.progress.tool_use_count > 0
                    || agent.progress.token_count > 0
                    || agent.retrieved
                    || !agent.stream_events.is_empty()
                    || agent.response_result.is_some()
                    || agent.model_error.is_some() =>
            {
                Some(TaskResult::Agent(AgentTaskResult {
                    retrieved: agent.retrieved,
                    progress: agent.progress.clone(),
                    stream_events: agent.stream_events.clone(),
                    response_result: agent.response_result.clone(),
                    model_error: agent.model_error.clone(),
                }))
            }
            Self::LocalAgent(_) => None,
            Self::Dream(dream)
                if dream.phase != DreamPhase::Starting
                    || !dream.files_touched.is_empty()
                    || !dream.turns.is_empty() =>
            {
                Some(TaskResult::Dream(DreamTaskResult {
                    phase: dream.phase,
                    files_touched: dream.files_touched.clone(),
                    turns: dream.turns.clone(),
                }))
            }
            Self::Dream(_) => None,
        }
    }

    pub fn result_preview(&self, max_chars: usize) -> Option<String> {
        self.result().map(|result| result.preview(max_chars))
    }

    pub fn result_pretty(&self) -> Option<String> {
        self.result().and_then(|result| result.pretty())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRecord {
    pub base: TaskStateBase,
    pub payload: TaskPayload,
}

impl TaskRecord {
    pub fn result(&self) -> Option<TaskResult> {
        self.payload.result()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentStep {
    pub tool_use_delta: u32,
    pub token_delta: u32,
    pub retrieved: bool,
    pub status: TaskStatus,
    pub stream_events: Vec<ModelStreamEventWire>,
    pub response_result: Option<Value>,
    pub model_error: Option<ModelErrorWire>,
}

impl AgentStep {
    pub fn progress(tool_use_delta: u32, token_delta: u32) -> Self {
        Self {
            tool_use_delta,
            token_delta,
            retrieved: false,
            status: TaskStatus::Running,
            stream_events: Vec::new(),
            response_result: None,
            model_error: None,
        }
    }

    pub fn completed(tool_use_delta: u32, token_delta: u32, retrieved: bool) -> Self {
        Self {
            tool_use_delta,
            token_delta,
            retrieved,
            status: TaskStatus::Completed,
            stream_events: Vec::new(),
            response_result: None,
            model_error: None,
        }
    }

    pub fn with_stream_events(mut self, events: Vec<ModelStreamEventWire>) -> Self {
        self.stream_events = events;
        self
    }

    pub fn with_response_result(mut self, response_result: Value) -> Self {
        self.response_result = Some(response_result);
        self
    }

    pub fn with_model_error(mut self, model_error: ModelErrorWire) -> Self {
        self.model_error = Some(model_error);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DreamStep {
    pub phase: DreamPhase,
    pub files_touched: Vec<String>,
    pub turn: Option<DreamTurn>,
    pub status: TaskStatus,
}

impl DreamStep {
    pub fn progress(
        phase: DreamPhase,
        files_touched: Vec<String>,
        turn: Option<DreamTurn>,
    ) -> Self {
        Self {
            phase,
            files_touched,
            turn,
            status: TaskStatus::Running,
        }
    }

    pub fn completed(
        phase: DreamPhase,
        files_touched: Vec<String>,
        turn: Option<DreamTurn>,
    ) -> Self {
        Self {
            phase,
            files_touched,
            turn,
            status: TaskStatus::Completed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskDriveReport {
    pub task_id: TaskId,
    pub task_type: TaskType,
    pub status: TaskStatus,
    pub summary: String,
    pub activity: Option<String>,
    pub result_preview: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskDriveError {
    DriverFailure { task_id: TaskId, message: String },
    MissingTask { task_id: TaskId },
}

pub trait TaskRuntimeDriver {
    fn run_local_shell(
        &mut self,
        task_id: &TaskId,
        task: &LocalShellTaskData,
    ) -> Result<CommandResult, String>;

    fn run_local_agent(
        &mut self,
        task_id: &TaskId,
        task: &LocalAgentTaskData,
    ) -> Result<AgentStep, String>;

    fn run_dream(&mut self, task_id: &TaskId, task: &DreamTaskData) -> Result<DreamStep, String>;
}

pub trait TaskShellHost {
    fn run_command(&mut self, command: &str) -> Result<CommandResult, String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LiveTaskShellHost;

impl TaskShellHost for LiveTaskShellHost {
    fn run_command(&mut self, command: &str) -> Result<CommandResult, String> {
        let output = Command::new("bash")
            .arg("-lc")
            .arg(command)
            .current_dir(Path::new("."))
            .output()
            .map_err(|error| format!("failed to run shell task: {error}"))?;
        Ok(CommandResult {
            code: output.status.code().unwrap_or(-1),
            interrupted: output.status.code().is_none(),
        })
    }
}

pub trait TaskAgentHost {
    fn run_agent(
        &mut self,
        task_id: &TaskId,
        agent_id: &str,
        prompt: &str,
    ) -> Result<AgentStep, String>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessAgentRequestWire {
    pub agent_id: String,
    pub prompt: String,
}

impl ProcessAgentRequestWire {
    pub fn new(agent_id: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            prompt: prompt.into(),
        }
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub fn from_json(raw: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(raw)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProcessAgentStatusWire {
    Pending,
    Running,
    Completed,
    Failed,
    Killed,
}

impl ProcessAgentStatusWire {
    fn into_task_status(self) -> TaskStatus {
        match self {
            Self::Pending => TaskStatus::Pending,
            Self::Running => TaskStatus::Running,
            Self::Completed => TaskStatus::Completed,
            Self::Failed => TaskStatus::Failed,
            Self::Killed => TaskStatus::Killed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessAgentResponseWire {
    pub tool_use_delta: u32,
    pub token_delta: u32,
    pub retrieved: bool,
    pub status: ProcessAgentStatusWire,
    #[serde(default)]
    pub stream_events: Vec<ModelStreamEventWire>,
    #[serde(alias = "structured_output")]
    pub response_result: Option<Value>,
    #[serde(default)]
    pub model_error: Option<ModelErrorWire>,
}

impl ProcessAgentResponseWire {
    pub fn new(
        tool_use_delta: u32,
        token_delta: u32,
        retrieved: bool,
        status: ProcessAgentStatusWire,
    ) -> Self {
        Self {
            tool_use_delta,
            token_delta,
            retrieved,
            status,
            stream_events: Vec::new(),
            response_result: None,
            model_error: None,
        }
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub fn from_json(raw: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(raw)
    }

    fn into_agent_step(self) -> AgentStep {
        AgentStep {
            tool_use_delta: self.tool_use_delta,
            token_delta: self.token_delta,
            retrieved: self.retrieved,
            status: self.status.into_task_status(),
            stream_events: self.stream_events,
            response_result: self.response_result,
            model_error: self.model_error,
        }
    }

    pub fn with_stream_events(mut self, stream_events: Vec<ModelStreamEventWire>) -> Self {
        self.stream_events = stream_events;
        self
    }

    pub fn with_response_result(mut self, response_result: Value) -> Self {
        self.response_result = Some(response_result);
        self
    }

    pub fn with_model_error(mut self, model_error: ModelErrorWire) -> Self {
        self.model_error = Some(model_error);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ProcessAgentOutputWire {
    Event { event: ModelStreamEventWire },
    ModelError { error: ModelErrorWire },
    Complete { response: ProcessAgentResponseWire },
}

impl ProcessAgentOutputWire {
    pub fn event(event: ModelStreamEventWire) -> Self {
        Self::Event { event }
    }

    pub fn model_error(error: ModelErrorWire) -> Self {
        Self::ModelError { error }
    }

    pub fn complete(response: ProcessAgentResponseWire) -> Self {
        Self::Complete { response }
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub fn from_json(raw: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(raw)
    }
}

fn agent_step_from_process_output(output: ProcessAgentOutputWire) -> AgentStep {
    match output {
        ProcessAgentOutputWire::Event { event } => {
            AgentStep::progress(0, 0).with_stream_events(vec![event])
        }
        ProcessAgentOutputWire::ModelError { error } => {
            AgentStep::progress(0, 0).with_model_error(error)
        }
        ProcessAgentOutputWire::Complete { response } => response.into_agent_step(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessTaskAgentHostMode {
    OneShot,
    Daemon,
}

#[derive(Debug)]
struct ProcessTaskAgentDaemon {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessAgentFailureKind {
    Spawn,
    PipeUnavailable,
    RequestWrite,
    RequestFlush,
    ResponseRead,
    ResponseUtf8,
    ResponseDecode,
    ProcessExit,
    ClosedOutput,
}

impl ProcessAgentFailureKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Spawn => "spawn",
            Self::PipeUnavailable => "pipe_unavailable",
            Self::RequestWrite => "request_write",
            Self::RequestFlush => "request_flush",
            Self::ResponseRead => "response_read",
            Self::ResponseUtf8 => "response_utf8",
            Self::ResponseDecode => "response_decode",
            Self::ProcessExit => "process_exit",
            Self::ClosedOutput => "closed_output",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProcessAgentFailure {
    kind: ProcessAgentFailureKind,
    message: String,
    exit_code: Option<i32>,
}

impl ProcessAgentFailure {
    fn new(kind: ProcessAgentFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            exit_code: None,
        }
    }

    fn with_exit_code(mut self, exit_code: Option<i32>) -> Self {
        self.exit_code = exit_code;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessAgentBackoffStrategy {
    Linear,
    Exponential,
}

impl ProcessAgentBackoffStrategy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Exponential => "exponential",
        }
    }

    pub const fn delay_ms(self, base_ms: u64, attempts_used: u8) -> u64 {
        if base_ms == 0 || attempts_used == 0 {
            return 0;
        }
        match self {
            Self::Linear => base_ms.saturating_mul(attempts_used as u64),
            Self::Exponential => {
                let factor = 1u64.checked_shl(attempts_used.saturating_sub(1) as u32);
                match factor {
                    Some(value) => value.saturating_mul(base_ms),
                    None => u64::MAX,
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessAgentBackoffProfileKind {
    Default,
    Io,
    Decode,
    Exit,
}

impl ProcessAgentBackoffProfileKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Io => "io",
            Self::Decode => "decode",
            Self::Exit => "exit",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessAgentRestartPolicy {
    pub max_restart_attempts: u8,
    pub max_consecutive_failures: u8,
    pub restart_on_io_error: bool,
    pub restart_on_decode_error: bool,
    pub restart_on_clean_exit: bool,
}

impl Default for ProcessAgentRestartPolicy {
    fn default() -> Self {
        Self {
            max_restart_attempts: 1,
            max_consecutive_failures: u8::MAX,
            restart_on_io_error: true,
            restart_on_decode_error: true,
            restart_on_clean_exit: true,
        }
    }
}

impl ProcessAgentRestartPolicy {
    fn should_restart(
        self,
        attempts_used: u8,
        consecutive_failure_count: u8,
        failure: &ProcessAgentFailure,
    ) -> bool {
        if attempts_used > self.max_restart_attempts {
            return false;
        }
        if consecutive_failure_count.saturating_add(1) >= self.max_consecutive_failures {
            return false;
        }
        match failure.kind {
            ProcessAgentFailureKind::Spawn => false,
            ProcessAgentFailureKind::PipeUnavailable
            | ProcessAgentFailureKind::RequestWrite
            | ProcessAgentFailureKind::RequestFlush
            | ProcessAgentFailureKind::ResponseRead
            | ProcessAgentFailureKind::ClosedOutput => self.restart_on_io_error,
            ProcessAgentFailureKind::ResponseUtf8 | ProcessAgentFailureKind::ResponseDecode => {
                self.restart_on_decode_error
            }
            ProcessAgentFailureKind::ProcessExit => {
                failure.exit_code != Some(0) || self.restart_on_clean_exit
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessAgentBackoffProfile {
    pub base_delay_ms: u64,
    pub strategy: ProcessAgentBackoffStrategy,
    pub jitter_percent: u8,
}

impl Default for ProcessAgentBackoffProfile {
    fn default() -> Self {
        Self {
            base_delay_ms: 0,
            strategy: ProcessAgentBackoffStrategy::Linear,
            jitter_percent: 0,
        }
    }
}

impl ProcessAgentBackoffProfile {
    pub fn delay_ms(self, attempts_used: u8, entropy: u64) -> u64 {
        let base_delay = self.strategy.delay_ms(self.base_delay_ms, attempts_used);
        if base_delay == 0 || self.jitter_percent == 0 {
            return base_delay;
        }
        let jitter_cap = base_delay
            .saturating_mul(u64::from(self.jitter_percent))
            .saturating_div(100);
        if jitter_cap == 0 {
            return base_delay;
        }
        let jitter = entropy % jitter_cap.saturating_add(1);
        base_delay.saturating_add(jitter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProcessAgentBackoffPolicy {
    pub default_profile: ProcessAgentBackoffProfile,
    pub io_profile: Option<ProcessAgentBackoffProfile>,
    pub decode_profile: Option<ProcessAgentBackoffProfile>,
    pub exit_profile: Option<ProcessAgentBackoffProfile>,
}

impl ProcessAgentBackoffPolicy {
    const fn profile_kind_for_failure(
        self,
        failure: &ProcessAgentFailure,
    ) -> ProcessAgentBackoffProfileKind {
        match failure.kind {
            ProcessAgentFailureKind::PipeUnavailable
            | ProcessAgentFailureKind::RequestWrite
            | ProcessAgentFailureKind::RequestFlush
            | ProcessAgentFailureKind::ResponseRead
            | ProcessAgentFailureKind::ClosedOutput => ProcessAgentBackoffProfileKind::Io,
            ProcessAgentFailureKind::ResponseUtf8 | ProcessAgentFailureKind::ResponseDecode => {
                ProcessAgentBackoffProfileKind::Decode
            }
            ProcessAgentFailureKind::ProcessExit => ProcessAgentBackoffProfileKind::Exit,
            ProcessAgentFailureKind::Spawn => ProcessAgentBackoffProfileKind::Default,
        }
    }

    pub const fn profile_for_kind(
        self,
        kind: ProcessAgentBackoffProfileKind,
    ) -> ProcessAgentBackoffProfile {
        match kind {
            ProcessAgentBackoffProfileKind::Default => self.default_profile,
            ProcessAgentBackoffProfileKind::Io => match self.io_profile {
                Some(profile) => profile,
                None => self.default_profile,
            },
            ProcessAgentBackoffProfileKind::Decode => match self.decode_profile {
                Some(profile) => profile,
                None => self.default_profile,
            },
            ProcessAgentBackoffProfileKind::Exit => match self.exit_profile {
                Some(profile) => profile,
                None => self.default_profile,
            },
        }
    }

    const fn profile_for_failure(
        self,
        failure: &ProcessAgentFailure,
    ) -> (ProcessAgentBackoffProfileKind, ProcessAgentBackoffProfile) {
        let kind = self.profile_kind_for_failure(failure);
        (kind, self.profile_for_kind(kind))
    }

    fn delay_ms(
        self,
        failure: &ProcessAgentFailure,
        attempts_used: u8,
        entropy: u64,
    ) -> (ProcessAgentBackoffProfileKind, u64) {
        let (kind, profile) = self.profile_for_failure(failure);
        (kind, profile.delay_ms(attempts_used, entropy))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProcessAgentSupervisorPolicy {
    pub restart: ProcessAgentRestartPolicy,
    pub backoff: ProcessAgentBackoffPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProcessAgentSupervisorState {
    pub request_count: u64,
    pub spawn_count: u64,
    pub consecutive_failure_count: u8,
    pub last_backoff_profile_kind: Option<ProcessAgentBackoffProfileKind>,
    pub last_backoff_ms: u64,
    pub last_failure_kind: Option<ProcessAgentFailureKind>,
    pub last_error: Option<String>,
    pub last_exit_code: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProcessAgentSupervisor {
    policy: ProcessAgentSupervisorPolicy,
    state: ProcessAgentSupervisorState,
}

impl ProcessAgentSupervisor {
    pub fn new(policy: ProcessAgentSupervisorPolicy) -> Self {
        Self {
            policy,
            state: ProcessAgentSupervisorState::default(),
        }
    }

    pub const fn policy(&self) -> ProcessAgentSupervisorPolicy {
        self.policy
    }

    pub const fn state(&self) -> &ProcessAgentSupervisorState {
        &self.state
    }

    pub fn record_request(&mut self) {
        self.state.request_count += 1;
    }

    pub fn record_spawn(&mut self) {
        self.state.spawn_count += 1;
        self.state.last_backoff_profile_kind = None;
        self.state.last_failure_kind = None;
        self.state.last_error = None;
        self.state.last_exit_code = None;
    }

    pub fn record_success(&mut self) {
        self.state.consecutive_failure_count = 0;
        self.state.last_backoff_profile_kind = None;
        self.state.last_failure_kind = None;
        self.state.last_error = None;
        self.state.last_exit_code = None;
    }

    pub fn record_failure(
        &mut self,
        kind: ProcessAgentFailureKind,
        message: String,
        exit_code: Option<i32>,
    ) {
        self.state.consecutive_failure_count =
            self.state.consecutive_failure_count.saturating_add(1);
        self.state.last_failure_kind = Some(kind);
        self.state.last_error = Some(message);
        self.state.last_exit_code = exit_code;
    }

    fn should_restart(&self, attempts_used: u8, failure: &ProcessAgentFailure) -> bool {
        self.policy.restart.should_restart(
            attempts_used,
            self.state.consecutive_failure_count,
            failure,
        )
    }

    fn apply_backoff(&self, attempts_used: u8, failure: &ProcessAgentFailure) {
        let (_, delay_ms) = self.backoff_delay_ms(attempts_used, failure);
        if delay_ms == 0 {
            return;
        }
        thread::sleep(std::time::Duration::from_millis(delay_ms));
    }

    fn record_backoff(&mut self, attempts_used: u8, failure: &ProcessAgentFailure) {
        let (kind, delay_ms) = self.backoff_delay_ms(attempts_used, failure);
        self.state.last_backoff_profile_kind = Some(kind);
        self.state.last_backoff_ms = delay_ms;
    }

    fn backoff_delay_ms(
        &self,
        attempts_used: u8,
        failure: &ProcessAgentFailure,
    ) -> (ProcessAgentBackoffProfileKind, u64) {
        self.policy
            .backoff
            .delay_ms(failure, attempts_used, self.backoff_entropy(attempts_used))
    }

    pub const fn restart_count(&self) -> u64 {
        self.state.spawn_count.saturating_sub(1)
    }

    const fn backoff_entropy(&self, attempts_used: u8) -> u64 {
        self.state
            .request_count
            .saturating_add(self.state.spawn_count)
            .saturating_add(attempts_used as u64)
    }
}

pub struct ProcessTaskAgentHost {
    mode: ProcessTaskAgentHostMode,
    command: String,
    args: Vec<String>,
    cwd: Option<String>,
    daemon: Option<ProcessTaskAgentDaemon>,
    active_task_id: Option<String>,
    pending_steps: HashMap<String, VecDeque<AgentStep>>,
    supervisor: ProcessAgentSupervisor,
}

impl ProcessTaskAgentHost {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            mode: ProcessTaskAgentHostMode::OneShot,
            command: command.into(),
            args: Vec::new(),
            cwd: None,
            daemon: None,
            active_task_id: None,
            pending_steps: HashMap::new(),
            supervisor: ProcessAgentSupervisor::default(),
        }
    }

    pub fn with_args(command: impl Into<String>, args: impl IntoIterator<Item = String>) -> Self {
        Self {
            mode: ProcessTaskAgentHostMode::OneShot,
            command: command.into(),
            args: args.into_iter().collect(),
            cwd: None,
            daemon: None,
            active_task_id: None,
            pending_steps: HashMap::new(),
            supervisor: ProcessAgentSupervisor::default(),
        }
    }

    pub fn with_cwd(mut self, cwd: impl Into<String>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn with_daemon_mode(mut self) -> Self {
        self.mode = ProcessTaskAgentHostMode::Daemon;
        self
    }

    pub fn with_daemon_restart_budget(mut self, attempts: u8) -> Self {
        self.supervisor.policy.restart.max_restart_attempts = attempts;
        self
    }

    pub fn with_daemon_max_consecutive_failures(mut self, failures: u8) -> Self {
        self.supervisor.policy.restart.max_consecutive_failures = failures;
        self
    }

    pub fn with_daemon_restart_backoff_ms(mut self, backoff_ms: u64) -> Self {
        self.supervisor.policy.backoff.default_profile.base_delay_ms = backoff_ms;
        self
    }

    pub fn with_daemon_restart_backoff_profile(
        mut self,
        profile: ProcessAgentBackoffProfile,
    ) -> Self {
        self.supervisor.policy.backoff.default_profile = profile;
        self
    }

    pub fn with_daemon_restart_backoff_strategy(
        mut self,
        strategy: ProcessAgentBackoffStrategy,
    ) -> Self {
        self.supervisor.policy.backoff.default_profile.strategy = strategy;
        self
    }

    pub fn with_daemon_restart_backoff_jitter_percent(mut self, jitter_percent: u8) -> Self {
        self.supervisor
            .policy
            .backoff
            .default_profile
            .jitter_percent = jitter_percent;
        self
    }

    pub fn with_daemon_io_backoff_profile(mut self, profile: ProcessAgentBackoffProfile) -> Self {
        self.supervisor.policy.backoff.io_profile = Some(profile);
        self
    }

    pub fn with_daemon_decode_backoff_profile(
        mut self,
        profile: ProcessAgentBackoffProfile,
    ) -> Self {
        self.supervisor.policy.backoff.decode_profile = Some(profile);
        self
    }

    pub fn with_daemon_exit_backoff_profile(mut self, profile: ProcessAgentBackoffProfile) -> Self {
        self.supervisor.policy.backoff.exit_profile = Some(profile);
        self
    }

    pub fn mode_label(&self) -> &'static str {
        match self.mode {
            ProcessTaskAgentHostMode::OneShot => "process-host",
            ProcessTaskAgentHostMode::Daemon => "process-daemon",
        }
    }

    pub fn daemon_running(&self) -> bool {
        self.daemon.is_some()
    }

    pub const fn request_count(&self) -> u64 {
        self.supervisor.state.request_count
    }

    pub const fn spawn_count(&self) -> u64 {
        self.supervisor.state.spawn_count
    }

    pub const fn restart_count(&self) -> u64 {
        self.supervisor.restart_count()
    }

    pub const fn max_restart_attempts(&self) -> u8 {
        self.supervisor.policy.restart.max_restart_attempts
    }

    pub const fn restart_backoff_ms(&self) -> u64 {
        self.supervisor.policy.backoff.default_profile.base_delay_ms
    }

    pub const fn restart_backoff_strategy(&self) -> ProcessAgentBackoffStrategy {
        self.supervisor.policy.backoff.default_profile.strategy
    }

    pub const fn restart_backoff_jitter_percent(&self) -> u8 {
        self.supervisor
            .policy
            .backoff
            .default_profile
            .jitter_percent
    }

    pub const fn max_consecutive_failures(&self) -> u8 {
        self.supervisor.policy.restart.max_consecutive_failures
    }

    pub const fn consecutive_failure_count(&self) -> u8 {
        self.supervisor.state.consecutive_failure_count
    }

    pub const fn last_backoff_ms(&self) -> u64 {
        self.supervisor.state.last_backoff_ms
    }

    pub fn last_backoff_profile_kind(&self) -> Option<ProcessAgentBackoffProfileKind> {
        self.supervisor.state.last_backoff_profile_kind
    }

    pub fn last_failure_kind(&self) -> Option<ProcessAgentFailureKind> {
        self.supervisor.state.last_failure_kind
    }

    pub fn last_error(&self) -> Option<&str> {
        self.supervisor.state.last_error.as_deref()
    }

    pub const fn last_exit_code(&self) -> Option<i32> {
        self.supervisor.state.last_exit_code
    }

    pub const fn supervisor(&self) -> &ProcessAgentSupervisor {
        &self.supervisor
    }

    pub fn with_daemon_restart_on_io_error(mut self, enabled: bool) -> Self {
        self.supervisor.policy.restart.restart_on_io_error = enabled;
        self
    }

    pub fn with_daemon_restart_on_decode_error(mut self, enabled: bool) -> Self {
        self.supervisor.policy.restart.restart_on_decode_error = enabled;
        self
    }

    pub fn with_daemon_restart_on_clean_exit(mut self, enabled: bool) -> Self {
        self.supervisor.policy.restart.restart_on_clean_exit = enabled;
        self
    }

    fn spawn_command(&self, stderr: Stdio) -> Command {
        let mut command = Command::new(&self.command);
        command.args(&self.args);
        command.stdin(Stdio::piped());
        command.stdout(Stdio::piped());
        command.stderr(stderr);
        if let Some(cwd) = &self.cwd {
            command.current_dir(cwd);
        }
        command
    }

    fn ensure_daemon(&mut self) -> Result<&mut ProcessTaskAgentDaemon, ProcessAgentFailure> {
        enum DaemonProbe {
            Running,
            Exited(Option<i32>),
            InspectError(String),
        }
        let probe = if let Some(daemon) = self.daemon.as_mut() {
            match daemon.child.try_wait() {
                Ok(Some(status)) => DaemonProbe::Exited(status.code()),
                Ok(None) => DaemonProbe::Running,
                Err(error) => DaemonProbe::InspectError(error.to_string()),
            }
        } else {
            DaemonProbe::Running
        };
        match probe {
            DaemonProbe::Running => {}
            DaemonProbe::Exited(exit_code) => {
                self.daemon = None;
                return Err(ProcessAgentFailure::new(
                    ProcessAgentFailureKind::ProcessExit,
                    match exit_code {
                        Some(code) => {
                            format!("process agent daemon exited before response: {code}")
                        }
                        None => String::from("process agent daemon closed stdout"),
                    },
                )
                .with_exit_code(exit_code));
            }
            DaemonProbe::InspectError(error) => {
                return Err(self.finalize_daemon_failure(ProcessAgentFailure::new(
                    ProcessAgentFailureKind::ResponseRead,
                    format!("failed to inspect process agent daemon: {error}"),
                )));
            }
        }
        if self.daemon.is_none() {
            let mut child = self
                .spawn_command(Stdio::inherit())
                .spawn()
                .map_err(|error| {
                    ProcessAgentFailure::new(
                        ProcessAgentFailureKind::Spawn,
                        format!("failed to spawn process agent daemon: {error}"),
                    )
                })?;
            let stdin = child.stdin.take().ok_or_else(|| {
                let _ = child.kill();
                let _ = child.wait();
                ProcessAgentFailure::new(
                    ProcessAgentFailureKind::PipeUnavailable,
                    "process agent daemon stdin unavailable",
                )
            })?;
            let stdout = child.stdout.take().ok_or_else(|| {
                let _ = child.kill();
                let _ = child.wait();
                ProcessAgentFailure::new(
                    ProcessAgentFailureKind::PipeUnavailable,
                    "process agent daemon stdout unavailable",
                )
            })?;
            self.daemon = Some(ProcessTaskAgentDaemon {
                child,
                stdin,
                stdout: BufReader::new(stdout),
            });
            self.supervisor.record_spawn();
        }
        self.daemon.as_mut().ok_or_else(|| {
            ProcessAgentFailure::new(
                ProcessAgentFailureKind::PipeUnavailable,
                "process agent daemon unavailable",
            )
        })
    }

    fn run_agent_one_shot_steps(
        &self,
        payload: &str,
    ) -> Result<VecDeque<AgentStep>, ProcessAgentFailure> {
        let mut child = self
            .spawn_command(Stdio::piped())
            .spawn()
            .map_err(|error| {
                ProcessAgentFailure::new(
                    ProcessAgentFailureKind::Spawn,
                    format!("failed to spawn process agent host: {error}"),
                )
            })?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(payload.as_bytes()).map_err(|error| {
                ProcessAgentFailure::new(
                    ProcessAgentFailureKind::RequestWrite,
                    format!("failed to write process agent request: {error}"),
                )
            })?;
        } else {
            return Err(ProcessAgentFailure::new(
                ProcessAgentFailureKind::PipeUnavailable,
                "process agent host stdin unavailable",
            ));
        }

        let output = child.wait_with_output().map_err(|error| {
            ProcessAgentFailure::new(
                ProcessAgentFailureKind::ResponseRead,
                format!("failed to read process agent host output: {error}"),
            )
        })?;
        if !output.status.success() {
            let code = output.status.code().unwrap_or(-1);
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ProcessAgentFailure::new(
                ProcessAgentFailureKind::ProcessExit,
                format!(
                    "process agent host exited with code {code}: {}",
                    stderr.trim()
                ),
            )
            .with_exit_code(output.status.code()));
        }

        let raw = String::from_utf8(output.stdout).map_err(|error| {
            ProcessAgentFailure::new(
                ProcessAgentFailureKind::ResponseUtf8,
                format!("process agent response not utf-8: {error}"),
            )
        })?;
        decode_process_agent_output_steps(raw.lines())
            .map(VecDeque::from)
            .map_err(|message| {
                ProcessAgentFailure::new(ProcessAgentFailureKind::ResponseDecode, message)
            })
    }

    fn start_agent_daemon(
        &mut self,
        task_id: &TaskId,
        payload: &str,
    ) -> Result<(), ProcessAgentFailure> {
        let write_request = {
            let daemon = self.ensure_daemon()?;
            daemon.stdin.write_all(payload.as_bytes())
        };
        if let Err(error) = write_request {
            self.active_task_id = None;
            return Err(self.finalize_daemon_failure(ProcessAgentFailure::new(
                ProcessAgentFailureKind::RequestWrite,
                format!("failed to write process agent daemon request: {error}"),
            )));
        }

        let terminate_request = {
            let daemon = self.ensure_daemon()?;
            daemon.stdin.write_all(b"\n")
        };
        if let Err(error) = terminate_request {
            self.active_task_id = None;
            return Err(self.finalize_daemon_failure(ProcessAgentFailure::new(
                ProcessAgentFailureKind::RequestWrite,
                format!("failed to terminate process agent daemon request: {error}"),
            )));
        }

        let flush_request = {
            let daemon = self.ensure_daemon()?;
            daemon.stdin.flush()
        };
        if let Err(error) = flush_request {
            self.active_task_id = None;
            return Err(self.finalize_daemon_failure(ProcessAgentFailure::new(
                ProcessAgentFailureKind::RequestFlush,
                format!("failed to flush process agent daemon request: {error}"),
            )));
        }
        self.active_task_id = Some(task_id.as_str().to_string());
        Ok(())
    }

    fn read_next_daemon_output(&mut self) -> Result<AgentStep, ProcessAgentFailure> {
        let active_task_id = self.active_task_id.clone();
        let read_result = {
            let daemon = self.ensure_daemon()?;
            let mut raw = String::new();
            match daemon.stdout.read_line(&mut raw) {
                Ok(0) => {
                    let exit_code = settle_child_exit_code(&mut daemon.child);
                    Err(match exit_code {
                        Some(code) => ProcessAgentFailure::new(
                            ProcessAgentFailureKind::ProcessExit,
                            format!("process agent daemon exited before response: {code}"),
                        )
                        .with_exit_code(Some(code)),
                        None => ProcessAgentFailure::new(
                            ProcessAgentFailureKind::ClosedOutput,
                            "process agent daemon closed stdout",
                        ),
                    })
                }
                Ok(_) => decode_process_agent_output(raw.trim())
                    .map(agent_step_from_process_output)
                    .map_err(|message| {
                        ProcessAgentFailure::new(ProcessAgentFailureKind::ResponseDecode, message)
                    }),
                Err(error) => Err(ProcessAgentFailure::new(
                    ProcessAgentFailureKind::ResponseRead,
                    format!("failed to read process agent daemon response: {error}"),
                )),
            }
        };
        match read_result {
            Ok(step) => {
                if step.status.is_terminal() {
                    self.active_task_id = None;
                } else {
                    self.active_task_id = active_task_id;
                }
                Ok(step)
            }
            Err(failure) => {
                self.active_task_id = None;
                Err(self.finalize_daemon_failure(failure))
            }
        }
    }

    fn run_agent_daemon(&mut self, task_id: &TaskId, payload: &str) -> Result<AgentStep, String> {
        let mut attempts = 0u8;
        loop {
            if self
                .active_task_id
                .as_deref()
                .is_some_and(|active| active != task_id.as_str())
            {
                return Err(format!(
                    "process agent daemon busy with task {}",
                    self.active_task_id.as_deref().unwrap_or("unknown")
                ));
            }
            let starting = self.active_task_id.is_none();
            if starting {
                if attempts == 0 {
                    self.supervisor.record_request();
                }
                attempts += 1;
                if let Err(failure) = self.start_agent_daemon(task_id, payload) {
                    if self.supervisor.should_restart(attempts, &failure) {
                        self.supervisor.record_failure(
                            failure.kind,
                            failure.message.clone(),
                            failure.exit_code,
                        );
                        self.supervisor.record_backoff(attempts, &failure);
                        self.supervisor.apply_backoff(attempts, &failure);
                        continue;
                    }
                    self.supervisor.record_failure(
                        failure.kind,
                        failure.message.clone(),
                        failure.exit_code,
                    );
                    return Err(failure.message);
                }
            }
            match self.read_next_daemon_output() {
                Ok(step) => {
                    if step.status.is_terminal() {
                        self.supervisor.record_success();
                    }
                    return Ok(step);
                }
                Err(failure) if self.supervisor.should_restart(attempts, &failure) => {
                    self.supervisor.record_failure(
                        failure.kind,
                        failure.message.clone(),
                        failure.exit_code,
                    );
                    self.supervisor.record_backoff(attempts, &failure);
                    self.supervisor.apply_backoff(attempts, &failure);
                }
                Err(failure) => {
                    self.supervisor.record_failure(
                        failure.kind,
                        failure.message.clone(),
                        failure.exit_code,
                    );
                    return Err(failure.message);
                }
            }
        }
    }

    fn finalize_daemon_failure(&mut self, mut failure: ProcessAgentFailure) -> ProcessAgentFailure {
        if self.daemon.is_none() {
            return failure;
        }
        let exit_code = self.reset_daemon();
        if failure.exit_code.is_none() {
            failure.exit_code = exit_code;
        }
        failure
    }

    fn reset_daemon(&mut self) -> Option<i32> {
        let mut daemon = self.daemon.take()?;
        self.active_task_id = None;
        let exit_code = settle_child_exit_code(&mut daemon.child);
        if exit_code.is_none() {
            let _ = daemon.child.kill();
            let _ = daemon.child.wait();
        }
        exit_code
    }
}

fn probe_child_exit_code(child: &mut Child) -> Option<i32> {
    child
        .try_wait()
        .ok()
        .flatten()
        .and_then(|status| status.code())
}

fn settle_child_exit_code(child: &mut Child) -> Option<i32> {
    let exit_code = probe_child_exit_code(child);
    if exit_code.is_some() {
        return exit_code;
    }
    thread::sleep(std::time::Duration::from_millis(10));
    probe_child_exit_code(child)
}

impl std::fmt::Debug for ProcessTaskAgentHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessTaskAgentHost")
            .field("mode", &self.mode)
            .field("command", &self.command)
            .field("args", &self.args)
            .field("cwd", &self.cwd)
            .field("daemon_running", &self.daemon.is_some())
            .field("active_task_id", &self.active_task_id)
            .field("pending_step_tasks", &self.pending_steps.len())
            .field("supervisor", &self.supervisor)
            .finish()
    }
}

impl Drop for ProcessTaskAgentHost {
    fn drop(&mut self) {
        if let Some(mut daemon) = self.daemon.take() {
            let _ = daemon.stdin.flush();
            let _ = daemon.child.kill();
            let _ = daemon.child.wait();
        }
    }
}

impl TaskAgentHost for ProcessTaskAgentHost {
    fn run_agent(
        &mut self,
        task_id: &TaskId,
        agent_id: &str,
        prompt: &str,
    ) -> Result<AgentStep, String> {
        if let Some(step) = self
            .pending_steps
            .get_mut(task_id.as_str())
            .and_then(VecDeque::pop_front)
        {
            if self
                .pending_steps
                .get(task_id.as_str())
                .is_some_and(VecDeque::is_empty)
            {
                self.pending_steps.remove(task_id.as_str());
            }
            return Ok(step);
        }
        let request = ProcessAgentRequestWire::new(agent_id, prompt);
        let payload = request
            .to_json()
            .map_err(|error| format!("failed to serialize process agent request: {error}"))?;
        match self.mode {
            ProcessTaskAgentHostMode::OneShot => {
                self.supervisor.record_request();
                let steps = self.run_agent_one_shot_steps(&payload);
                match &steps {
                    Ok(_) => self.supervisor.record_success(),
                    Err(failure) => self.supervisor.record_failure(
                        failure.kind,
                        failure.message.clone(),
                        failure.exit_code,
                    ),
                }
                let mut steps = steps.map_err(|failure| failure.message)?;
                let step = steps.pop_front().ok_or_else(|| {
                    String::from("process agent protocol ended without complete frame")
                })?;
                if !steps.is_empty() {
                    self.pending_steps
                        .insert(task_id.as_str().to_string(), steps);
                }
                Ok(step)
            }
            ProcessTaskAgentHostMode::Daemon => self.run_agent_daemon(task_id, &payload),
        }
    }
}

fn decode_process_agent_output(raw: &str) -> Result<ProcessAgentOutputWire, String> {
    ProcessAgentOutputWire::from_json(raw)
        .or_else(|_| ProcessAgentResponseWire::from_json(raw).map(ProcessAgentOutputWire::complete))
        .map_err(|error| format!("failed to decode process agent response: {error}"))
}

fn decode_process_agent_output_steps<'a>(
    lines: impl IntoIterator<Item = &'a str>,
) -> Result<Vec<AgentStep>, String> {
    let mut steps = Vec::new();
    let mut saw_complete = false;
    for raw in lines
        .into_iter()
        .map(str::trim)
        .filter(|raw| !raw.is_empty())
    {
        let output = decode_process_agent_output(raw)?;
        let terminal = matches!(output, ProcessAgentOutputWire::Complete { .. });
        steps.push(agent_step_from_process_output(output));
        if terminal {
            saw_complete = true;
            break;
        }
    }
    if !saw_complete {
        return Err(String::from(
            "process agent protocol ended without complete frame",
        ));
    }
    Ok(steps)
}

#[derive(Debug)]
pub struct InProcessAgentHost {
    template: QueryEngineConfig,
    engines: HashMap<String, QueryEngine>,
    pending_steps: HashMap<String, VecDeque<AgentStep>>,
}

impl InProcessAgentHost {
    pub fn new(template: QueryEngineConfig) -> Self {
        Self {
            template,
            engines: HashMap::new(),
            pending_steps: HashMap::new(),
        }
    }

    fn engine_for(&mut self, agent_id: &str) -> &mut QueryEngine {
        self.engines.entry(agent_id.to_string()).or_insert_with(|| {
            let mut config = self.template.clone();
            config.session_id = format!("{}-{agent_id}", self.template.session_id);
            QueryEngine::new(config)
        })
    }
}

impl TaskAgentHost for InProcessAgentHost {
    fn run_agent(
        &mut self,
        task_id: &TaskId,
        agent_id: &str,
        prompt: &str,
    ) -> Result<AgentStep, String> {
        if let Some(step) = self
            .pending_steps
            .get_mut(task_id.as_str())
            .and_then(VecDeque::pop_front)
        {
            if self
                .pending_steps
                .get(task_id.as_str())
                .is_some_and(VecDeque::is_empty)
            {
                self.pending_steps.remove(task_id.as_str());
            }
            return Ok(step);
        }
        let mut stream = RecordingModelStream::default();
        let plan = self.engine_for(agent_id).submit_message_with_stream(
            prompt.to_string(),
            SubmitMessageOptions::default(),
            &mut stream,
        );
        let tool_use_delta = u32::try_from(plan.tool_results.len()).unwrap_or(u32::MAX);
        let token_delta = u32::try_from(plan.usage_snapshot.output_tokens).unwrap_or(u32::MAX);
        let retrieved = plan.model_response.final_assistant_message.is_some();
        let mut steps = stream
            .events
            .iter()
            .map(|event| {
                AgentStep::progress(0, 0)
                    .with_stream_events(vec![ModelStreamEventWire::from(event)])
            })
            .collect::<VecDeque<_>>();
        let mut final_step = match plan.assistant_turn.status {
            AssistantTurnStatus::Continue => AgentStep::progress(tool_use_delta, token_delta),
            AssistantTurnStatus::Completed => {
                AgentStep::completed(tool_use_delta, token_delta, retrieved)
            }
            AssistantTurnStatus::Terminal => AgentStep {
                tool_use_delta,
                token_delta,
                retrieved,
                status: TaskStatus::Failed,
                stream_events: Vec::new(),
                response_result: None,
                model_error: None,
            },
        };
        if let Some(response_result) = plan.response_result.clone() {
            final_step = final_step.with_response_result(response_result);
        }
        if let Some(model_error) = plan.model_error.as_ref().map(ModelErrorWire::from) {
            final_step = final_step.with_model_error(model_error);
        }
        steps.push_back(final_step);
        let step = steps
            .pop_front()
            .ok_or_else(|| String::from("agent step queue missing"))?;
        if !steps.is_empty() {
            self.pending_steps
                .insert(task_id.as_str().to_string(), steps);
        }
        Ok(step)
    }
}

pub trait TaskDreamHost {
    fn run_dream(&mut self, task_id: &TaskId, task: &DreamTaskData) -> Result<DreamStep, String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DefaultDreamHost;

impl TaskDreamHost for DefaultDreamHost {
    fn run_dream(&mut self, _task_id: &TaskId, task: &DreamTaskData) -> Result<DreamStep, String> {
        Ok(DreamStep::completed(
            DreamPhase::Updating,
            task.files_touched.clone(),
            Some(DreamTurn {
                text: format!("dream review {}", task.sessions_reviewing),
                tool_use_count: task.turns.len(),
            }),
        ))
    }
}

#[derive(Debug)]
pub struct LiveTaskRuntimeDriver<
    S = LiveTaskShellHost,
    A = InProcessAgentHost,
    D = DefaultDreamHost,
> {
    pub shell_host: S,
    pub agent_host: A,
    pub dream_host: D,
}

impl LiveTaskRuntimeDriver<LiveTaskShellHost, InProcessAgentHost, DefaultDreamHost> {
    pub fn new(agent_config: QueryEngineConfig) -> Self {
        Self {
            shell_host: LiveTaskShellHost,
            agent_host: InProcessAgentHost::new(agent_config),
            dream_host: DefaultDreamHost,
        }
    }
}

impl<S, A, D> LiveTaskRuntimeDriver<S, A, D> {
    pub fn with_hosts(shell_host: S, agent_host: A, dream_host: D) -> Self {
        Self {
            shell_host,
            agent_host,
            dream_host,
        }
    }
}

impl<S: TaskShellHost, A: TaskAgentHost, D: TaskDreamHost> TaskRuntimeDriver
    for LiveTaskRuntimeDriver<S, A, D>
{
    fn run_local_shell(
        &mut self,
        _task_id: &TaskId,
        task: &LocalShellTaskData,
    ) -> Result<CommandResult, String> {
        self.shell_host.run_command(&task.command)
    }

    fn run_local_agent(
        &mut self,
        task_id: &TaskId,
        task: &LocalAgentTaskData,
    ) -> Result<AgentStep, String> {
        self.agent_host
            .run_agent(task_id, &task.agent_id, &task.prompt)
    }

    fn run_dream(&mut self, task_id: &TaskId, task: &DreamTaskData) -> Result<DreamStep, String> {
        self.dream_host.run_dream(task_id, task)
    }
}

#[derive(Debug, Default)]
pub struct TaskCoordinator {
    tasks: HashMap<TaskId, TaskRecord>,
    queue: VecDeque<TaskId>,
    counter: u64,
}

impl TaskCoordinator {
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
            queue: VecDeque::new(),
            counter: 0,
        }
    }

    fn allocate_id(&mut self, task_type: TaskType) -> TaskId {
        self.counter += 1;
        TaskId::new(task_type.prefix(), self.counter)
    }

    fn register(&mut self, payload: TaskPayload, description: String) -> TaskId {
        let task_type = payload.task_type();
        let id = self.allocate_id(task_type);
        let record = TaskRecord {
            base: TaskStateBase::new(id.clone(), task_type, description),
            payload,
        };
        self.queue.push_back(id.clone());
        self.tasks.insert(id.clone(), record);
        id
    }

    pub fn spawn_local_shell(
        &mut self,
        command: String,
        description: Option<String>,
        kind: Option<BashTaskKind>,
    ) -> TaskId {
        let payload = TaskPayload::LocalShell(LocalShellTaskData {
            command: command.clone(),
            kind: kind.unwrap_or(BashTaskKind::Bash),
            result: None,
        });
        let desc = description.unwrap_or(command);
        self.register(payload, desc)
    }

    pub fn spawn_local_agent(&mut self, agent_id: String, prompt: String) -> TaskId {
        let payload = TaskPayload::LocalAgent(LocalAgentTaskData {
            agent_id: agent_id.clone(),
            prompt: prompt.clone(),
            progress: AgentProgress::new(),
            retrieved: false,
            stream_events: Vec::new(),
            response_result: None,
            model_error: None,
        });
        let desc = format!("agent {}", agent_id);
        self.register(payload, desc)
    }

    pub fn spawn_dream(
        &mut self,
        sessions_reviewing: usize,
        description: Option<String>,
    ) -> TaskId {
        let payload = TaskPayload::Dream(DreamTaskData {
            phase: DreamPhase::Starting,
            sessions_reviewing,
            files_touched: Vec::new(),
            turns: Vec::new(),
        });
        self.register(
            payload,
            description.unwrap_or_else(|| "dreaming".to_string()),
        )
    }

    pub fn pending_count(&self) -> usize {
        self.queue.len()
    }

    pub fn next_pending(&mut self) -> Option<TaskId> {
        while let Some(task_id) = self.queue.pop_front() {
            if let Some(record) = self.tasks.get_mut(&task_id) {
                match record.base.status {
                    TaskStatus::Pending => {
                        record.base.mark_running();
                        return Some(task_id);
                    }
                    TaskStatus::Running => return Some(task_id),
                    TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Killed => {}
                }
            }
        }
        None
    }

    pub fn complete_task(&mut self, task_id: &TaskId) -> bool {
        self.tasks
            .get_mut(task_id)
            .map(|record| {
                record.base.mark_completed();
            })
            .is_some()
    }

    pub fn fail_task(&mut self, task_id: &TaskId) -> bool {
        self.tasks
            .get_mut(task_id)
            .map(|record| {
                record.base.mark_failed();
            })
            .is_some()
    }

    pub fn record_shell_result(&mut self, task_id: &TaskId, result: CommandResult) -> bool {
        if let Some(record) = self.tasks.get_mut(task_id) {
            if record.base.status != TaskStatus::Running {
                return false;
            }
            if let TaskPayload::LocalShell(shell) = &mut record.payload {
                shell.result = Some(result);
                record.base.mark_completed();
                return true;
            }
        }
        false
    }

    pub fn update_agent_progress(
        &mut self,
        task_id: &TaskId,
        tool_use_delta: u32,
        token_delta: u32,
    ) -> bool {
        if let Some(record) = self.tasks.get_mut(task_id)
            && let TaskPayload::LocalAgent(agent) = &mut record.payload
        {
            agent.progress.tool_use_count += tool_use_delta;
            agent.progress.token_count += token_delta;
            return true;
        }
        false
    }

    pub fn append_dream_turn(&mut self, task_id: &TaskId, turn: DreamTurn) -> bool {
        if let Some(record) = self.tasks.get_mut(task_id)
            && let TaskPayload::Dream(dream) = &mut record.payload
        {
            dream.turns.push(turn);
            return true;
        }
        false
    }

    pub fn record_agent_step(&mut self, task_id: &TaskId, step: AgentStep) -> bool {
        if let Some(record) = self.tasks.get_mut(task_id)
            && let TaskPayload::LocalAgent(agent) = &mut record.payload
        {
            if record.base.status.is_terminal() {
                return false;
            }
            agent.progress.tool_use_count += step.tool_use_delta;
            agent.progress.token_count += step.token_delta;
            agent.retrieved |= step.retrieved;
            if !step.stream_events.is_empty() {
                agent.stream_events.extend(step.stream_events);
            }
            if let Some(response_result) = step.response_result {
                agent.response_result = Some(response_result);
            }
            if let Some(model_error) = step.model_error {
                agent.model_error = Some(model_error);
            }
            apply_task_status(&mut record.base, step.status);
            return true;
        }
        false
    }

    pub fn record_dream_step(&mut self, task_id: &TaskId, step: DreamStep) -> bool {
        if let Some(record) = self.tasks.get_mut(task_id)
            && let TaskPayload::Dream(dream) = &mut record.payload
        {
            if record.base.status.is_terminal() {
                return false;
            }
            dream.phase = step.phase;
            if !step.files_touched.is_empty() {
                dream.files_touched = step.files_touched;
            }
            if let Some(turn) = step.turn {
                dream.turns.push(turn);
            }
            apply_task_status(&mut record.base, step.status);
            return true;
        }
        false
    }

    pub fn queue_snapshot(&self) -> Vec<TaskId> {
        self.queue.iter().cloned().collect()
    }

    pub fn list_tasks(&self) -> Vec<TaskRecord> {
        self.tasks.values().cloned().collect()
    }

    pub fn record(&self, task_id: &TaskId) -> Option<&TaskRecord> {
        self.tasks.get(task_id)
    }

    pub fn record_mut(&mut self, task_id: &TaskId) -> Option<&mut TaskRecord> {
        self.tasks.get_mut(task_id)
    }

    pub fn drive_next(
        &mut self,
        driver: &mut (impl TaskRuntimeDriver + ?Sized),
    ) -> Result<Option<TaskDriveReport>, TaskDriveError> {
        let Some(task_id) = self.next_pending() else {
            return Ok(None);
        };
        let record = self
            .record(&task_id)
            .cloned()
            .ok_or_else(|| TaskDriveError::MissingTask {
                task_id: task_id.clone(),
            })?;

        let report = match &record.payload {
            TaskPayload::LocalShell(shell) => match driver.run_local_shell(&task_id, shell) {
                Ok(result) => {
                    let activity = Some(render_shell_activity(&result));
                    self.record_shell_result(&task_id, result);
                    self.build_drive_report(&task_id, activity)?
                }
                Err(message) => return Err(self.fail_drive(&task_id, message)),
            },
            TaskPayload::LocalAgent(agent) => match driver.run_local_agent(&task_id, agent) {
                Ok(step) => {
                    let activity = render_agent_activity(&step);
                    self.record_agent_step(&task_id, step);
                    self.build_drive_report(&task_id, activity)?
                }
                Err(message) => return Err(self.fail_drive(&task_id, message)),
            },
            TaskPayload::Dream(dream) => match driver.run_dream(&task_id, dream) {
                Ok(step) => {
                    let activity = render_dream_activity(&step);
                    self.record_dream_step(&task_id, step);
                    self.build_drive_report(&task_id, activity)?
                }
                Err(message) => return Err(self.fail_drive(&task_id, message)),
            },
        };

        if report.status == TaskStatus::Running {
            self.queue.push_front(task_id.clone());
        }

        Ok(Some(report))
    }

    pub fn drive_all(
        &mut self,
        driver: &mut (impl TaskRuntimeDriver + ?Sized),
    ) -> Result<Vec<TaskDriveReport>, TaskDriveError> {
        let max_iterations = self.tasks.len().saturating_mul(8).max(1);
        self.drive_until_idle(driver, max_iterations)
    }

    pub fn drive_until_idle(
        &mut self,
        driver: &mut (impl TaskRuntimeDriver + ?Sized),
        max_iterations: usize,
    ) -> Result<Vec<TaskDriveReport>, TaskDriveError> {
        let mut reports = Vec::new();
        let mut iterations = 0usize;
        while let Some(report) = self.drive_next(driver)? {
            reports.push(report);
            iterations += 1;
            if iterations >= max_iterations {
                break;
            }
        }
        Ok(reports)
    }

    fn fail_drive(&mut self, task_id: &TaskId, message: String) -> TaskDriveError {
        let _ = self.fail_task(task_id);
        TaskDriveError::DriverFailure {
            task_id: task_id.clone(),
            message,
        }
    }

    fn build_drive_report(
        &self,
        task_id: &TaskId,
        activity: Option<String>,
    ) -> Result<TaskDriveReport, TaskDriveError> {
        let record = self
            .record(task_id)
            .ok_or_else(|| TaskDriveError::MissingTask {
                task_id: task_id.clone(),
            })?;
        Ok(TaskDriveReport {
            task_id: task_id.clone(),
            task_type: record.base.task_type,
            status: record.base.status,
            summary: record.base.description.clone(),
            activity,
            result_preview: record.result().map(|result| result.preview(192)),
        })
    }

    /// Resume tasks from persisted records. Running tasks from a prior session
    /// are marked as Failed (interrupted by restart). Completed/Failed/Killed
    /// tasks are restored as-is for history visibility.
    pub fn resume_from_persisted(
        &mut self,
        records: &[crate::session_persistence::PersistedTaskRecord],
    ) -> ResumeResult {
        let mut result = ResumeResult::default();
        for record in records {
            let task_type = match record.task_type.as_str() {
                "shell" | "local_shell" => TaskType::LocalShell,
                "agent" | "local_agent" => TaskType::LocalAgent,
                "dream" => TaskType::Dream,
                _ => {
                    result.skipped += 1;
                    continue;
                }
            };
            self.counter += 1;
            let id = TaskId::new(task_type.prefix(), self.counter);
            let status = match record.status.as_str() {
                "running" => {
                    // Running tasks from a dead session are failed-on-resume.
                    result.interrupted += 1;
                    TaskStatus::Failed
                }
                "completed" => TaskStatus::Completed,
                "failed" => TaskStatus::Failed,
                "killed" => TaskStatus::Killed,
                "pending" => {
                    // Re-queue pending tasks.
                    result.requeued += 1;
                    TaskStatus::Pending
                }
                _ => {
                    result.skipped += 1;
                    continue;
                }
            };
            let mut base = TaskStateBase::new(id.clone(), task_type, record.summary.clone());
            base.status = status;
            base.start_time = record.timestamp_ms;
            if status != TaskStatus::Pending {
                base.end_time = Some(record.timestamp_ms);
            }
            // Restore as shell stub — payload details are lost across sessions.
            let payload = TaskPayload::LocalShell(LocalShellTaskData {
                command: record.summary.clone(),
                kind: BashTaskKind::Bash,
                result: match status {
                    TaskStatus::Completed => Some(CommandResult {
                        code: 0,
                        interrupted: false,
                    }),
                    TaskStatus::Failed => Some(CommandResult {
                        code: -1,
                        interrupted: true,
                    }),
                    _ => None,
                },
            });
            let task_record = TaskRecord { base, payload };
            if status == TaskStatus::Pending {
                self.queue.push_back(id.clone());
            }
            self.tasks.insert(id, task_record);
            result.restored += 1;
        }
        result
    }
}

/// Summary of a task resume operation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResumeResult {
    /// Total tasks restored into coordinator.
    pub restored: usize,
    /// Running tasks that were marked Failed (interrupted by restart).
    pub interrupted: usize,
    /// Pending tasks that were re-queued.
    pub requeued: usize,
    /// Records skipped due to unknown type/status.
    pub skipped: usize,
}

fn render_shell_activity(result: &CommandResult) -> String {
    format!("code={} interrupted={}", result.code, result.interrupted)
}

fn render_agent_activity(step: &AgentStep) -> Option<String> {
    let mut parts = Vec::new();
    if step.tool_use_delta > 0 || step.token_delta > 0 {
        parts.push(format!(
            "progress=tools+{} tokens+{}",
            step.tool_use_delta, step.token_delta
        ));
    }
    if step.retrieved {
        parts.push(String::from("retrieved=true"));
    }
    for event in &step.stream_events {
        parts.push(render_stream_event_activity(event));
    }
    if let Some(response_result) = &step.response_result {
        parts.push(format!(
            "response-result={}",
            truncate_preview(response_result.to_string(), 80)
        ));
    }
    if let Some(model_error) = &step.model_error {
        parts.push(format!(
            "model-error={} retryable={} message={}",
            model_error.kind,
            model_error.retryable,
            truncate_preview(model_error.message.clone(), 80)
        ));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

fn render_stream_event_activity(event: &ModelStreamEventWire) -> String {
    match event {
        ModelStreamEventWire::Start { provider, model } => {
            format!("stream=start:{provider}/{model}")
        }
        ModelStreamEventWire::Delta { text, .. } => {
            format!("stream=delta:{}", truncate_preview(text.clone(), 64))
        }
        ModelStreamEventWire::StreamError { message, .. } => {
            format!("stream=error:{}", truncate_preview(message.clone(), 64))
        }
        ModelStreamEventWire::Complete { role, content } => format!(
            "stream=complete:{role}:{}",
            truncate_preview(content.clone(), 64)
        ),
    }
}

fn render_dream_activity(step: &DreamStep) -> Option<String> {
    let mut parts = vec![format!("phase={}", step.phase.as_str())];
    if !step.files_touched.is_empty() {
        parts.push(format!("files={}", step.files_touched.len()));
    }
    if let Some(turn) = &step.turn {
        parts.push(format!(
            "turn={} tools={}",
            truncate_preview(turn.text.clone(), 48),
            turn.tool_use_count
        ));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

fn apply_task_status(base: &mut TaskStateBase, status: TaskStatus) {
    match status {
        TaskStatus::Pending => {}
        TaskStatus::Running => base.mark_running(),
        TaskStatus::Completed => base.mark_completed(),
        TaskStatus::Failed => base.mark_failed(),
        TaskStatus::Killed => base.mark_killed(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopTaskError {
    NotFound,
    NotRunning,
    UnsupportedType,
    KillFailed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopTaskResult {
    pub task_id: TaskId,
    pub task_type: TaskType,
    pub summary: String,
    pub command: Option<String>,
}

pub fn stop_task(
    coordinator: &mut TaskCoordinator,
    task_id: &TaskId,
) -> Result<StopTaskResult, StopTaskError> {
    let record = coordinator
        .record_mut(task_id)
        .ok_or(StopTaskError::NotFound)?;
    if record.base.status != TaskStatus::Running {
        return Err(StopTaskError::NotRunning);
    }

    // Extract info before marking killed.
    let task_type = record.base.task_type;
    let summary = record.base.description.clone();
    let command = match &record.payload {
        TaskPayload::LocalShell(shell) => Some(shell.command.clone()),
        _ => None,
    };

    // Mark shell tasks as interrupted so callers know it was user-initiated.
    if let TaskPayload::LocalShell(ref mut shell) = record.payload
        && shell.result.is_none()
    {
        shell.result = Some(CommandResult {
            code: -1,
            interrupted: true,
        });
    }

    record.base.mark_killed();

    Ok(StopTaskResult {
        task_id: task_id.clone(),
        task_type,
        summary,
        command,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::QueryMessage;
    use crate::provider::ModelProvider;
    use crate::query_engine::{QueryEngineConfig, ThinkingMode};
    use crate::query_loop::TaskBudget;
    use crate::tool_registry::{ToolPermissionContext, ToolRuntimeMode};
    use serde_json::json;
    use std::cell::RefCell;
    use std::process::Command as OsCommand;
    use std::rc::Rc;

    #[derive(Default)]
    struct StubDriver;

    impl TaskRuntimeDriver for StubDriver {
        fn run_local_shell(
            &mut self,
            _task_id: &TaskId,
            _task: &LocalShellTaskData,
        ) -> Result<CommandResult, String> {
            Ok(CommandResult {
                code: 0,
                interrupted: false,
            })
        }

        fn run_local_agent(
            &mut self,
            _task_id: &TaskId,
            task: &LocalAgentTaskData,
        ) -> Result<AgentStep, String> {
            if task.prompt.contains("done") {
                Ok(AgentStep::completed(1, 32, true))
            } else {
                Ok(AgentStep::progress(1, 32))
            }
        }

        fn run_dream(
            &mut self,
            _task_id: &TaskId,
            task: &DreamTaskData,
        ) -> Result<DreamStep, String> {
            Ok(DreamStep::completed(
                DreamPhase::Updating,
                vec![format!("session-{}", task.sessions_reviewing)],
                Some(DreamTurn {
                    text: String::from("vision"),
                    tool_use_count: 1,
                }),
            ))
        }
    }

    struct FailingDriver;

    impl TaskRuntimeDriver for FailingDriver {
        fn run_local_shell(
            &mut self,
            _task_id: &TaskId,
            _task: &LocalShellTaskData,
        ) -> Result<CommandResult, String> {
            Err(String::from("shell failed"))
        }

        fn run_local_agent(
            &mut self,
            _task_id: &TaskId,
            _task: &LocalAgentTaskData,
        ) -> Result<AgentStep, String> {
            Err(String::from("agent failed"))
        }

        fn run_dream(
            &mut self,
            _task_id: &TaskId,
            _task: &DreamTaskData,
        ) -> Result<DreamStep, String> {
            Err(String::from("dream failed"))
        }
    }

    #[derive(Debug, Clone)]
    struct RecordingShellHost {
        commands: Rc<RefCell<Vec<String>>>,
        result: CommandResult,
    }

    impl TaskShellHost for RecordingShellHost {
        fn run_command(&mut self, command: &str) -> Result<CommandResult, String> {
            self.commands.borrow_mut().push(command.to_string());
            Ok(self.result.clone())
        }
    }

    #[derive(Debug, Clone)]
    struct RecordingAgentHost {
        calls: Rc<RefCell<Vec<(String, String)>>>,
        step: AgentStep,
    }

    impl TaskAgentHost for RecordingAgentHost {
        fn run_agent(
            &mut self,
            _task_id: &TaskId,
            agent_id: &str,
            prompt: &str,
        ) -> Result<AgentStep, String> {
            self.calls
                .borrow_mut()
                .push((agent_id.to_string(), prompt.to_string()));
            Ok(self.step.clone())
        }
    }

    #[derive(Debug, Clone)]
    struct RecordingDreamHost {
        calls: Rc<RefCell<Vec<(String, usize)>>>,
        step: DreamStep,
    }

    impl TaskDreamHost for RecordingDreamHost {
        fn run_dream(
            &mut self,
            task_id: &TaskId,
            task: &DreamTaskData,
        ) -> Result<DreamStep, String> {
            self.calls
                .borrow_mut()
                .push((task_id.as_str().to_string(), task.sessions_reviewing));
            Ok(self.step.clone())
        }
    }

    #[derive(Default)]
    struct RequeueDriver {
        attempts: usize,
    }

    impl TaskRuntimeDriver for RequeueDriver {
        fn run_local_shell(
            &mut self,
            _task_id: &TaskId,
            _task: &LocalShellTaskData,
        ) -> Result<CommandResult, String> {
            Ok(CommandResult {
                code: 0,
                interrupted: false,
            })
        }

        fn run_local_agent(
            &mut self,
            _task_id: &TaskId,
            _task: &LocalAgentTaskData,
        ) -> Result<AgentStep, String> {
            self.attempts += 1;
            if self.attempts >= 2 {
                Ok(AgentStep::completed(1, 16, true)
                    .with_response_result(json!({"ok": true, "source": "requeue"})))
            } else {
                Ok(AgentStep::progress(1, 8).with_stream_events(vec![
                    ModelStreamEventWire::Delta {
                        text: String::from("loop delta"),
                        sequence: 0,
                        timestamp_ms: 0,
                        chunk_bytes: 10,
                    },
                ]))
            }
        }

        fn run_dream(
            &mut self,
            _task_id: &TaskId,
            _task: &DreamTaskData,
        ) -> Result<DreamStep, String> {
            Ok(DreamStep::completed(DreamPhase::Updating, Vec::new(), None))
        }
    }

    fn sample_agent_config() -> QueryEngineConfig {
        QueryEngineConfig {
            cwd: String::from("/tmp"),
            session_id: String::from("task-runtime"),
            persist_session: false,
            persist_history: false,
            file_history_enabled: false,
            tools: vec![String::from("Read")],
            tool_runtime_mode: ToolRuntimeMode::Standard,
            tool_permission_context: ToolPermissionContext::default(),
            commands: vec![String::from("/help")],
            mcp_clients: Vec::new(),
            agents: vec![String::from("worker")],
            initial_messages: vec![QueryMessage::system("seed")],
            read_file_cache_entries: 0,
            custom_system_prompt: Some(String::from("custom")),
            append_system_prompt: None,
            model_provider: ModelProvider::Mock,
            user_specified_model: Some(String::from("sonnet")),
            fallback_model: Some(String::from("haiku")),
            model_reasoning_effort: None,
            thinking_mode: ThinkingMode::Adaptive,
            max_turns: Some(2),
            max_budget_usd: None,
            task_budget: Some(TaskBudget { total: 10_000 }),
            json_schema: None,
            verbose: false,
            replay_user_messages: false,
            include_partial_messages: false,
            stream_model_responses: true,
        }
    }

    #[test]
    fn coordinator_schedules_pending_tasks() {
        let mut coordinator = TaskCoordinator::new();
        let shell = coordinator.spawn_local_shell("ls".into(), None, None);
        let agent = coordinator.spawn_local_agent("agent-a".into(), "prompt".into());
        assert_eq!(coordinator.pending_count(), 2);

        let first = coordinator.next_pending();
        assert_eq!(first.as_ref(), Some(&shell));
        assert_eq!(
            coordinator.record(&shell).unwrap().base.status,
            TaskStatus::Running
        );

        let second = coordinator.next_pending();
        assert_eq!(second.as_ref(), Some(&agent));
        assert_eq!(
            coordinator.record(&agent).unwrap().base.status,
            TaskStatus::Running
        );

        assert!(coordinator.next_pending().is_none());

        assert!(coordinator.complete_task(&shell));
        assert_eq!(
            coordinator.record(&shell).unwrap().base.status,
            TaskStatus::Completed
        );
    }

    #[test]
    fn stop_task_runs_only_when_running() {
        let mut coordinator = TaskCoordinator::new();
        let shell = coordinator.spawn_local_shell("echo hi".into(), None, None);
        assert_eq!(
            stop_task(&mut coordinator, &shell),
            Err(StopTaskError::NotRunning)
        );

        coordinator.next_pending();

        let result = stop_task(&mut coordinator, &shell).expect("should stop running task");
        assert_eq!(result.command.as_deref(), Some("echo hi"));
        assert_eq!(
            coordinator.record(&shell).unwrap().base.status,
            TaskStatus::Killed
        );
        assert!(result.summary.contains("echo hi"));
    }

    #[test]
    fn stop_task_missing_task() {
        let mut coordinator = TaskCoordinator::new();
        let phantom = TaskId::new("x", 999);
        assert_eq!(
            stop_task(&mut coordinator, &phantom),
            Err(StopTaskError::NotFound)
        );
    }

    #[test]
    fn record_shell_result_completes_running_task_and_records_result() {
        let mut coordinator = TaskCoordinator::new();
        let shell = coordinator.spawn_local_shell("echo hi".into(), None, None);
        assert!(coordinator.next_pending().is_some());
        let result = CommandResult {
            code: 0,
            interrupted: false,
        };

        assert!(coordinator.record_shell_result(&shell, result.clone()));
        let record = coordinator.record(&shell).unwrap();
        assert_eq!(record.base.status, TaskStatus::Completed);
        assert_eq!(
            record.payload,
            TaskPayload::LocalShell(LocalShellTaskData {
                command: String::from("echo hi"),
                kind: BashTaskKind::Bash,
                result: Some(result)
            })
        );
    }

    #[test]
    fn update_agent_progress_accumulates_values() {
        let mut coordinator = TaskCoordinator::new();
        let agent = coordinator.spawn_local_agent("agent-a".into(), "prompt".into());
        coordinator.next_pending();

        assert!(coordinator.update_agent_progress(&agent, 2, 100));
        let record = coordinator.record(&agent).unwrap();
        if let TaskPayload::LocalAgent(agent) = &record.payload {
            assert_eq!(agent.progress.tool_use_count, 2);
            assert_eq!(agent.progress.token_count, 100);
        } else {
            panic!("expected local agent");
        }
    }

    #[test]
    fn record_agent_step_can_complete_and_mark_retrieved() {
        let mut coordinator = TaskCoordinator::new();
        let agent = coordinator.spawn_local_agent("agent-z".into(), "done".into());
        coordinator.next_pending();

        assert!(
            coordinator.record_agent_step(
                &agent,
                AgentStep::completed(2, 64, true)
                    .with_response_result(json!({"ok": true, "source": "task"}))
            )
        );
        let record = coordinator.record(&agent).unwrap();
        assert_eq!(record.base.status, TaskStatus::Completed);
        if let TaskPayload::LocalAgent(agent) = &record.payload {
            assert_eq!(agent.progress.tool_use_count, 2);
            assert_eq!(agent.progress.token_count, 64);
            assert!(agent.retrieved);
            assert_eq!(
                agent.response_result,
                Some(json!({"ok": true, "source": "task"}))
            );
            assert_eq!(
                agent.response_result_preview(64),
                "{\"ok\":true,\"source\":\"task\"}"
            );
        } else {
            panic!("expected agent task");
        }
    }

    #[test]
    fn build_drive_report_surfaces_agent_result_preview() {
        let mut coordinator = TaskCoordinator::new();
        let agent = coordinator.spawn_local_agent("agent-z".into(), "done".into());
        coordinator.next_pending();
        assert!(
            coordinator.record_agent_step(
                &agent,
                AgentStep::completed(2, 64, true)
                    .with_response_result(json!({"ok": true, "source": "drive-report"}))
            )
        );

        let report = coordinator
            .build_drive_report(&agent, Some(String::from("response-result={\"ok\":true}")))
            .expect("drive report should build");

        assert_eq!(
            report.activity.as_deref(),
            Some("response-result={\"ok\":true}")
        );
        let preview = report.result_preview.expect("result preview should exist");
        assert!(preview.starts_with("{\"kind\":\"agent\""));
        assert!(preview.contains("\"response_result\""));
    }

    #[test]
    fn append_dream_turn_pushes_turn_data() {
        let mut coordinator = TaskCoordinator::new();
        let dream = coordinator.spawn_dream(1, Some("dream".to_string()));
        let turn = DreamTurn {
            text: String::from("vision"),
            tool_use_count: 0,
        };

        assert!(coordinator.append_dream_turn(&dream, turn.clone()));
        let record = coordinator.record(&dream).unwrap();
        if let TaskPayload::Dream(dream) = &record.payload {
            assert_eq!(dream.turns.len(), 1);
            assert_eq!(dream.turns[0], turn);
        } else {
            panic!("expected dream");
        }
    }

    #[test]
    fn record_dream_step_updates_phase_files_and_turns() {
        let mut coordinator = TaskCoordinator::new();
        let dream = coordinator.spawn_dream(2, None);
        coordinator.next_pending();

        assert!(coordinator.record_dream_step(
            &dream,
            DreamStep::progress(
                DreamPhase::Updating,
                vec![String::from("a.rs"), String::from("b.rs")],
                Some(DreamTurn {
                    text: String::from("trace"),
                    tool_use_count: 2,
                }),
            ),
        ));
        let record = coordinator.record(&dream).unwrap();
        assert_eq!(record.base.status, TaskStatus::Running);
        if let TaskPayload::Dream(dream) = &record.payload {
            assert_eq!(dream.phase, DreamPhase::Updating);
            assert_eq!(
                dream.files_touched,
                vec![String::from("a.rs"), String::from("b.rs")]
            );
            assert_eq!(dream.turns.len(), 1);
        } else {
            panic!("expected dream task");
        }
    }

    #[test]
    fn snapshots_and_listings_reflect_state() {
        let mut coordinator = TaskCoordinator::new();
        let shell = coordinator.spawn_local_shell("echo".into(), None, None);
        let agent = coordinator.spawn_local_agent("agent-b".into(), "prompt".into());
        let dream = coordinator.spawn_dream(0, None);

        let queue = coordinator.queue_snapshot();
        assert_eq!(queue, vec![shell.clone(), agent.clone(), dream.clone()]);

        let tasks = coordinator.list_tasks();
        assert_eq!(tasks.len(), 3);
        assert!(tasks.iter().any(|record| record.base.id == shell));
        assert!(tasks.iter().any(|record| record.base.id == agent));
        assert!(tasks.iter().any(|record| record.base.id == dream));
    }

    #[test]
    fn drive_next_executes_pending_shell_task() {
        let mut coordinator = TaskCoordinator::new();
        let shell = coordinator.spawn_local_shell("echo hi".into(), None, None);
        let mut driver = StubDriver;

        let report = coordinator
            .drive_next(&mut driver)
            .expect("drive should succeed")
            .expect("report should exist");

        assert_eq!(report.task_id, shell);
        assert_eq!(report.status, TaskStatus::Completed);
        assert_eq!(
            coordinator.record(&shell).unwrap().base.status,
            TaskStatus::Completed
        );
    }

    #[test]
    fn drive_all_runs_remaining_queue_in_order() {
        let mut coordinator = TaskCoordinator::new();
        let shell = coordinator.spawn_local_shell("echo".into(), None, None);
        let agent = coordinator.spawn_local_agent("agent-a".into(), "done".into());
        let dream = coordinator.spawn_dream(3, Some(String::from("dream")));
        let mut driver = StubDriver;

        let reports = coordinator
            .drive_all(&mut driver)
            .expect("drive all should succeed");

        assert_eq!(
            reports
                .iter()
                .map(|report| report.task_id.clone())
                .collect::<Vec<_>>(),
            vec![shell.clone(), agent.clone(), dream.clone()]
        );
        assert_eq!(
            coordinator.record(&shell).unwrap().base.status,
            TaskStatus::Completed
        );
        assert_eq!(
            coordinator.record(&agent).unwrap().base.status,
            TaskStatus::Completed
        );
        assert_eq!(
            coordinator.record(&dream).unwrap().base.status,
            TaskStatus::Completed
        );
        if let TaskPayload::Dream(dream_payload) = &coordinator.record(&dream).unwrap().payload {
            assert_eq!(dream_payload.turns.len(), 1);
        } else {
            panic!("expected dream payload");
        }
    }

    #[test]
    fn drive_until_idle_requeues_running_tasks_until_completion() {
        let mut coordinator = TaskCoordinator::new();
        let agent = coordinator.spawn_local_agent("agent-loop".into(), "keep going".into());
        let mut driver = RequeueDriver::default();

        let reports = coordinator
            .drive_until_idle(&mut driver, 4)
            .expect("drive until idle should succeed");

        assert_eq!(reports.len(), 2);
        assert_eq!(reports[0].task_id, agent);
        assert_eq!(reports[0].status, TaskStatus::Running);
        assert_eq!(reports[1].status, TaskStatus::Completed);
        assert_eq!(
            reports[0].activity.as_deref(),
            Some("progress=tools+1 tokens+8 stream=delta:loop delta")
        );
        assert!(reports[1].activity.as_deref().is_some_and(|activity| {
            activity.contains("response-result={\"ok\":true,\"source\":\"requeue\"}")
        }));
        assert_eq!(driver.attempts, 2);
        assert_eq!(coordinator.pending_count(), 0);
        let record = coordinator
            .record(&agent)
            .expect("agent record should exist");
        assert_eq!(record.base.status, TaskStatus::Completed);
        if let TaskPayload::LocalAgent(agent) = &record.payload {
            assert_eq!(agent.progress.tool_use_count, 2);
            assert_eq!(agent.progress.token_count, 24);
            assert!(agent.retrieved);
            assert_eq!(agent.stream_events.len(), 1);
            assert_eq!(
                agent.response_result,
                Some(json!({"ok": true, "source": "requeue"}))
            );
        } else {
            panic!("expected agent payload");
        }
    }

    #[test]
    fn live_task_runtime_driver_delegates_to_each_host() {
        let shell_calls = Rc::new(RefCell::new(Vec::new()));
        let agent_calls = Rc::new(RefCell::new(Vec::new()));
        let dream_calls = Rc::new(RefCell::new(Vec::new()));
        let shell_host = RecordingShellHost {
            commands: shell_calls.clone(),
            result: CommandResult {
                code: 0,
                interrupted: false,
            },
        };
        let agent_host = RecordingAgentHost {
            calls: agent_calls.clone(),
            step: AgentStep::completed(2, 48, true),
        };
        let dream_host = RecordingDreamHost {
            calls: dream_calls.clone(),
            step: DreamStep::completed(
                DreamPhase::Updating,
                vec![String::from("bridge_runtime.rs")],
                Some(DreamTurn {
                    text: String::from("vision"),
                    tool_use_count: 1,
                }),
            ),
        };
        let mut driver = LiveTaskRuntimeDriver::with_hosts(shell_host, agent_host, dream_host);
        let mut coordinator = TaskCoordinator::new();
        let shell = coordinator.spawn_local_shell("printf ready".into(), None, None);
        let agent = coordinator.spawn_local_agent("agent-live".into(), "done".into());
        let dream = coordinator.spawn_dream(2, Some(String::from("dream-live")));

        let reports = coordinator
            .drive_all(&mut driver)
            .expect("live driver should run all tasks");

        assert_eq!(reports.len(), 3);
        assert_eq!(
            shell_calls.borrow().as_slice(),
            &[String::from("printf ready")]
        );
        assert_eq!(
            agent_calls.borrow().as_slice(),
            &[(String::from("agent-live"), String::from("done"))]
        );
        assert_eq!(dream_calls.borrow().len(), 1);
        assert_eq!(
            coordinator.record(&shell).unwrap().base.status,
            TaskStatus::Completed
        );
        assert_eq!(
            coordinator.record(&agent).unwrap().base.status,
            TaskStatus::Completed
        );
        assert_eq!(
            coordinator.record(&dream).unwrap().base.status,
            TaskStatus::Completed
        );
    }

    #[test]
    fn in_process_agent_host_uses_query_engine_for_agent_turns() {
        let mut host = InProcessAgentHost::new(sample_agent_config());
        let task_id = TaskId::new("a", 1);

        let first = host
            .run_agent(&task_id, "agent-alpha", "bridge prompt")
            .expect("agent host should submit first stream step");
        let second = host
            .run_agent(&task_id, "agent-alpha", "bridge prompt")
            .expect("agent host should submit second stream step");
        let third = host
            .run_agent(&task_id, "agent-alpha", "bridge prompt")
            .expect("agent host should submit third stream step");
        let final_step = host
            .run_agent(&task_id, "agent-alpha", "bridge prompt")
            .expect("agent host should submit final step");

        assert_eq!(first.status, TaskStatus::Running);
        assert_eq!(second.status, TaskStatus::Running);
        assert_eq!(third.status, TaskStatus::Running);
        assert_eq!(final_step.status, TaskStatus::Completed);
        assert!(final_step.retrieved);
        assert!(final_step.token_delta > 0);
        assert_eq!(first.stream_events.len(), 1);
        assert_eq!(second.stream_events.len(), 1);
        assert_eq!(third.stream_events.len(), 1);
        assert!(final_step.stream_events.is_empty());
        assert_eq!(host.engines.len(), 1);
        assert!(host.engines.contains_key("agent-alpha"));
    }

    #[test]
    fn in_process_agent_host_surfaces_model_error() {
        let mut config = sample_agent_config();
        config.json_schema = Some(String::from("{\"type\":\"object\"}"));
        let mut host = InProcessAgentHost::new(config);
        let task_id = TaskId::new("a", 2);

        let step = host
            .run_agent(&task_id, "agent-beta", "bridge prompt")
            .expect("agent host should still return terminal step");

        assert_eq!(step.status, TaskStatus::Failed);
        assert!(step.model_error.is_some());
        assert!(step.stream_events.is_empty());
    }

    #[test]
    fn process_agent_wire_round_trip_json() {
        let request = ProcessAgentRequestWire::new("daemon-a", "route prompt");
        let request_json = request.to_json().expect("request should serialize");
        let decoded_request =
            ProcessAgentRequestWire::from_json(&request_json).expect("request should deserialize");
        assert_eq!(decoded_request, request);

        let response = ProcessAgentResponseWire::new(2, 88, true, ProcessAgentStatusWire::Running)
            .with_stream_events(vec![ModelStreamEventWire::Delta {
                text: String::from("wire delta"),
                sequence: 0,
                timestamp_ms: 0,
                chunk_bytes: 10,
            }])
            .with_response_result(json!({"ok": true, "source": "wire"}));
        let response_json = response.to_json().expect("response should serialize");
        assert!(response_json.contains("\"response_result\""));
        assert!(response_json.contains("\"stream_events\""));
        let decoded_response = ProcessAgentResponseWire::from_json(&response_json)
            .expect("response should deserialize");
        assert_eq!(decoded_response, response);
        assert_eq!(
            decoded_response.response_result,
            Some(json!({"ok": true, "source": "wire"}))
        );
        assert_eq!(decoded_response.stream_events.len(), 1);
        assert_eq!(
            decoded_response.clone().into_agent_step().status,
            TaskStatus::Running
        );
    }

    #[test]
    fn process_agent_output_wire_round_trip_json() {
        let output = ProcessAgentOutputWire::event(ModelStreamEventWire::Delta {
            text: String::from("wire delta"),
            sequence: 0,
            timestamp_ms: 0,
            chunk_bytes: 10,
        });
        let encoded = output.to_json().expect("output wire should serialize");
        let decoded =
            ProcessAgentOutputWire::from_json(&encoded).expect("output wire should deserialize");
        assert_eq!(decoded, output);
    }

    #[test]
    fn process_agent_wire_accepts_legacy_structured_output_alias() {
        let decoded = ProcessAgentResponseWire::from_json(
            r#"{
                "tool_use_delta": 2,
                "token_delta": 88,
                "retrieved": true,
                "status": "running",
                "structured_output": {"ok": true, "source": "legacy-wire"}
            }"#,
        )
        .expect("legacy response wire should deserialize");

        assert_eq!(
            decoded.response_result,
            Some(json!({"ok": true, "source": "legacy-wire"}))
        );
    }

    #[test]
    fn process_task_agent_host_runs_external_process() {
        if OsCommand::new("python3").arg("--version").output().is_err() {
            return;
        }

        let script = r#"import json, sys
request = json.load(sys.stdin)
response = {
    "tool_use_delta": len(request.get("agent_id", "")),
    "token_delta": 41,
    "retrieved": True,
    "status": "completed",
}
json.dump(response, sys.stdout)
"#;
        let mut host = ProcessTaskAgentHost::with_args(
            "python3",
            vec![String::from("-c"), script.to_string()],
        );
        let step = host
            .run_agent(&TaskId::new("a", 3), "daemon-a", "remote prompt")
            .expect("process host should produce agent step");

        assert_eq!(step.status, TaskStatus::Completed);
        assert_eq!(step.tool_use_delta, 8);
        assert_eq!(step.token_delta, 41);
        assert!(step.retrieved);
    }

    #[test]
    fn process_task_agent_host_runs_external_process_multiframe() {
        if OsCommand::new("python3").arg("--version").output().is_err() {
            return;
        }

        let script = r#"import json, sys
request = json.load(sys.stdin)
frames = [
    {"kind": "event", "event": {"kind": "start", "provider": "mock", "model": "worker"}},
    {"kind": "event", "event": {"kind": "delta", "text": request.get("prompt", ""), "sequence": 1, "timestamp_ms": 0, "chunk_bytes": 10}},
    {"kind": "complete", "response": {
        "tool_use_delta": len(request.get("agent_id", "")),
        "token_delta": 9,
        "retrieved": True,
        "status": "completed"
    }},
]
for frame in frames:
    sys.stdout.write(json.dumps(frame) + "\n")
"#;
        let mut host = ProcessTaskAgentHost::with_args(
            "python3",
            vec![String::from("-c"), script.to_string()],
        );
        let task_id = TaskId::new("a", 4);
        let start = host
            .run_agent(&task_id, "daemon-a", "remote prompt")
            .expect("process host should consume first multiframe output");
        let delta = host
            .run_agent(&task_id, "daemon-a", "remote prompt")
            .expect("process host should consume second multiframe output");
        let step = host
            .run_agent(&task_id, "daemon-a", "remote prompt")
            .expect("process host should consume completion output");

        assert_eq!(start.status, TaskStatus::Running);
        assert_eq!(delta.status, TaskStatus::Running);
        assert_eq!(step.status, TaskStatus::Completed);
        assert_eq!(step.tool_use_delta, 8);
        assert_eq!(step.token_delta, 9);
        assert_eq!(start.stream_events.len(), 1);
        assert_eq!(delta.stream_events.len(), 1);
    }

    #[test]
    fn process_task_agent_host_daemon_mode_reuses_child_process() {
        if OsCommand::new("python3").arg("--version").output().is_err() {
            return;
        }

        let script = r#"import json, sys
count = 0
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    count += 1
    request = json.loads(line)
    response = {
        "tool_use_delta": len(request.get("agent_id", "")),
        "token_delta": count,
        "retrieved": True,
        "status": "completed",
    }
    json.dump(response, sys.stdout)
    sys.stdout.write("\n")
    sys.stdout.flush()
"#;
        let mut host = ProcessTaskAgentHost::with_args(
            "python3",
            vec![String::from("-u"), String::from("-c"), script.to_string()],
        )
        .with_daemon_mode();

        let task_id = TaskId::new("a", 5);
        let first = host
            .run_agent(&task_id, "daemon-a", "first prompt")
            .expect("daemon host should produce first step");
        let second = host
            .run_agent(&TaskId::new("a", 6), "daemon-a", "second prompt")
            .expect("daemon host should produce second step");

        assert_eq!(first.status, TaskStatus::Completed);
        assert_eq!(second.status, TaskStatus::Completed);
        assert_eq!(first.token_delta, 1);
        assert_eq!(second.token_delta, 2);
        assert!(host.daemon_running());
        assert_eq!(host.request_count(), 2);
        assert_eq!(host.spawn_count(), 1);
        assert_eq!(host.restart_count(), 0);
        assert_eq!(host.consecutive_failure_count(), 0);
        assert_eq!(host.last_backoff_ms(), 0);
        assert_eq!(host.last_failure_kind(), None);
        assert_eq!(host.last_error(), None);
    }

    #[test]
    fn process_task_agent_host_daemon_mode_consumes_multiframe_streams() {
        if OsCommand::new("python3").arg("--version").output().is_err() {
            return;
        }

        let script = r#"import json, sys
count = 0
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    count += 1
    request = json.loads(line)
    sys.stdout.write(json.dumps({
        "kind": "event",
        "event": {"kind": "delta", "text": request.get("prompt", ""), "sequence": count, "timestamp_ms": 0, "chunk_bytes": 10}
    }) + "\n")
    sys.stdout.write(json.dumps({
        "kind": "complete",
        "response": {
            "tool_use_delta": len(request.get("agent_id", "")),
            "token_delta": count,
            "retrieved": True,
            "status": "completed"
        }
    }) + "\n")
    sys.stdout.flush()
"#;
        let mut host = ProcessTaskAgentHost::with_args(
            "python3",
            vec![String::from("-u"), String::from("-c"), script.to_string()],
        )
        .with_daemon_mode();

        let task_a = TaskId::new("a", 7);
        let task_b = TaskId::new("a", 8);
        let first_event = host
            .run_agent(&task_a, "daemon-a", "first prompt")
            .expect("daemon host should consume first stream");
        let first_complete = host
            .run_agent(&task_a, "daemon-a", "first prompt")
            .expect("daemon host should consume first completion");
        let second_event = host
            .run_agent(&task_b, "daemon-a", "second prompt")
            .expect("daemon host should consume second stream");
        let second_complete = host
            .run_agent(&task_b, "daemon-a", "second prompt")
            .expect("daemon host should consume second completion");

        assert_eq!(first_event.stream_events.len(), 1);
        assert_eq!(second_event.stream_events.len(), 1);
        assert_eq!(first_complete.token_delta, 1);
        assert_eq!(second_complete.token_delta, 2);
    }

    #[test]
    fn process_task_agent_host_daemon_mode_restarts_after_child_exit() {
        if OsCommand::new("python3").arg("--version").output().is_err() {
            return;
        }

        let script = r#"import json, sys
request = json.loads(sys.stdin.readline())
response = {
    "tool_use_delta": len(request.get("agent_id", "")),
    "token_delta": 1,
    "retrieved": True,
    "status": "completed",
}
json.dump(response, sys.stdout)
sys.stdout.write("\n")
sys.stdout.flush()
"#;
        let mut host = ProcessTaskAgentHost::with_args(
            "python3",
            vec![String::from("-u"), String::from("-c"), script.to_string()],
        )
        .with_daemon_mode();

        let first_task = TaskId::new("a", 9);
        let second_task = TaskId::new("a", 10);
        let first = host
            .run_agent(&first_task, "daemon-a", "first prompt")
            .expect("daemon host should produce first step");
        let second = host
            .run_agent(&second_task, "daemon-a", "second prompt")
            .expect("daemon host should restart and produce second step");

        assert_eq!(first.status, TaskStatus::Completed);
        assert_eq!(second.status, TaskStatus::Completed);
        assert_eq!(host.request_count(), 2);
        assert_eq!(host.spawn_count(), 2);
        assert_eq!(host.restart_count(), 1);
        assert!(host.daemon_running());
        assert_eq!(host.consecutive_failure_count(), 0);
        assert_eq!(host.last_backoff_ms(), 0);
        assert_eq!(host.last_failure_kind(), None);
        assert_eq!(host.last_error(), None);
    }

    #[test]
    fn process_task_agent_host_daemon_respects_restart_budget() {
        if OsCommand::new("python3").arg("--version").output().is_err() {
            return;
        }

        let script = r#"import json, sys
request = json.loads(sys.stdin.readline())
response = {
    "tool_use_delta": len(request.get("agent_id", "")),
    "token_delta": 1,
    "retrieved": True,
    "status": "completed",
}
json.dump(response, sys.stdout)
sys.stdout.write("\n")
sys.stdout.flush()
"#;
        let mut host = ProcessTaskAgentHost::with_args(
            "python3",
            vec![String::from("-u"), String::from("-c"), script.to_string()],
        )
        .with_daemon_mode()
        .with_daemon_restart_budget(0);

        let first_task = TaskId::new("a", 11);
        let second_task = TaskId::new("a", 12);
        let first = host
            .run_agent(&first_task, "daemon-a", "first prompt")
            .expect("daemon host should produce first step");
        let second = host
            .run_agent(&second_task, "daemon-a", "second prompt")
            .expect_err("daemon host should fail without restart budget");

        assert_eq!(first.status, TaskStatus::Completed);
        assert!(
            second.contains("process agent daemon exited before response: 0"),
            "unexpected error: {second}"
        );
        assert_eq!(host.request_count(), 2);
        assert_eq!(host.spawn_count(), 1);
        assert_eq!(host.restart_count(), 0);
        assert_eq!(host.consecutive_failure_count(), 1);
        assert_eq!(host.last_backoff_ms(), 0);
        assert_eq!(
            host.last_failure_kind(),
            Some(ProcessAgentFailureKind::ProcessExit)
        );
        assert_eq!(
            host.last_error(),
            Some("process agent daemon exited before response: 0")
        );
        assert_eq!(host.last_exit_code(), Some(0));
    }

    #[test]
    fn process_task_agent_host_daemon_exposes_supervisor_policy() {
        let host = ProcessTaskAgentHost::new("python3")
            .with_daemon_mode()
            .with_daemon_restart_budget(3)
            .with_daemon_max_consecutive_failures(2)
            .with_daemon_restart_backoff_strategy(ProcessAgentBackoffStrategy::Exponential)
            .with_daemon_restart_backoff_ms(25)
            .with_daemon_restart_on_io_error(false)
            .with_daemon_restart_on_decode_error(false)
            .with_daemon_restart_on_clean_exit(false);

        assert_eq!(host.mode_label(), "process-daemon");
        assert_eq!(host.max_restart_attempts(), 3);
        assert_eq!(host.max_consecutive_failures(), 2);
        assert_eq!(
            host.restart_backoff_strategy(),
            ProcessAgentBackoffStrategy::Exponential
        );
        assert_eq!(host.restart_backoff_ms(), 25);
        assert_eq!(host.supervisor().policy().restart.max_restart_attempts, 3);
        assert_eq!(
            host.supervisor().policy().restart.max_consecutive_failures,
            2
        );
        assert_eq!(
            host.supervisor().policy().backoff.default_profile.strategy,
            ProcessAgentBackoffStrategy::Exponential
        );
        assert_eq!(
            host.supervisor()
                .policy()
                .backoff
                .default_profile
                .base_delay_ms,
            25
        );
        assert_eq!(
            host.supervisor()
                .policy()
                .backoff
                .default_profile
                .jitter_percent,
            0
        );
        assert!(!host.supervisor().policy().restart.restart_on_io_error);
        assert!(!host.supervisor().policy().restart.restart_on_decode_error);
        assert!(!host.supervisor().policy().restart.restart_on_clean_exit);
    }

    #[test]
    fn process_agent_backoff_strategy_delay_profiles_are_stable() {
        assert_eq!(ProcessAgentBackoffStrategy::Linear.delay_ms(25, 0), 0);
        assert_eq!(ProcessAgentBackoffStrategy::Linear.delay_ms(25, 2), 50);
        assert_eq!(ProcessAgentBackoffStrategy::Exponential.delay_ms(25, 1), 25);
        assert_eq!(ProcessAgentBackoffStrategy::Exponential.delay_ms(25, 2), 50);
        assert_eq!(
            ProcessAgentBackoffStrategy::Exponential.delay_ms(25, 3),
            100
        );
    }

    #[test]
    fn process_agent_backoff_policy_applies_deterministic_jitter() {
        let policy = ProcessAgentBackoffPolicy {
            default_profile: ProcessAgentBackoffProfile {
                base_delay_ms: 100,
                strategy: ProcessAgentBackoffStrategy::Linear,
                jitter_percent: 10,
            },
            ..ProcessAgentBackoffPolicy::default()
        };
        let failure = ProcessAgentFailure::new(ProcessAgentFailureKind::Spawn, "spawn");
        assert_eq!(
            policy.delay_ms(&failure, 0, 99),
            (ProcessAgentBackoffProfileKind::Default, 0)
        );
        assert_eq!(
            policy.delay_ms(&failure, 1, 3),
            (ProcessAgentBackoffProfileKind::Default, 103)
        );
        assert_eq!(
            policy.delay_ms(&failure, 2, 8),
            (ProcessAgentBackoffProfileKind::Default, 208)
        );
    }

    #[test]
    fn process_agent_backoff_policy_selects_failure_specific_profiles() {
        let policy = ProcessAgentBackoffPolicy {
            default_profile: ProcessAgentBackoffProfile {
                base_delay_ms: 10,
                strategy: ProcessAgentBackoffStrategy::Linear,
                jitter_percent: 0,
            },
            io_profile: Some(ProcessAgentBackoffProfile {
                base_delay_ms: 25,
                strategy: ProcessAgentBackoffStrategy::Linear,
                jitter_percent: 0,
            }),
            decode_profile: Some(ProcessAgentBackoffProfile {
                base_delay_ms: 50,
                strategy: ProcessAgentBackoffStrategy::Exponential,
                jitter_percent: 0,
            }),
            exit_profile: Some(ProcessAgentBackoffProfile {
                base_delay_ms: 5,
                strategy: ProcessAgentBackoffStrategy::Linear,
                jitter_percent: 0,
            }),
        };
        let io_failure =
            ProcessAgentFailure::new(ProcessAgentFailureKind::ResponseRead, "io failure");
        let decode_failure =
            ProcessAgentFailure::new(ProcessAgentFailureKind::ResponseDecode, "decode failure");
        let exit_failure = ProcessAgentFailure::new(ProcessAgentFailureKind::ProcessExit, "exit")
            .with_exit_code(Some(7));
        let spawn_failure = ProcessAgentFailure::new(ProcessAgentFailureKind::Spawn, "spawn");

        assert_eq!(
            policy.delay_ms(&io_failure, 2, 0),
            (ProcessAgentBackoffProfileKind::Io, 50)
        );
        assert_eq!(
            policy.delay_ms(&decode_failure, 2, 0),
            (ProcessAgentBackoffProfileKind::Decode, 100)
        );
        assert_eq!(
            policy.delay_ms(&exit_failure, 2, 0),
            (ProcessAgentBackoffProfileKind::Exit, 10)
        );
        assert_eq!(
            policy.delay_ms(&spawn_failure, 2, 0),
            (ProcessAgentBackoffProfileKind::Default, 20)
        );
    }

    #[test]
    fn process_task_agent_host_reports_process_failures() {
        if OsCommand::new("python3").arg("--version").output().is_err() {
            return;
        }

        let script = r#"import sys
sys.stderr.write("daemon denied")
sys.exit(7)
"#;
        let mut host = ProcessTaskAgentHost::with_args(
            "python3",
            vec![String::from("-c"), script.to_string()],
        );
        let error = host
            .run_agent(&TaskId::new("a", 13), "daemon-a", "remote prompt")
            .expect_err("process host should fail when subprocess exits non-zero");

        assert!(error.contains("code 7"));
        assert!(error.contains("daemon denied"));
        assert_eq!(host.consecutive_failure_count(), 1);
        assert_eq!(
            host.last_failure_kind(),
            Some(ProcessAgentFailureKind::ProcessExit)
        );
        assert_eq!(host.last_exit_code(), Some(7));
    }

    #[test]
    fn process_task_agent_host_daemon_can_disable_clean_exit_restart() {
        if OsCommand::new("python3").arg("--version").output().is_err() {
            return;
        }

        let script = r#"import json, sys
request = json.loads(sys.stdin.readline())
response = {
    "tool_use_delta": len(request.get("agent_id", "")),
    "token_delta": 1,
    "retrieved": True,
    "status": "completed",
}
json.dump(response, sys.stdout)
sys.stdout.write("\n")
sys.stdout.flush()
"#;
        let mut host = ProcessTaskAgentHost::with_args(
            "python3",
            vec![String::from("-u"), String::from("-c"), script.to_string()],
        )
        .with_daemon_mode()
        .with_daemon_restart_budget(1)
        .with_daemon_restart_on_clean_exit(false);

        let first_task = TaskId::new("a", 14);
        let second_task = TaskId::new("a", 15);
        let first = host
            .run_agent(&first_task, "daemon-a", "first prompt")
            .expect("daemon host should produce first step");
        let second = host
            .run_agent(&second_task, "daemon-a", "second prompt")
            .expect_err("daemon host should not restart after clean exit");

        assert_eq!(first.status, TaskStatus::Completed);
        assert!(second.contains("process agent daemon exited before response: 0"));
        assert_eq!(host.spawn_count(), 1);
        assert_eq!(host.restart_count(), 0);
        assert_eq!(host.consecutive_failure_count(), 1);
        assert_eq!(
            host.last_failure_kind(),
            Some(ProcessAgentFailureKind::ProcessExit)
        );
        assert_eq!(host.last_exit_code(), Some(0));
    }

    #[test]
    fn process_task_agent_host_daemon_restarts_after_decode_error() {
        if OsCommand::new("python3").arg("--version").output().is_err() {
            return;
        }

        let marker = std::env::temp_dir().join(format!(
            "nocode-daemon-decode-{}-{}.marker",
            std::process::id(),
            current_time_millis()
        ));
        let marker_path = marker.to_string_lossy().replace('\\', "\\\\");
        let script = format!(
            r#"import json, os, sys
marker = r"{marker_path}"
request = json.loads(sys.stdin.readline())
if not os.path.exists(marker):
    open(marker, "w").close()
    sys.stdout.write("not-json\n")
    sys.stdout.flush()
    sys.exit(0)
response = {{
    "tool_use_delta": len(request.get("agent_id", "")),
    "token_delta": 2,
    "retrieved": True,
    "status": "completed",
}}
json.dump(response, sys.stdout)
sys.stdout.write("\n")
sys.stdout.flush()
"#
        );
        let _ = std::fs::remove_file(&marker);
        let mut host = ProcessTaskAgentHost::with_args(
            "python3",
            vec![String::from("-u"), String::from("-c"), script],
        )
        .with_daemon_mode()
        .with_daemon_restart_budget(1);

        let step = host
            .run_agent(&TaskId::new("a", 16), "daemon-a", "decode prompt")
            .expect("daemon host should restart after decode failure");

        assert_eq!(step.status, TaskStatus::Completed);
        assert_eq!(step.token_delta, 2);
        assert_eq!(host.request_count(), 1);
        assert_eq!(host.spawn_count(), 2);
        assert_eq!(host.restart_count(), 1);
        assert_eq!(host.last_backoff_ms(), 0);
        assert_eq!(host.last_failure_kind(), None);
        assert_eq!(host.last_exit_code(), None);
        let _ = std::fs::remove_file(&marker);
    }

    #[test]
    fn process_task_agent_host_daemon_limits_consecutive_failures() {
        if OsCommand::new("python3").arg("--version").output().is_err() {
            return;
        }

        let script = r#"import sys
sys.stdout.write("not-json\n")
sys.stdout.flush()
sys.exit(0)
"#;
        let mut host = ProcessTaskAgentHost::with_args(
            "python3",
            vec![String::from("-u"), String::from("-c"), script.to_string()],
        )
        .with_daemon_mode()
        .with_daemon_restart_budget(3)
        .with_daemon_max_consecutive_failures(2)
        .with_daemon_restart_backoff_ms(5);

        let error = host
            .run_agent(&TaskId::new("a", 17), "daemon-a", "decode prompt")
            .expect_err("daemon host should stop after hitting consecutive failure cap");

        assert!(error.contains("failed to decode process agent response"));
        assert_eq!(host.request_count(), 1);
        assert_eq!(host.spawn_count(), 2);
        assert_eq!(host.restart_count(), 1);
        assert_eq!(host.consecutive_failure_count(), 2);
        assert_eq!(host.last_backoff_ms(), 5);
        assert_eq!(host.last_backoff_profile_kind(), None);
        assert_eq!(
            host.last_failure_kind(),
            Some(ProcessAgentFailureKind::ResponseDecode)
        );
    }

    #[test]
    fn process_task_agent_host_daemon_records_exponential_backoff_delay() {
        if OsCommand::new("python3").arg("--version").output().is_err() {
            return;
        }

        let marker = std::env::temp_dir().join(format!(
            "nocode-daemon-exp-{}-{}.marker",
            std::process::id(),
            current_time_millis()
        ));
        let marker_path = marker.to_string_lossy().replace('\\', "\\\\");
        let script = format!(
            r#"import json, os, sys
marker = r"{marker_path}"
request = json.loads(sys.stdin.readline())
count = 0
if os.path.exists(marker):
    count = int(open(marker, "r").read().strip() or "0")
count += 1
open(marker, "w").write(str(count))
if count < 3:
    sys.stdout.write("not-json\n")
    sys.stdout.flush()
    sys.exit(0)
response = {{
    "tool_use_delta": len(request.get("agent_id", "")),
    "token_delta": count,
    "retrieved": True,
    "status": "completed",
}}
json.dump(response, sys.stdout)
sys.stdout.write("\n")
sys.stdout.flush()
"#
        );
        let _ = std::fs::remove_file(&marker);
        let mut host = ProcessTaskAgentHost::with_args(
            "python3",
            vec![String::from("-u"), String::from("-c"), script],
        )
        .with_daemon_mode()
        .with_daemon_restart_budget(3)
        .with_daemon_max_consecutive_failures(4)
        .with_daemon_restart_backoff_ms(2)
        .with_daemon_restart_backoff_strategy(ProcessAgentBackoffStrategy::Exponential)
        .with_daemon_restart_backoff_jitter_percent(10)
        .with_daemon_decode_backoff_profile(ProcessAgentBackoffProfile {
            base_delay_ms: 7,
            strategy: ProcessAgentBackoffStrategy::Linear,
            jitter_percent: 0,
        });

        let step = host
            .run_agent(&TaskId::new("a", 18), "daemon-a", "decode prompt")
            .expect("daemon host should recover after exponential backoff retries");

        assert_eq!(step.status, TaskStatus::Completed);
        assert_eq!(host.spawn_count(), 3);
        assert_eq!(host.restart_count(), 2);
        assert_eq!(host.last_backoff_ms(), 14);
        assert_eq!(host.last_backoff_profile_kind(), None);
        assert_eq!(host.consecutive_failure_count(), 0);
        let _ = std::fs::remove_file(&marker);
    }

    #[test]
    fn live_task_shell_host_executes_real_shell_command() {
        let mut host = LiveTaskShellHost;

        let result = host
            .run_command("exit 0")
            .expect("shell host should execute command");

        assert_eq!(result.code, 0);
        assert!(!result.interrupted);
    }

    #[test]
    fn drive_next_marks_task_failed_when_driver_errors() {
        let mut coordinator = TaskCoordinator::new();
        let shell = coordinator.spawn_local_shell("echo fail".into(), None, None);
        let mut driver = FailingDriver;

        let error = coordinator
            .drive_next(&mut driver)
            .expect_err("drive should fail");

        assert_eq!(
            error,
            TaskDriveError::DriverFailure {
                task_id: shell.clone(),
                message: String::from("shell failed"),
            }
        );
        assert_eq!(
            coordinator.record(&shell).unwrap().base.status,
            TaskStatus::Failed
        );
    }
}
