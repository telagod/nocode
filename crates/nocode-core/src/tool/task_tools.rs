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
        "Update a task's status, subject, description, or owner."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "id":{"type":"string"},"status":{"type":"string"},
            "subject":{"type":"string"},"description":{"type":"string"},
            "owner":{"type":"string"}
        },"required":["id"]})
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(id) = input["id"].as_str() else {
            return ToolOutput::error("Missing required parameter: id");
        };
        let tc = global_task_coordinator();
        let mut guard = tc.lock().unwrap();
        if let Some(status) = input["status"].as_str() {
            let s = match status {
                "pending" => TaskStatus::Pending,
                "in_progress" => TaskStatus::InProgress,
                "completed" => TaskStatus::Completed,
                "deleted" => TaskStatus::Deleted,
                _ => TaskStatus::Pending,
            };
            if let Err(e) = guard.set_status(id, s) {
                return ToolOutput::error(e);
            }
        }
        if let Some(owner) = input["owner"].as_str()
            && let Err(e) = guard.set_owner(id, owner)
        {
            return ToolOutput::error(e);
        }
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
        json!({"type":"object","properties":{"id":{"type":"string"}},"required":["id"]})
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(id) = input["id"].as_str() else {
            return ToolOutput::error("Missing required parameter: id");
        };
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
                    "id": task.id, "status": format!("{:?}", task.status),
                    "description": task.description
                })
                .to_string(),
            ),
            None => ToolOutput::error(format!("Task {id} not found")),
        }
    }
}
