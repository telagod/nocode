use super::model::{
    ToolCallInput, ToolCallOutput, ToolCallResult, ToolExecutionTrace, ToolPermissionDecision,
    ToolProgressUpdate,
};
use crate::message::QueryMessage;
use crate::task_runtime::{
    global_task_coordinator, stop_task, TaskId, TaskPayload, TaskRecord, TaskStatus,
};

fn missing_argument(call: ToolCallInput, key: &str) -> ToolExecutionTrace {
    ToolExecutionTrace {
        progress_updates: Vec::new(),
        result: ToolCallResult::failed(call, format!("missing required argument: {key}")),
        permission_denial: None,
    }
}

fn ok_response(call: &ToolCallInput, tool_label: &str, summary: &str) -> ToolExecutionTrace {
    let progress = ToolProgressUpdate::new(call.tool_use_id.clone(), tool_label.to_string());
    ToolExecutionTrace {
        progress_updates: vec![progress],
        result: ToolPermissionDecision::allow(false).settle(
            call.clone(),
            ToolCallOutput {
                summary: summary.to_string(),
                generated_messages: vec![QueryMessage::assistant(format!(
                    "tool-message: {tool_label} — {summary}"
                ))],
                context_label: Some(call.context_label.clone()),
                progress_updates: vec![ToolProgressUpdate::new(
                    call.tool_use_id.clone(),
                    format!("{tool_label} complete"),
                )],
            },
        ),
        permission_denial: None,
    }
}

fn format_status(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Pending => "pending",
        TaskStatus::Running => "running",
        TaskStatus::Completed => "completed",
        TaskStatus::Failed => "failed",
        TaskStatus::Killed => "killed",
    }
}

fn format_task_type(record: &TaskRecord) -> &'static str {
    match &record.payload {
        TaskPayload::LocalShell(_) => "shell",
        TaskPayload::LocalAgent(_) => "agent",
        TaskPayload::Dream(_) => "dream",
    }
}

fn format_record(record: &TaskRecord) -> String {
    format!(
        "id: {} | type: {} | status: {} | description: {} | start_time: {}",
        record.base.id.as_str(),
        format_task_type(record),
        format_status(record.base.status),
        record.base.description,
        record.base.start_time,
    )
}

/// Retrieve details for a single task by ID.
pub fn execute_task_get(call: ToolCallInput) -> ToolExecutionTrace {
    let Some(task_id) = call.argument("task_id") else {
        return missing_argument(call, "task_id");
    };
    let task_id_str = task_id.to_string();
    let tid = TaskId::from_string(task_id_str.clone());
    let coordinator = global_task_coordinator();
    let guard = coordinator.lock().unwrap();
    match guard.record(&tid) {
        Some(record) => {
            let summary = format_record(record);
            drop(guard);
            ok_response(&call, "TaskGet", &summary)
        }
        None => {
            drop(guard);
            ok_response(&call, "TaskGet", &format!("task {task_id_str} not found"))
        }
    }
}

/// List tasks, optionally filtered by status.
pub fn execute_task_list(call: ToolCallInput) -> ToolExecutionTrace {
    let filter = call.argument("filter").unwrap_or("all").to_string();
    let coordinator = global_task_coordinator();
    let guard = coordinator.lock().unwrap();
    let all_tasks = guard.list_tasks();
    drop(guard);

    let filtered: Vec<&TaskRecord> = all_tasks
        .iter()
        .filter(|r| match filter.as_str() {
            "running" => r.base.status == TaskStatus::Running,
            "completed" => r.base.status == TaskStatus::Completed,
            "failed" => r.base.status == TaskStatus::Failed,
            _ => true,
        })
        .collect();

    let count = filtered.len();
    let mut lines = vec![format!("{count} tasks")];
    for record in &filtered {
        lines.push(format!(
            "{} | {} | {} | {}",
            record.base.id.as_str(),
            format_task_type(record),
            format_status(record.base.status),
            record.base.description,
        ));
    }
    let summary = lines.join("\n");
    ok_response(&call, "TaskList", &summary)
}

