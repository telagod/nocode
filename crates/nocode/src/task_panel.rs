use nocode_core::{
    BashTaskKind, DreamPhase, ModelErrorWire, ModelStreamEventWire, StopTaskError, StopTaskResult,
    TaskCoordinator, TaskDriveError, TaskDriveReport, TaskId, TaskPayload, TaskRecord, TaskStatus,
    TaskType,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TaskFilterSpec {
    raw: Option<String>,
    terms: Vec<TaskFilterTerm>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TaskFilterTerm {
    Status(TaskStatus),
    TaskType(TaskType),
    ResultKind(String),
    ResultStructured(bool),
    Text(String),
}

impl TaskFilterSpec {
    fn parse(raw: Option<&str>) -> Self {
        let normalized = raw
            .map(str::trim)
            .filter(|value| !value.is_empty() && !matches!(*value, "all" | "*"))
            .map(ToString::to_string);
        let terms = normalized
            .as_deref()
            .map(|value| {
                value
                    .split_whitespace()
                    .filter_map(TaskFilterTerm::parse)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Self {
            raw: normalized,
            terms,
        }
    }

    fn label(&self) -> &str {
        self.raw.as_deref().unwrap_or("all")
    }

    fn matches(&self, record: &TaskRecord) -> bool {
        self.terms.iter().all(|term| term.matches(record))
    }
}

impl TaskFilterTerm {
    fn parse(raw: &str) -> Option<Self> {
        let token = raw.trim();
        if token.is_empty() {
            return None;
        }
        if let Some((field, value)) = token.split_once(':') {
            return Self::parse_scoped(field.trim(), value.trim());
        }
        Some(
            parse_status_token(token)
                .map(Self::Status)
                .or_else(|| parse_type_token(token).map(Self::TaskType))
                .unwrap_or_else(|| Self::Text(token.to_ascii_lowercase())),
        )
    }

    fn parse_scoped(field: &str, value: &str) -> Option<Self> {
        match field.to_ascii_lowercase().as_str() {
            "status" => parse_status_token(value).map(Self::Status),
            "type" => parse_type_token(value).map(Self::TaskType),
            "result" => parse_result_kind_token(value).map(Self::ResultKind),
            "result-structured" | "result_structured" => {
                parse_bool_token(value).map(Self::ResultStructured)
            }
            "text" | "match" | "contains" => Some(Self::Text(value.to_ascii_lowercase())),
            _ => Some(Self::Text(format!(
                "{}:{}",
                field.to_ascii_lowercase(),
                value.to_ascii_lowercase()
            ))),
        }
    }

    fn matches(&self, record: &TaskRecord) -> bool {
        match self {
            Self::Status(status) => &record.base.status == status,
            Self::TaskType(task_type) => &record.base.task_type == task_type,
            Self::ResultKind(kind) => record
                .result()
                .is_some_and(|result| result.kind_label() == kind.as_str()),
            Self::ResultStructured(expected) => record
                .result()
                .is_some_and(|result| result.has_response_result_payload() == *expected),
            Self::Text(needle) => {
                let payload = render_task_payload(&record.payload).to_ascii_lowercase();
                let result = record
                    .result()
                    .map(|result| result.to_value().to_string().to_ascii_lowercase())
                    .unwrap_or_default();
                record
                    .base
                    .id
                    .as_str()
                    .to_ascii_lowercase()
                    .contains(needle)
                    || record
                        .base
                        .description
                        .to_ascii_lowercase()
                        .contains(needle)
                    || render_task_type(record.base.task_type).contains(needle)
                    || render_task_status(record.base.status).contains(needle)
                    || payload.contains(needle)
                    || result.contains(needle)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskPanelView {
    list: TaskListView,
    detail: Option<TaskDetailView>,
    summary: Option<TaskSummaryView>,
}

impl TaskPanelView {
    pub(crate) fn from_state(
        coordinator: &TaskCoordinator,
        filter: Option<&str>,
        selected_id: Option<&TaskId>,
        detail_id: Option<&TaskId>,
    ) -> Self {
        let filter_spec = TaskFilterSpec::parse(filter);
        let queue = coordinator.queue_snapshot();
        let mut tasks = coordinator.list_tasks();
        if !filter_spec.terms.is_empty() {
            tasks.retain(|record| filter_spec.matches(record));
        }
        tasks.sort_by(|left, right| left.base.id.as_str().cmp(right.base.id.as_str()));
        let detail = detail_id
            .and_then(|task_id| tasks.iter().find(|record| &record.base.id == task_id))
            .map(TaskDetailView::from_record);
        if tasks.is_empty() {
            let empty_message = if filter_spec.raw.is_some() {
                format!("tasks: none for filter={}", filter_spec.label())
            } else {
                String::from("tasks: none")
            };
            return Self {
                list: TaskListView {
                    entries: Vec::new(),
                    empty_message,
                },
                detail,
                summary: None,
            };
        }
        let entries = tasks
            .iter()
            .map(|record| {
                TaskListEntryView::from_record(
                    record,
                    queue.as_slice(),
                    selected_id.is_some_and(|task_id| task_id == &record.base.id),
                )
            })
            .collect::<Vec<_>>();
        let summary = Some(TaskSummaryView::from_records(
            tasks.as_slice(),
            queue.len(),
            filter_spec.label(),
        ));
        Self {
            list: TaskListView {
                entries,
                empty_message: String::from("tasks: none"),
            },
            detail,
            summary,
        }
    }

    pub(crate) fn render_list_summary(&self) -> String {
        if self.list.entries.is_empty() {
            return self.list.empty_message.clone();
        }
        let mut lines = self
            .list
            .entries
            .iter()
            .map(TaskListEntryView::render)
            .collect::<Vec<_>>();
        if let Some(summary) = &self.summary {
            lines.push(summary.render());
        }
        lines.join("\n")
    }

    #[allow(dead_code)]
    pub(crate) fn render_detail(&self) -> String {
        self.detail
            .as_ref()
            .map(TaskDetailView::render)
            .unwrap_or_else(|| String::from("task show: task not found"))
    }

    pub(crate) fn has_detail(&self) -> bool {
        self.detail.is_some()
    }

    #[allow(dead_code)]
    pub(crate) fn render_layout(&self) -> String {
        self.render_layout_with_focus("tasks")
    }

    pub(crate) fn render_layout_with_focus(&self, focus_label: &str) -> String {
        let mut sections = vec![format!("task pane: focus={focus_label}")];
        sections.extend(self.render_layout_sections());
        sections.join("\n")
    }

    fn render_layout_sections(&self) -> Vec<String> {
        let mut sections = vec![format!("task list:\n{}", self.render_list_summary())];
        if let Some(detail) = &self.detail {
            sections.push(format!("task detail:\n{}", detail.render()));
        }
        sections
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TaskListView {
    entries: Vec<TaskListEntryView>,
    empty_message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TaskListEntryView {
    task_id: String,
    task_type: &'static str,
    status: &'static str,
    queue_position: Option<usize>,
    selected: bool,
    summary: String,
    payload: String,
}

impl TaskListEntryView {
    fn from_record(record: &TaskRecord, queue: &[TaskId], selected: bool) -> Self {
        Self {
            task_id: record.base.id.as_str().to_string(),
            task_type: render_task_type(record.base.task_type),
            status: render_task_status(record.base.status),
            queue_position: queue.iter().position(|task_id| task_id == &record.base.id),
            selected,
            summary: record.base.description.clone(),
            payload: render_task_payload(&record.payload),
        }
    }

    fn render(&self) -> String {
        format!(
            "{}{} type={} status={} queue={} summary={} {}",
            if self.selected { "> " } else { "  " },
            self.task_id,
            self.task_type,
            self.status,
            self.queue_position
                .map_or_else(|| String::from("-"), |index| (index + 1).to_string()),
            self.summary,
            self.payload
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TaskDetailView {
    lines: Vec<String>,
}

impl TaskDetailView {
    fn from_record(record: &TaskRecord) -> Self {
        Self {
            lines: vec![
                format!("task {}", record.base.id.as_str()),
                format!("type={}", render_task_type(record.base.task_type)),
                format!("status={}", render_task_status(record.base.status)),
                format!("summary={}", record.base.description),
                format!("start_ms={}", record.base.start_time),
                format!(
                    "end_ms={}",
                    record
                        .base
                        .end_time
                        .map_or_else(|| String::from("pending"), |value| value.to_string())
                ),
                format!("notified={}", record.base.notified),
                render_task_detail_payload(&record.payload),
            ],
        }
    }

    fn render(&self) -> String {
        self.lines.join("\n")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TaskSummaryView {
    filter_label: String,
    total: usize,
    queued: usize,
    pending: usize,
    running: usize,
    completed: usize,
    failed: usize,
    killed: usize,
}

impl TaskSummaryView {
    fn from_records(tasks: &[TaskRecord], queued: usize, filter_label: &str) -> Self {
        let mut pending = 0usize;
        let mut running = 0usize;
        let mut completed = 0usize;
        let mut failed = 0usize;
        let mut killed = 0usize;
        for record in tasks {
            match record.base.status {
                TaskStatus::Pending => pending += 1,
                TaskStatus::Running => running += 1,
                TaskStatus::Completed => completed += 1,
                TaskStatus::Failed => failed += 1,
                TaskStatus::Killed => killed += 1,
            }
        }
        Self {
            filter_label: filter_label.to_string(),
            total: tasks.len(),
            queued,
            pending,
            running,
            completed,
            failed,
            killed,
        }
    }

    fn render(&self) -> String {
        format!(
            "task summary: filter={} total={} queued={} pending={} running={} completed={} failed={} killed={}",
            self.filter_label,
            self.total,
            self.queued,
            self.pending,
            self.running,
            self.completed,
            self.failed,
            self.killed
        )
    }
}

pub(crate) fn normalize_task_filter(filter: Option<String>) -> Option<String> {
    filter.and_then(|value| {
        let normalized = value.trim();
        if normalized.is_empty() || matches!(normalized, "all" | "*") {
            None
        } else {
            Some(normalized.to_string())
        }
    })
}

pub(crate) fn align_selected_task_id(
    coordinator: &TaskCoordinator,
    filter: Option<&str>,
    current: Option<&TaskId>,
) -> Option<TaskId> {
    let filter_spec = TaskFilterSpec::parse(filter);
    if let Some(task_id) = current {
        let tasks = filtered_tasks_by_start(coordinator, &filter_spec);
        if tasks.iter().any(|record| &record.base.id == task_id) {
            return Some(task_id.clone());
        }
    }
    first_task_id(coordinator, &filter_spec)
}

pub(crate) fn resolve_task_id(
    coordinator: &TaskCoordinator,
    raw_id: &str,
    current: Option<&TaskId>,
    filter: Option<&str>,
) -> Option<TaskId> {
    let normalized = raw_id.trim();
    let filter_spec = TaskFilterSpec::parse(filter);
    if matches!(normalized, "latest" | "last") {
        return last_task_id(coordinator, &filter_spec);
    }
    if normalized == "first" {
        return first_task_id(coordinator, &filter_spec);
    }
    if matches!(normalized, "prev" | "previous") {
        return browse_task_id(coordinator, current, &filter_spec, -1);
    }
    if normalized == "next" {
        return browse_task_id(coordinator, current, &filter_spec, 1);
    }

    let exact = coordinator
        .list_tasks()
        .into_iter()
        .find(|record| record.base.id.as_str() == normalized)
        .map(|record| record.base.id);
    if exact.is_some() {
        return exact;
    }

    let matches = coordinator
        .list_tasks()
        .into_iter()
        .filter(|record| record.base.id.as_str().starts_with(normalized))
        .map(|record| record.base.id)
        .collect::<Vec<_>>();
    if matches.len() == 1 {
        matches.into_iter().next()
    } else {
        None
    }
}

pub(crate) fn step_selected_task_id(
    coordinator: &TaskCoordinator,
    filter: Option<&str>,
    current: Option<&TaskId>,
    direction: i8,
) -> Option<TaskId> {
    let filter_spec = TaskFilterSpec::parse(filter);
    browse_task_id(coordinator, current, &filter_spec, direction)
}

fn first_task_id(coordinator: &TaskCoordinator, filter: &TaskFilterSpec) -> Option<TaskId> {
    filtered_tasks_by_start(coordinator, filter)
        .into_iter()
        .next()
        .map(|record| record.base.id)
}

fn last_task_id(coordinator: &TaskCoordinator, filter: &TaskFilterSpec) -> Option<TaskId> {
    filtered_tasks_by_start(coordinator, filter)
        .into_iter()
        .last()
        .map(|record| record.base.id)
}

fn filtered_tasks_by_start(
    coordinator: &TaskCoordinator,
    filter: &TaskFilterSpec,
) -> Vec<TaskRecord> {
    let mut tasks = coordinator.list_tasks();
    if !filter.terms.is_empty() {
        tasks.retain(|record| filter.matches(record));
    }
    tasks.sort_by(|left, right| left.base.start_time.cmp(&right.base.start_time));
    tasks
}

fn browse_task_id(
    coordinator: &TaskCoordinator,
    current: Option<&TaskId>,
    filter: &TaskFilterSpec,
    direction: i8,
) -> Option<TaskId> {
    let tasks = filtered_tasks_by_start(coordinator, filter);
    if tasks.is_empty() {
        return None;
    }
    let current_index =
        current.and_then(|task_id| tasks.iter().position(|record| &record.base.id == task_id));
    let next_index = match (direction, current_index) {
        (d, Some(index)) if d < 0 => index.saturating_sub(1),
        (_, Some(index)) if index + 1 < tasks.len() => index + 1,
        (_, Some(index)) => index,
        (d, None) if d < 0 => tasks.len().saturating_sub(1),
        _ => 0,
    };
    tasks.get(next_index).map(|record| record.base.id.clone())
}

pub(crate) fn render_task_queue(coordinator: &TaskCoordinator) -> String {
    let queue = coordinator.queue_snapshot();
    if queue.is_empty() {
        return String::from("task queue: empty");
    }
    queue
        .iter()
        .enumerate()
        .map(|(index, task_id)| format!("queue[{index}] {}", task_id.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_task_payload(payload: &TaskPayload) -> String {
    match payload {
        TaskPayload::LocalShell(shell) => {
            let result = payload.result_preview(96).map_or_else(
                || String::from("result=pending"),
                |result| format!("result={result}"),
            );
            format!("cmd={} {}", shell.command, result)
        }
        TaskPayload::LocalAgent(agent) => format!(
            "agent={} tools={} tokens={} retrieved={} result={}",
            agent.agent_id,
            agent.progress.tool_use_count,
            agent.progress.token_count,
            agent.retrieved,
            payload
                .result_preview(96)
                .unwrap_or_else(|| String::from("none"))
        ),
        TaskPayload::Dream(dream) => format!(
            "phase={} sessions={} files={} turns={} result={}",
            render_dream_phase(dream.phase),
            dream.sessions_reviewing,
            dream.files_touched.len(),
            dream.turns.len(),
            payload
                .result_preview(96)
                .unwrap_or_else(|| String::from("none"))
        ),
    }
}

pub(crate) fn render_task_spawned(task_id: TaskId, task_type: TaskType, detail: &str) -> String {
    format!(
        "task spawned: {} type={} {}",
        task_id.as_str(),
        render_task_type(task_type),
        detail
    )
}

pub(crate) fn render_task_drive_report(report: &TaskDriveReport) -> String {
    let activity = report
        .activity
        .as_deref()
        .map_or_else(String::new, |activity| format!(" activity={activity}"));
    format!(
        "task drive: {} type={} status={} summary={}{} result={}",
        report.task_id.as_str(),
        render_task_type(report.task_type),
        render_task_status(report.status),
        report.summary,
        activity,
        report.result_preview.as_deref().unwrap_or("none")
    )
}

pub(crate) fn render_task_drive_reports(reports: &[TaskDriveReport]) -> String {
    if reports.is_empty() {
        return String::from("task drive: idle");
    }
    reports
        .iter()
        .map(render_task_drive_report)
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn render_task_auto_drive(
    reports: &[TaskDriveReport],
    coordinator: &TaskCoordinator,
    filter: Option<&str>,
    selected_id: Option<&TaskId>,
    detail_id: Option<&TaskId>,
    focus_label: &str,
) -> String {
    let panel = TaskPanelView::from_state(coordinator, filter, selected_id, detail_id);
    format!(
        "task auto-drive:\n{}\ntask refresh:\n{}",
        render_task_drive_reports(reports),
        panel.render_layout_with_focus(focus_label)
    )
}

pub(crate) fn render_task_drive_error(error: &TaskDriveError) -> String {
    match error {
        TaskDriveError::DriverFailure { task_id, message } => {
            format!("task drive error: {} {}", task_id.as_str(), message)
        }
        TaskDriveError::MissingTask { task_id } => {
            format!("task drive error: missing {}", task_id.as_str())
        }
    }
}

pub(crate) fn render_task_stop_result(result: &StopTaskResult) -> String {
    let command = result
        .command
        .as_deref()
        .map_or_else(String::new, |command| format!(" command={command}"));
    format!(
        "task stopped: {} type={} summary={}{}",
        result.task_id.as_str(),
        render_task_type(result.task_type),
        result.summary,
        command
    )
}

pub(crate) fn render_task_stop_error(task_id: &str, error: &StopTaskError) -> String {
    let reason = match error {
        StopTaskError::NotFound => "not_found",
        StopTaskError::NotRunning => "not_running",
        StopTaskError::UnsupportedType => "unsupported_type",
    };
    format!("task stop error: {task_id} {reason}")
}

fn render_task_detail_payload(payload: &TaskPayload) -> String {
    match payload {
        TaskPayload::LocalShell(shell) => {
            let mut lines = vec![
                String::from("payload=local_shell"),
                format!("kind={}", render_bash_task_kind(shell.kind)),
                format!("command={}", shell.command),
            ];
            match &shell.result {
                Some(result) => {
                    lines.push(format!("result.code={}", result.code));
                    lines.push(format!("result.interrupted={}", result.interrupted));
                }
                None => lines.push(String::from("result=pending")),
            }
            if let Some(result) = payload.result_preview(120) {
                lines.push(format!("result={result}"));
            }
            if let Some(pretty) = payload.result_pretty() {
                lines.push(format!("result.pretty:\n{pretty}"));
            }
            lines.join("\n")
        }
        TaskPayload::LocalAgent(agent) => {
            let mut lines = vec![
                String::from("payload=local_agent"),
                format!("agent_id={}", agent.agent_id),
                format!("prompt={}", agent.prompt),
                format!("progress.tool_use_count={}", agent.progress.tool_use_count),
                format!("progress.token_count={}", agent.progress.token_count),
                format!("retrieved={}", agent.retrieved),
                format!("stream_events.count={}", agent.stream_events.len()),
            ];
            if let Some(last_event) = agent.stream_events.last() {
                lines.push(format!(
                    "stream_events.last={}",
                    render_stream_event_summary(last_event)
                ));
            }
            for (index, event) in recent_stream_events(agent.stream_events.as_slice())
                .into_iter()
                .enumerate()
            {
                lines.push(format!("stream_events.recent[{index}]={event}"));
            }
            if let Some(model_error) = &agent.model_error {
                lines.extend(render_model_error_lines(model_error));
            }
            if let Some(pretty) = agent.response_result_pretty() {
                lines.push(format!(
                    "response-result.preview={}",
                    agent.response_result_preview(120)
                ));
                lines.push(format!("response-result.pretty:\n{pretty}"));
            }
            if let Some(result) = payload.result_preview(120) {
                lines.push(format!("result={result}"));
            }
            if let Some(pretty) = payload.result_pretty() {
                lines.push(format!("result.pretty:\n{pretty}"));
            }
            lines.join("\n")
        }
        TaskPayload::Dream(dream) => {
            let mut lines = vec![
                String::from("payload=dream"),
                format!("phase={}", render_dream_phase(dream.phase)),
                format!("sessions_reviewing={}", dream.sessions_reviewing),
                format!("files_touched={}", dream.files_touched.len()),
                format!("turns={}", dream.turns.len()),
            ];
            for (index, file) in dream.files_touched.iter().enumerate() {
                lines.push(format!("file[{index}]={file}"));
            }
            for (index, turn) in dream.turns.iter().enumerate() {
                lines.push(format!(
                    "turn[{index}]={} tools={}",
                    turn.text, turn.tool_use_count
                ));
            }
            if let Some(result) = payload.result_preview(120) {
                lines.push(format!("result={result}"));
            }
            if let Some(pretty) = payload.result_pretty() {
                lines.push(format!("result.pretty:\n{pretty}"));
            }
            lines.join("\n")
        }
    }
}

fn recent_stream_events(events: &[ModelStreamEventWire]) -> Vec<String> {
    let keep = events.len().min(3);
    events[events.len().saturating_sub(keep)..]
        .iter()
        .map(render_stream_event_summary)
        .collect()
}

fn render_stream_event_summary(event: &ModelStreamEventWire) -> String {
    match event {
        ModelStreamEventWire::Start { provider, model } => {
            format!("start provider={provider} model={model}")
        }
        ModelStreamEventWire::Delta { text } => format!("delta text={}", truncate_value(text, 96)),
        ModelStreamEventWire::Complete { role, content } => {
            format!(
                "complete role={role} content={}",
                truncate_value(content, 96)
            )
        }
    }
}

fn render_model_error_lines(error: &ModelErrorWire) -> Vec<String> {
    vec![
        format!("model_error.surface={}", error.surface),
        format!("model_error.kind={}", error.kind),
        format!(
            "model_error.provider={}",
            error.provider.as_deref().unwrap_or("none")
        ),
        format!(
            "model_error.status_code={}",
            error
                .status_code
                .map_or_else(|| String::from("none"), |code| code.to_string())
        ),
        format!(
            "model_error.status_class={}",
            error.status_class.as_deref().unwrap_or("none")
        ),
        format!("model_error.retryable={}", error.retryable),
        format!(
            "model_error.message={}",
            truncate_value(error.message.as_str(), 160)
        ),
    ]
}

fn truncate_value(value: &str, max_chars: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= max_chars {
        return value.to_string();
    }
    let keep = max_chars.saturating_sub(3);
    format!("{}...", value.chars().take(keep).collect::<String>())
}

fn parse_status_token(value: &str) -> Option<TaskStatus> {
    match value.trim().to_ascii_lowercase().as_str() {
        "pending" => Some(TaskStatus::Pending),
        "running" => Some(TaskStatus::Running),
        "completed" | "done" => Some(TaskStatus::Completed),
        "failed" => Some(TaskStatus::Failed),
        "killed" | "stopped" => Some(TaskStatus::Killed),
        _ => None,
    }
}

fn parse_type_token(value: &str) -> Option<TaskType> {
    match value.trim().to_ascii_lowercase().as_str() {
        "shell" | "local_shell" | "local-bash" | "bash" => Some(TaskType::LocalShell),
        "agent" | "local_agent" => Some(TaskType::LocalAgent),
        "dream" => Some(TaskType::Dream),
        _ => None,
    }
}

fn parse_result_kind_token(value: &str) -> Option<String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "shell" | "agent" | "dream" => Some(value.trim().to_ascii_lowercase()),
        _ => None,
    }
}

fn parse_bool_token(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "yes" | "true" | "1" => Some(true),
        "no" | "false" | "0" => Some(false),
        _ => None,
    }
}

fn render_task_type(task_type: TaskType) -> &'static str {
    match task_type {
        TaskType::LocalShell => "local_shell",
        TaskType::LocalAgent => "local_agent",
        TaskType::Dream => "dream",
    }
}

fn render_task_status(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Pending => "pending",
        TaskStatus::Running => "running",
        TaskStatus::Completed => "completed",
        TaskStatus::Failed => "failed",
        TaskStatus::Killed => "killed",
    }
}

fn render_dream_phase(phase: DreamPhase) -> &'static str {
    match phase {
        DreamPhase::Starting => "starting",
        DreamPhase::Updating => "updating",
    }
}

fn render_bash_task_kind(kind: BashTaskKind) -> &'static str {
    match kind {
        BashTaskKind::Bash => "bash",
        BashTaskKind::Monitor => "monitor",
    }
}

#[cfg(test)]
mod tests {
    use super::TaskPanelView;
    use nocode_core::{AgentStep, ModelErrorWire, ModelStreamEventWire, TaskCoordinator};
    use serde_json::json;

    #[test]
    fn task_panel_surfaces_agent_result_in_list_and_detail() {
        let mut coordinator = TaskCoordinator::new();
        let agent = coordinator.spawn_local_agent("agent-a".into(), "done".into());
        coordinator.next_pending();
        assert!(
            coordinator.record_agent_step(
                &agent,
                AgentStep::completed(2, 64, true)
                    .with_stream_events(vec![ModelStreamEventWire::Delta {
                        text: String::from("task-panel delta"),
                    }])
                    .with_model_error(ModelErrorWire {
                        surface: String::from("provider"),
                        kind: String::from("structured_output_failure"),
                        provider: Some(String::from("openai-responses")),
                        status_code: Some(400),
                        status_class: Some(String::from("4xx")),
                        retryable: false,
                        message: String::from("schema mismatch"),
                    })
                    .with_response_result(json!({"ok": true, "source": "task-panel"}))
            )
        );

        let panel = TaskPanelView::from_state(&coordinator, None, Some(&agent), Some(&agent));
        let list = panel.render_list_summary();
        let detail = panel.render_detail();

        assert!(list.contains("result={\"kind\":\"agent\""));
        assert!(detail.contains("result={\"kind\":\"agent\""));
        assert!(detail.contains("result.pretty:"));
        assert!(detail.contains("\"response_result\": {"));
        assert!(detail.contains("\"source\": \"task-panel\""));
        assert!(detail.contains("stream_events.count=1"));
        assert!(detail.contains("stream_events.last=delta text=task-panel delta"));
        assert!(detail.contains("model_error.kind=structured_output_failure"));
        assert!(detail.contains("response-result.pretty:"));
    }

    #[test]
    fn task_panel_filters_support_result_kind_and_structured_flags() {
        let mut coordinator = TaskCoordinator::new();
        let shell = coordinator.spawn_local_shell("printf ready".into(), None, None);
        let agent = coordinator.spawn_local_agent("agent-a".into(), "done".into());
        let dream = coordinator.spawn_dream(2, Some(String::from("dream")));
        coordinator.next_pending();
        coordinator.next_pending();
        coordinator.next_pending();
        assert!(coordinator.record_shell_result(
            &shell,
            nocode_core::CommandResult {
                code: 0,
                interrupted: false,
            }
        ));
        assert!(
            coordinator.record_agent_step(
                &agent,
                AgentStep::completed(2, 64, true)
                    .with_response_result(json!({"ok": true, "source": "task-panel"}))
            )
        );
        assert!(coordinator.record_dream_step(
            &dream,
            nocode_core::DreamStep::completed(
                nocode_core::DreamPhase::Updating,
                vec![String::from("a.rs")],
                Some(nocode_core::DreamTurn {
                    text: String::from("vision"),
                    tool_use_count: 1,
                }),
            )
        ));

        let result_agent = TaskPanelView::from_state(
            &coordinator,
            Some("result:agent"),
            Some(&agent),
            Some(&agent),
        );
        let result_structured = TaskPanelView::from_state(
            &coordinator,
            Some("result-structured:yes"),
            Some(&agent),
            Some(&agent),
        );
        let result_unstructured = TaskPanelView::from_state(
            &coordinator,
            Some("result-structured:no"),
            Some(&shell),
            Some(&shell),
        );

        assert!(
            result_agent
                .render_list_summary()
                .contains("a0000000000000002")
        );
        assert!(
            !result_agent
                .render_list_summary()
                .contains("b0000000000000001")
        );
        assert!(
            !result_agent
                .render_list_summary()
                .contains("d0000000000000003")
        );
        assert!(
            result_structured
                .render_list_summary()
                .contains("a0000000000000002")
        );
        assert!(
            !result_structured
                .render_list_summary()
                .contains("b0000000000000001")
        );
        assert!(
            !result_structured
                .render_list_summary()
                .contains("d0000000000000003")
        );
        assert!(
            result_unstructured
                .render_list_summary()
                .contains("b0000000000000001")
        );
        assert!(
            result_unstructured
                .render_list_summary()
                .contains("d0000000000000003")
        );
        assert!(
            !result_unstructured
                .render_list_summary()
                .contains("a0000000000000002")
        );
    }
}
