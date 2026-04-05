use super::model::{
    ToolCallInput, ToolCallOutput, ToolCallResult, ToolExecutionTrace, ToolPermissionDecision,
    ToolProgressUpdate,
};
use crate::message::QueryMessage;

fn missing_argument(call: ToolCallInput, key: &str) -> ToolExecutionTrace {
    ToolExecutionTrace {
        progress_updates: Vec::new(),
        result: ToolCallResult::failed(call, format!("missing required argument: {key}")),
        permission_denial: None,
    }
}

fn stub_response(call: &ToolCallInput, tool_label: &str, summary: &str) -> ToolExecutionTrace {
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

/// Retrieve details for a single task by ID.
pub fn execute_task_get(call: ToolCallInput) -> ToolExecutionTrace {
    let Some(task_id) = call.argument("task_id") else {
        return missing_argument(call, "task_id");
    };
    let task_id = task_id.to_string();
    stub_response(
        &call,
        "TaskGet",
        &format!("task system: task {task_id} not found (task runtime not connected)"),
    )
}

/// List tasks, optionally filtered by status.
pub fn execute_task_list(call: ToolCallInput) -> ToolExecutionTrace {
    // filter is optional — "all", "running", "completed", "failed"
    let _filter = call.argument("filter").unwrap_or("all").to_string();
    stub_response(
        &call,
        "TaskList",
        "task system: 0 tasks (task runtime not connected)",
    )
}

/// Update a task's status.
pub fn execute_task_update(call: ToolCallInput) -> ToolExecutionTrace {
    let Some(task_id) = call.argument("task_id") else {
        return missing_argument(call, "task_id");
    };
    let Some(status) = call.argument("status") else {
        return missing_argument(call, "status");
    };
    let task_id = task_id.to_string();
    let _status = status.to_string();
    let _message = call.argument("message").map(ToString::to_string);
    stub_response(
        &call,
        "TaskUpdate",
        &format!("task system: update queued for {task_id} (task runtime not connected)"),
    )
}

/// Stop a running task.
pub fn execute_task_stop(call: ToolCallInput) -> ToolExecutionTrace {
    let Some(task_id) = call.argument("task_id") else {
        return missing_argument(call, "task_id");
    };
    let task_id = task_id.to_string();
    stub_response(
        &call,
        "TaskStop",
        &format!("task system: stop requested for {task_id} (task runtime not connected)"),
    )
}

/// Retrieve the full output of a task.
pub fn execute_task_output(call: ToolCallInput) -> ToolExecutionTrace {
    let Some(task_id) = call.argument("task_id") else {
        return missing_argument(call, "task_id");
    };
    let task_id = task_id.to_string();
    stub_response(
        &call,
        "TaskOutput",
        &format!("task system: no output for {task_id} (task runtime not connected)"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_execution::ToolCallInput;

    // --- TaskGet ---
    #[test]
    fn task_get_returns_stub() {
        let call = ToolCallInput::new("TaskGet", "t1")
            .with_argument("task_id", "a0000000000000001");
        let trace = execute_task_get(call);
        assert_eq!(trace.result.status_label(), "completed");
        assert!(trace.result.message().contains("task runtime not connected"));
    }

    #[test]
    fn task_get_missing_task_id() {
        let call = ToolCallInput::new("TaskGet", "t2");
        let trace = execute_task_get(call);
        assert_eq!(trace.result.status_label(), "failed");
        assert!(trace.result.message().contains("missing required argument: task_id"));
    }

    // --- TaskList ---
    #[test]
    fn task_list_returns_stub() {
        let call = ToolCallInput::new("TaskList", "t3");
        let trace = execute_task_list(call);
        assert_eq!(trace.result.status_label(), "completed");
    }

    #[test]
    fn task_list_with_filter() {
        let call = ToolCallInput::new("TaskList", "t4")
            .with_argument("filter", "running");
        let trace = execute_task_list(call);
        assert_eq!(trace.result.status_label(), "completed");
    }

    // --- TaskUpdate ---
    #[test]
    fn task_update_returns_stub() {
        let call = ToolCallInput::new("TaskUpdate", "t5")
            .with_argument("task_id", "a0000000000000001")
            .with_argument("status", "completed");
        let trace = execute_task_update(call);
        assert_eq!(trace.result.status_label(), "completed");
    }

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

    // --- TaskStop ---
    #[test]
    fn task_stop_returns_stub() {
        let call = ToolCallInput::new("TaskStop", "t8")
            .with_argument("task_id", "a0000000000000001");
        let trace = execute_task_stop(call);
        assert_eq!(trace.result.status_label(), "completed");
    }

    #[test]
    fn task_stop_missing_task_id() {
        let call = ToolCallInput::new("TaskStop", "t9");
        let trace = execute_task_stop(call);
        assert_eq!(trace.result.status_label(), "failed");
        assert!(trace.result.message().contains("task_id"));
    }

    // --- TaskOutput ---
    #[test]
    fn task_output_returns_stub() {
        let call = ToolCallInput::new("TaskOutput", "t10")
            .with_argument("task_id", "a0000000000000001");
        let trace = execute_task_output(call);
        assert_eq!(trace.result.status_label(), "completed");
    }

    #[test]
    fn task_output_missing_task_id() {
        let call = ToolCallInput::new("TaskOutput", "t11");
        let trace = execute_task_output(call);
        assert_eq!(trace.result.status_label(), "failed");
        assert!(trace.result.message().contains("task_id"));
    }
}