/// Update a task's status.
pub fn execute_task_update(call: ToolCallInput) -> ToolExecutionTrace {
    let Some(task_id) = call.argument("task_id") else {
        return missing_argument(call, "task_id");
    };
    let Some(status) = call.argument("status") else {
        return missing_argument(call, "status");
    };
    let task_id_str = task_id.to_string();
    let status_str = status.to_string();
    let tid = TaskId::from_string(task_id_str.clone());
    let coordinator = global_task_coordinator();
    let mut guard = coordinator.lock().unwrap();
    let success = match status_str.as_str() {
        "completed" => guard.complete_task(&tid),
        "failed" => guard.fail_task(&tid),
        other => {
            drop(guard);
            return ok_response(
                &call,
                "TaskUpdate",
                &format!("unsupported status: {other} (expected completed or failed)"),
            );
        }
    };
    drop(guard);
    if success {
        ok_response(
            &call,
            "TaskUpdate",
            &format!("task {task_id_str} marked {status_str}"),
        )
    } else {
        ok_response(
            &call,
            "TaskUpdate",
            &format!("task {task_id_str} not found"),
        )
    }
}

/// Stop a running task.
pub fn execute_task_stop(call: ToolCallInput) -> ToolExecutionTrace {
    let Some(task_id) = call.argument("task_id") else {
        return missing_argument(call, "task_id");
    };
    let task_id_str = task_id.to_string();
    let tid = TaskId::from_string(task_id_str.clone());
    let coordinator = global_task_coordinator();
    let mut guard = coordinator.lock().unwrap();
    match stop_task(&mut guard, &tid) {
        Ok(result) => {
            let summary = format!(
                "stopped task {} (type: {:?}, summary: {})",
                result.task_id.as_str(),
                result.task_type,
                result.summary,
            );
            drop(guard);
            ok_response(&call, "TaskStop", &summary)
        }
        Err(e) => {
            let msg = match e {
                crate::task_runtime::StopTaskError::NotFound => {
                    format!("task {task_id_str} not found")
                }
                crate::task_runtime::StopTaskError::NotRunning => {
                    format!("task {task_id_str} is not running")
                }
                crate::task_runtime::StopTaskError::UnsupportedType => {
                    format!("task {task_id_str} has unsupported type for stop")
                }
            };
            drop(guard);
            ok_response(&call, "TaskStop", &msg)
        }
    }
}

