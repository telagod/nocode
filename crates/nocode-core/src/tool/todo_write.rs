//! TodoWrite tool — batch write/replace the entire todo list.

use crate::agent::task::{TaskStatus, global_task_coordinator};
use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};

pub struct TodoWriteTool;

impl Tool for TodoWriteTool {
    fn name(&self) -> &str {
        "TodoWrite"
    }
    fn description(&self) -> &str {
        "Write the entire todo list. Replaces all existing todos with the provided list."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "content": { "type": "string" },
                            "status": { "type": "string", "enum": ["pending", "in_progress", "completed"] },
                            "activeForm": { "type": "string" }
                        },
                        "required": ["content", "status", "activeForm"]
                    }
                }
            },
            "required": ["todos"]
        })
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(todos) = input["todos"].as_array() else {
            return ToolOutput::error("Missing required parameter: todos");
        };

        let tc = global_task_coordinator();
        let mut guard = tc.lock().unwrap();

        // Clear existing tasks
        guard.clear();

        // Create new tasks from the provided list
        let mut created = Vec::new();
        for todo in todos {
            let content = todo["content"].as_str().unwrap_or("");
            let status_str = todo["status"].as_str().unwrap_or("pending");
            let status = match status_str {
                "in_progress" => TaskStatus::InProgress,
                "completed" => TaskStatus::Completed,
                _ => TaskStatus::Pending,
            };

            let id = guard.create(content, content);
            let _ = guard.set_status(&id, status);
            created.push(json!({"id": id, "content": content, "status": status_str}));
        }

        ToolOutput::success(json!({"todos": created}).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn todo_write_creates_tasks() {
        let tool = TodoWriteTool;
        let result = tool.execute(&json!({
            "todos": [
                {"content": "task one", "status": "pending", "activeForm": "Working on task one"},
                {"content": "task two", "status": "in_progress", "activeForm": "Working on task two"}
            ]
        }));
        assert!(!result.is_error, "Failed: {}", result.content);
        let v: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(v["todos"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn todo_write_missing_todos() {
        let tool = TodoWriteTool;
        let result = tool.execute(&json!({}));
        assert!(result.is_error);
    }

    #[test]
    fn todo_write_replaces_existing() {
        let tool = TodoWriteTool;
        // First write
        tool.execute(
            &json!({"todos": [{"content": "old", "status": "pending", "activeForm": "old"}]}),
        );
        // Second write replaces
        let result = tool.execute(&json!({"todos": [
            {"content": "new1", "status": "completed", "activeForm": "new1"},
            {"content": "new2", "status": "pending", "activeForm": "new2"}
        ]}));
        assert!(!result.is_error);
        let v: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(v["todos"].as_array().unwrap().len(), 2);
    }
}
