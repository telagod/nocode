//! Unified Task tool — single `Task` tool with an `action` parameter dispatching
//! to create, get, list, update, output, and stop operations.

use crate::agent::task::{TaskStatus, global_task_coordinator};
use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};

pub struct TaskTool;

impl Tool for TaskTool {
    fn name(&self) -> &str {
        "Task"
    }

    fn description(&self) -> &str {
        "Manage tasks: create, get, list, update, retrieve output, or stop a task."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create", "get", "list", "update", "output", "stop"],
                    "description": "The task operation to perform"
                },
                "subject": {
                    "type": "string",
                    "description": "Brief title for the task (create)"
                },
                "description": {
                    "type": "string",
                    "description": "What needs to be done (create/update)"
                },
                "taskId": {
                    "type": "string",
                    "description": "Task ID (get/update/output/stop)"
                },
                "status": {
                    "type": "string",
                    "enum": ["pending", "in_progress", "completed", "failed", "deleted"],
                    "description": "New status (update)"
                },
                "activeForm": {
                    "type": "string",
                    "description": "Present continuous form shown in spinner when in_progress (create/update)"
                },
                "owner": {
                    "type": "string",
                    "description": "Owner for the task (update)"
                },
                "addBlocks": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Task IDs that this task blocks (update)"
                },
                "addBlockedBy": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Task IDs that block this task (update)"
                },
                "block": {
                    "type": "boolean",
                    "description": "Whether to wait for completion (output)"
                },
                "timeout": {
                    "type": "number",
                    "description": "Max wait time in ms (output)"
                }
            },
            "required": ["action"]
        })
    }

    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(action) = input["action"].as_str() else {
            return ToolOutput::error("Missing required parameter: action");
        };
        match action {
            "create" => execute_create(input),
            "get" => execute_get(input),
            "list" => execute_list(input),
            "update" => execute_update(input),
            "output" => execute_output(input),
            "stop" => execute_stop(input),
            other => ToolOutput::error(format!(
                "Unknown action '{other}'. Expected one of: create, get, list, update, output, stop"
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Action implementations
// ---------------------------------------------------------------------------

fn execute_create(input: &Value) -> ToolOutput {
    let Some(subject) = input["subject"].as_str() else {
        return ToolOutput::error("Missing required parameter: subject");
    };
    let Some(description) = input["description"].as_str() else {
        return ToolOutput::error("Missing required parameter: description");
    };
    let tc = global_task_coordinator();
    let mut guard = tc.lock().unwrap();
    let id = guard.create(subject, description);
    ToolOutput::success(
        json!({
            "id": id,
            "subject": subject,
            "status": "pending",
        })
        .to_string(),
    )
}

fn execute_get(input: &Value) -> ToolOutput {
    let id = input["taskId"].as_str().or_else(|| input["id"].as_str());
    let Some(id) = id else {
        return ToolOutput::error("Missing required parameter: taskId");
    };
    let tc = global_task_coordinator();
    let guard = tc.lock().unwrap();
    match guard.get(id) {
        Some(task) => ToolOutput::success(
            json!({
                "id": task.id, "subject": task.subject, "description": task.description,
                "status": format!("{:?}", task.status), "owner": task.owner,
                "blocked_by": task.blocked_by, "blocks": task.blocks
            })
            .to_string(),
        ),
        None => ToolOutput::error(format!("Task {id} not found")),
    }
}

fn execute_list(_input: &Value) -> ToolOutput {
    let tc = global_task_coordinator();
    let guard = tc.lock().unwrap();
    let tasks: Vec<Value> = guard
        .list()
        .iter()
        .map(|t| {
            json!({
                "id": t.id, "subject": t.subject, "status": format!("{:?}", t.status),
                "owner": t.owner, "blocked_by": t.blocked_by
            })
        })
        .collect();
    ToolOutput::success(serde_json::to_string(&tasks).unwrap_or_default())
}

fn execute_update(input: &Value) -> ToolOutput {
    // Support both "taskId" and "id" for compatibility
    let id = input["taskId"].as_str().or_else(|| input["id"].as_str());
    let Some(id) = id else {
        return ToolOutput::error("Missing required parameter: taskId");
    };
    let tc = global_task_coordinator();
    let mut guard = tc.lock().unwrap();

    // Status
    if let Some(status) = input["status"].as_str() {
        let s = match status {
            "pending" => TaskStatus::Pending,
            "in_progress" => TaskStatus::InProgress,
            "completed" => TaskStatus::Completed,
            "failed" => TaskStatus::Failed,
            "deleted" => TaskStatus::Deleted,
            _ => TaskStatus::Pending,
        };
        if let Err(e) = guard.set_status(id, s) {
            return ToolOutput::error(e);
        }
    }

    // Owner
    if let Some(owner) = input["owner"].as_str()
        && let Err(e) = guard.set_owner(id, owner)
    {
        return ToolOutput::error(e);
    }

    // addBlockedBy
    if let Some(blockers) = input["addBlockedBy"].as_array() {
        for blocker in blockers {
            if let Some(bid) = blocker.as_str()
                && let Err(e) = guard.add_blocked_by(id, bid)
            {
                return ToolOutput::error(e);
            }
        }
    }

    // addBlocks
    if let Some(blocked) = input["addBlocks"].as_array() {
        for b in blocked {
            if let Some(bid) = b.as_str()
                && let Err(e) = guard.add_blocks(id, bid)
            {
                return ToolOutput::error(e);
            }
        }
    }

    // Subject, description
    if let Some(task) = guard.get_mut(id) {
        if let Some(s) = input["subject"].as_str() {
            task.subject = s.to_string();
        }
        if let Some(d) = input["description"].as_str() {
            task.description = d.to_string();
        }
    }

    ToolOutput::success(format!("Task {id} updated"))
}

fn execute_output(input: &Value) -> ToolOutput {
    let id = input["taskId"]
        .as_str()
        .or_else(|| input["task_id"].as_str())
        .or_else(|| input["id"].as_str());
    let Some(id) = id else {
        return ToolOutput::error("Missing required parameter: taskId");
    };
    let tc = global_task_coordinator();
    let guard = tc.lock().unwrap();
    match guard.get(id) {
        Some(task) => ToolOutput::success(
            json!({
                "id": task.id, "status": format!("{:?}", task.status),
                "description": task.description
            })
            .to_string(),
        ),
        None => ToolOutput::error(format!("Task {id} not found")),
    }
}

fn execute_stop(input: &Value) -> ToolOutput {
    let id = input["taskId"]
        .as_str()
        .or_else(|| input["task_id"].as_str())
        .or_else(|| input["shell_id"].as_str())
        .or_else(|| input["id"].as_str());
    let Some(id) = id else {
        return ToolOutput::error("Missing required parameter: taskId");
    };

    // Try to kill as PID first (background Bash processes)
    if let Ok(pid) = id.parse::<u32>() {
        #[cfg(unix)]
        {
            use std::process::Command;
            let _ = Command::new("kill").arg(pid.to_string()).output();
            return ToolOutput::success(format!("Sent kill signal to PID {pid}"));
        }
        #[cfg(not(unix))]
        {
            let _ = pid;
        }
    }

    // Fall back to task coordinator
    let tc = global_task_coordinator();
    let mut guard = tc.lock().unwrap();
    match guard.set_status(id, TaskStatus::Failed) {
        Ok(()) => ToolOutput::success(format!("Task {id} stopped")),
        Err(e) => ToolOutput::error(e),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod task_tool_tests {
    use super::*;

    #[test]
    fn create_returns_id() {
        let tool = TaskTool;
        let result = tool.execute(
            &json!({"action": "create", "subject": "test task", "description": "do stuff"}),
        );
        assert!(!result.is_error);
        let v: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert!(v["id"].as_str().is_some());
        assert_eq!(v["status"], "pending");
    }

    #[test]
    fn create_missing_subject() {
        let tool = TaskTool;
        let result = tool.execute(&json!({"action": "create", "description": "no subject"}));
        assert!(result.is_error);
    }

    #[test]
    fn create_missing_description() {
        let tool = TaskTool;
        let result = tool.execute(&json!({"action": "create", "subject": "no desc"}));
        assert!(result.is_error);
    }

    #[test]
    fn update_with_task_id_param() {
        let tool = TaskTool;
        let r = tool.execute(&json!({"action": "create", "subject": "s", "description": "d"}));
        let v: serde_json::Value = serde_json::from_str(&r.content).unwrap();
        let id = v["id"].as_str().unwrap();

        let r2 = tool.execute(&json!({"action": "update", "taskId": id, "status": "in_progress"}));
        assert!(!r2.is_error, "Update failed: {}", r2.content);
    }

    #[test]
    fn missing_action() {
        let tool = TaskTool;
        let result = tool.execute(&json!({"subject": "oops"}));
        assert!(result.is_error);
        assert!(result.content.contains("action"));
    }

    #[test]
    fn unknown_action() {
        let tool = TaskTool;
        let result = tool.execute(&json!({"action": "explode"}));
        assert!(result.is_error);
        assert!(result.content.contains("Unknown action"));
    }

    #[test]
    fn list_action_works() {
        let tool = TaskTool;
        let result = tool.execute(&json!({"action": "list"}));
        assert!(!result.is_error);
    }
}