/// Retrieve the full output of a task.
pub fn execute_task_output(call: ToolCallInput) -> ToolExecutionTrace {
    let Some(task_id) = call.argument("task_id") else {
        return missing_argument(call, "task_id");
    };
    let task_id_str = task_id.to_string();
    let tid = TaskId::from_string(task_id_str.clone());
    let coordinator = global_task_coordinator();
    let guard = coordinator.lock().unwrap();
    match guard.record(&tid) {
        Some(record) => {
            let output = match &record.payload {
                TaskPayload::LocalShell(shell) => match &shell.result {
                    Some(r) => format!(
                        "shell result: code={}, interrupted={}",
                        r.code, r.interrupted
                    ),
                    None => "shell: no result yet".to_string(),
                },
                TaskPayload::LocalAgent(agent) => match &agent.response_result {
                    Some(val) => {
                        serde_json::to_string_pretty(val).unwrap_or_else(|_| val.to_string())
                    }
                    None => "agent: no response result yet".to_string(),
                },
                TaskPayload::Dream(dream) => {
                    format!("dream: {} turns", dream.turns.len())
                }
            };
            drop(guard);
            ok_response(&call, "TaskOutput", &output)
        }
        None => {
            drop(guard);
            ok_response(
                &call,
                "TaskOutput",
                &format!("task {task_id_str} not found"),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_execution::ToolCallInput;

    // --- TaskGet ---
    #[test]
    fn task_get_missing_task_id() {
        let call = ToolCallInput::new("TaskGet", "t2");
        let trace = execute_task_get(call);
        assert_eq!(trace.result.status_label(), "failed");
        assert!(trace.result.message().contains("missing required argument: task_id"));
    }

    #[test]
    fn task_get_not_found() {
        let call = ToolCallInput::new("TaskGet", "t1")
            .with_argument("task_id", "nonexistent_id");
        let trace = execute_task_get(call);
        assert_eq!(trace.result.status_label(), "completed");
        assert!(trace.result.message().contains("not found"));
    }

    #[test]
    fn task_get_finds_existing_task() {
        let coordinator = global_task_coordinator();
        let tid = {
            let mut guard = coordinator.lock().unwrap();
            guard.spawn_local_shell("echo hello".to_string(), Some("test task".to_string()), None)
        };
        let call = ToolCallInput::new("TaskGet", "tg1")
            .with_argument("task_id", tid.as_str());
        let trace = execute_task_get(call);
        assert_eq!(trace.result.status_label(), "completed");
        let msg = trace.result.message();
        assert!(msg.contains(tid.as_str()));
        assert!(msg.contains("shell"));
        assert!(msg.contains("test task"));
    }

    // --- TaskList ---
    #[test]
    fn task_list_returns_count() {
        let call = ToolCallInput::new("TaskList", "t3");
        let trace = execute_task_list(call);
        assert_eq!(trace.result.status_label(), "completed");
        assert!(trace.result.message().contains("tasks"));
    }

    #[test]
    fn task_list_with_filter() {
        let call = ToolCallInput::new("TaskList", "t4")
            .with_argument("filter", "running");
        let trace = execute_task_list(call);
        assert_eq!(trace.result.status_label(), "completed");
    }

    #[test]
    fn task_list_filters_by_status() {
        let coordinator = global_task_coordinator();
        let tid = {
            let mut guard = coordinator.lock().unwrap();
            let tid = guard.spawn_local_shell(
                "echo filter_test".to_string(),
                Some("filter test".to_string()),
                None,
            );
            guard.complete_task(&tid);
            tid
        };
        let call = ToolCallInput::new("TaskList", "tl_filter")
            .with_argument("filter", "completed");
        let trace = execute_task_list(call);
        let msg = trace.result.message();
        assert!(msg.contains(tid.as_str()));
        assert!(msg.contains("completed"));
    }

    // --- TaskUpdate ---
    #[test]
    fn task_update_missing_task_id() {
        let call = ToolCallInput::new("TaskUpdate", "t6")
            .with_argument("status", "completed");
        let trace = execute_task_update(call);
        assert_eq!(trace.result.status_label(), "failed");
        assert!(trace.result.message().contains("task_id"));
    }

    #[test]
    fn task_update_missing_status() {
        let call = ToolCallInput::new("TaskUpdate", "t7")
            .with_argument("task_id", "a0000000000000001");
        let trace = execute_task_update(call);
        assert_eq!(trace.result.status_label(), "failed");
        assert!(trace.result.message().contains("status"));
    }

    #[test]
    fn task_update_not_found() {
        let call = ToolCallInput::new("TaskUpdate", "tu1")
            .with_argument("task_id", "nonexistent")
            .with_argument("status", "completed");
        let trace = execute_task_update(call);
        assert_eq!(trace.result.status_label(), "completed");
        assert!(trace.result.message().contains("not found"));
    }

    // --- TaskStop ---
    #[test]
    fn task_stop_missing_task_id() {
        let call = ToolCallInput::new("TaskStop", "t9");
        let trace = execute_task_stop(call);
        assert_eq!(trace.result.status_label(), "failed");
        assert!(trace.result.message().contains("task_id"));
    }

    #[test]
    fn task_stop_not_found() {
        let call = ToolCallInput::new("TaskStop", "ts1")
            .with_argument("task_id", "nonexistent");
        let trace = execute_task_stop(call);
        assert_eq!(trace.result.status_label(), "completed");
        assert!(trace.result.message().contains("not found"));
    }

    // --- TaskOutput ---
    #[test]
    fn task_output_missing_task_id() {
        let call = ToolCallInput::new("TaskOutput", "t11");
        let trace = execute_task_output(call);
        assert_eq!(trace.result.status_label(), "failed");
        assert!(trace.result.message().contains("task_id"));
    }

    #[test]
    fn task_output_not_found() {
        let call = ToolCallInput::new("TaskOutput", "to1")
            .with_argument("task_id", "nonexistent");
        let trace = execute_task_output(call);
        assert_eq!(trace.result.status_label(), "completed");
        assert!(trace.result.message().contains("not found"));
    }

    #[test]
    fn task_output_shell_no_result() {
        let coordinator = global_task_coordinator();
        let tid = {
            let mut guard = coordinator.lock().unwrap();
            guard.spawn_local_shell(
                "echo output_test".to_string(),
                Some("output test".to_string()),
                None,
            )
        };
        let call = ToolCallInput::new("TaskOutput", "to2")
            .with_argument("task_id", tid.as_str());
        let trace = execute_task_output(call);
        assert_eq!(trace.result.status_label(), "completed");
        assert!(trace.result.message().contains("shell: no result yet"));
    }
}
