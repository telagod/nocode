use super::model::{
    ToolCallInput, ToolCallOutput, ToolExecutionTrace, ToolPermissionDecision, ToolProgressUpdate,
};
use crate::message::QueryMessage;

/// Execute a TeamCreate tool call.
///
/// Required arguments:
/// - `team_name`: name of the team to create
/// - `subtasks`: semicolon-separated list of subtask descriptions
pub fn execute_team_create(call: ToolCallInput) -> ToolExecutionTrace {
    let Some(team_name) = call.argument("team_name") else {
        return missing_argument(call, "team_name");
    };
    let Some(subtasks_raw) = call.argument("subtasks") else {
        return missing_argument(call, "subtasks");
    };

    let team_name = team_name.to_string();
    let subtasks: Vec<&str> = subtasks_raw
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let count = subtasks.len();

    let listing = subtasks
        .iter()
        .enumerate()
        .map(|(i, task)| format!("  {}. {task}", i + 1))
        .collect::<Vec<_>>()
        .join("\n");

    let summary = format!("team {team_name} created with {count} subtasks");
    let detail = format!("{summary}\n{listing}");
    let progress =
        ToolProgressUpdate::new(call.tool_use_id.clone(), format!("creating team {team_name}"));

    ToolExecutionTrace {
        progress_updates: vec![progress],
        result: ToolPermissionDecision::allow(false).settle(
            call.clone(),
            ToolCallOutput {
                summary: summary.clone(),
                generated_messages: vec![QueryMessage::assistant(format!(
                    "tool-message: {detail}"
                ))],
                context_label: Some(call.context_label.clone()),
                progress_updates: vec![ToolProgressUpdate::new(
                    call.tool_use_id,
                    format!("team created: {team_name}"),
                )],
            },
        ),
        permission_denial: None,
    }
}

/// Execute a TeamDelete tool call.
///
/// Required arguments:
/// - `team_name`: name of the team to delete
pub fn execute_team_delete(call: ToolCallInput) -> ToolExecutionTrace {
    let Some(team_name) = call.argument("team_name") else {
        return missing_argument(call, "team_name");
    };

    let team_name = team_name.to_string();
    let summary = format!("team {team_name} deleted");
    let progress =
        ToolProgressUpdate::new(call.tool_use_id.clone(), format!("deleting team {team_name}"));

    ToolExecutionTrace {
        progress_updates: vec![progress],
        result: ToolPermissionDecision::allow(false).settle(
            call.clone(),
            ToolCallOutput {
                summary: summary.clone(),
                generated_messages: vec![QueryMessage::assistant(format!(
                    "tool-message: {summary}"
                ))],
                context_label: Some(call.context_label.clone()),
                progress_updates: vec![ToolProgressUpdate::new(
                    call.tool_use_id,
                    format!("team deleted: {team_name}"),
                )],
            },
        ),
        permission_denial: None,
    }
}

fn missing_argument(call: ToolCallInput, key: &str) -> ToolExecutionTrace {
    ToolExecutionTrace {
        progress_updates: Vec::new(),
        result: super::model::ToolCallResult::failed(
            call,
            format!("missing required argument: {key}"),
        ),
        permission_denial: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_execution::model::ToolCallInput;

    #[test]
    fn team_create_parses_subtasks() {
        let call = ToolCallInput::new("TeamCreate", "toolu-tc1")
            .with_argument("team_name", "alpha")
            .with_argument("subtasks", "recon target;exploit vuln;write report")
            .with_context_label("test");

        let trace = execute_team_create(call);
        assert_eq!(trace.result.status_label(), "completed");
        let msg = trace.result.message();
        assert!(msg.contains("team alpha created with 3 subtasks"), "{msg}");
    }

    #[test]
    fn team_create_missing_name_fails() {
        let call = ToolCallInput::new("TeamCreate", "toolu-tc2")
            .with_argument("subtasks", "a;b")
            .with_context_label("test");

        let trace = execute_team_create(call);
        assert_eq!(trace.result.status_label(), "failed");
        assert!(
            trace.result.message().contains("missing required argument: team_name"),
            "{}",
            trace.result.message()
        );
    }

    #[test]
    fn team_delete_returns_confirmation() {
        let call = ToolCallInput::new("TeamDelete", "toolu-td1")
            .with_argument("team_name", "alpha")
            .with_context_label("test");

        let trace = execute_team_delete(call);
        assert_eq!(trace.result.status_label(), "completed");
        let msg = trace.result.message();
        assert!(msg.contains("team alpha deleted"), "{msg}");
    }
}
