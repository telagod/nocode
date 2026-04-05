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
