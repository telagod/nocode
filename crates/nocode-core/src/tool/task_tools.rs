//! Task tools — TaskCreate, TaskGet, TaskList, TaskUpdate, TaskStop, TaskOutput.

use crate::agent::task::{TaskStatus, global_task_coordinator};
use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// TaskCreate
// ---------------------------------------------------------------------------

pub struct TaskCreateTool;

impl Tool for TaskCreateTool {
    fn name(&self) -> &str {
        "TaskCreate"
    }
    fn description(&self) -> &str {
        "Create a new task to track work progress."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "subject": { "type": "string", "description": "Brief title for the task" },
                "description": { "type": "string", "description": "What needs to be done" },
                "activeForm": { "type": "string", "description": "Present continuous form shown in spinner when in_progress" }
            },
            "required": ["subject", "description"]
        })
    }
    fn execute(&self, input: &Value) -> ToolOutput {
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
}

// ---------------------------------------------------------------------------
// TaskGet
// ---------------------------------------------------------------------------

pub struct TaskGetTool;

impl Tool for TaskGetTool {
    fn name(&self) -> &str {
        "TaskGet"
    }
    fn description(&self) -> &str {
        "Get a task by its ID."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"id":{"type":"string"}},"required":["id"]})
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(id) = input["id"].as_str() else {
            return ToolOutput::error("Missing required parameter: id");
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
}

// ---------------------------------------------------------------------------
// TaskList
// ---------------------------------------------------------------------------

pub struct TaskListTool;

impl Tool for TaskListTool {
    fn name(&self) -> &str {
        "TaskList"
    }
    fn description(&self) -> &str {
        "List all tasks."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{}})
    }
    fn execute(&self, _input: &Value) -> ToolOutput {
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
}

// ---------------------------------------------------------------------------
// TaskUpdate
// ---------------------------------------------------------------------------

pub struct TaskUpdateTool;

impl Tool for TaskUpdateTool {
    fn name(&self) -> &str {
        "TaskUpdate"
    }
    fn description(&self) -> &str {
        "Update a task's status, subject, description, owner, or dependencies."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "taskId":{"type":"string","description":"The ID of the task to update"},
            "status":{"type":"string","enum":["pending","in_progress","completed","failed","deleted"]},
            "subject":{"type":"string","description":"New subject for the task"},
            "description":{"type":"string","description":"New description"},
            "activeForm":{"type":"string","description":"Present continuous form for spinner"},
            "owner":{"type":"string","description":"New owner for the task"},
            "addBlocks":{"type":"array","items":{"type":"string"},"description":"Task IDs that this task blocks"},
            "addBlockedBy":{"type":"array","items":{"type":"string"},"description":"Task IDs that block this task"}
        },"required":["taskId"]})
    }
    fn execute(&self, input: &Value) -> ToolOutput {
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

        // Subject, description, activeForm
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
}

// ---------------------------------------------------------------------------
// TaskStop
// ---------------------------------------------------------------------------

pub struct TaskStopTool;

impl Tool for TaskStopTool {
    fn name(&self) -> &str {
        "TaskStop"
    }
    fn description(&self) -> &str {
        "Stop/cancel a running task."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "task_id":{"type":"string","description":"The ID of the background task to stop"},
            "shell_id":{"type":"string","description":"Deprecated: use task_id instead"}
        }})
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        let id = input["task_id"]
            .as_str()
            .or_else(|| input["shell_id"].as_str())
            .or_else(|| input["id"].as_str());
        let Some(id) = id else {
            return ToolOutput::error("Missing required parameter: task_id");
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
}

// ---------------------------------------------------------------------------
// TaskOutput
// ---------------------------------------------------------------------------

pub struct TaskOutputTool;

impl Tool for TaskOutputTool {
    fn name(&self) -> &str {
        "TaskOutput"
    }
    fn description(&self) -> &str {
        "Get the output/result of a completed task."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "task_id":{"type":"string","description":"The task ID to get output from"},
            "block":{"type":"boolean","description":"Whether to wait for completion"},
            "timeout":{"type":"number","description":"Max wait time in ms"}
        },"required":["task_id","block","timeout"]})
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        let id = input["task_id"].as_str().or_else(|| input["id"].as_str());
        let Some(id) = id else {
            return ToolOutput::error("Missing required parameter: task_id");
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
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod task_create_tests {
    use super::*;

    #[test]
    fn create_returns_id() {
        let tool = TaskCreateTool;
        let result = tool.execute(&json!({"subject": "test task", "description": "do stuff"}));
        assert!(!result.is_error);
        let v: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert!(v["id"].as_str().is_some());
        assert_eq!(v["status"], "pending");
    }

    #[test]
    fn create_missing_subject() {
        let tool = TaskCreateTool;
        let result = tool.execute(&json!({"description": "no subject"}));
        assert!(result.is_error);
    }

    #[test]
    fn create_missing_description() {
        let tool = TaskCreateTool;
        let result = tool.execute(&json!({"subject": "no desc"}));
        assert!(result.is_error);
    }

    #[test]
    fn update_with_task_id_param() {
        let create = TaskCreateTool;
        let r = create.execute(&json!({"subject": "s", "description": "d"}));
        let v: serde_json::Value = serde_json::from_str(&r.content).unwrap();
        let id = v["id"].as_str().unwrap();

        let update = TaskUpdateTool;
        let r2 = update.execute(&json!({"taskId": id, "status": "in_progress"}));
        assert!(!r2.is_error, "Update failed: {}", r2.content);
    }
}
