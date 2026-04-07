use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};
use std::fs;

pub struct EditTool;

impl Tool for EditTool {
    fn name(&self) -> &str {
        "Edit"
    }

    fn description(&self) -> &str {
        "Replace an exact string in a file with new content. The old_string must be unique in the file."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": { "type": "string", "description": "Absolute path to the file" },
                "old_string": { "type": "string", "description": "The exact text to replace" },
                "new_string": { "type": "string", "description": "The replacement text" }
            },
            "required": ["file_path", "old_string", "new_string"]
        })
    }

    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(path) = input["file_path"].as_str() else {
            return ToolOutput::error("Missing required parameter: file_path");
        };
        let Some(old_string) = input["old_string"].as_str() else {
            return ToolOutput::error("Missing required parameter: old_string");
        };
        let Some(new_string) = input["new_string"].as_str() else {
            return ToolOutput::error("Missing required parameter: new_string");
        };

        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => return ToolOutput::error(format!("Failed to read {path}: {e}")),
        };

        let count = content.matches(old_string).count();
        if count == 0 {
            return ToolOutput::error(format!(
                "old_string not found in {path}. Make sure it matches exactly."
            ));
        }
        if count > 1 {
            return ToolOutput::error(format!(
                "old_string found {count} times in {path}. It must be unique. Provide more context."
            ));
        }

        let new_content = content.replacen(old_string, new_string, 1);
        match fs::write(path, &new_content) {
            Ok(()) => ToolOutput::success(format!("Edited {path}")),
            Err(e) => ToolOutput::error(format!("Failed to write {path}: {e}")),
        }
    }
}
