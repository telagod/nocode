use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

pub struct WriteTool;

impl Tool for WriteTool {
    fn name(&self) -> &str {
        "Write"
    }

    fn description(&self) -> &str {
        "Create or overwrite a file with the given content."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": { "type": "string", "description": "Absolute path to the file" },
                "content": { "type": "string", "description": "Content to write" }
            },
            "required": ["file_path", "content"]
        })
    }

    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(path) = input["file_path"].as_str() else {
            return ToolOutput::error("Missing required parameter: file_path");
        };
        let Some(content) = input["content"].as_str() else {
            return ToolOutput::error("Missing required parameter: content");
        };

        // Ensure parent directory exists
        if let Some(parent) = Path::new(path).parent()
            && !parent.exists()
            && fs::create_dir_all(parent).is_err()
        {
            return ToolOutput::error("Failed to create parent directory");
        }

        match fs::write(path, content) {
            Ok(()) => ToolOutput::success(format!("Wrote {path} ({} bytes)", content.len())),
            Err(e) => ToolOutput::error(format!("Failed to write {path}: {e}")),
        }
    }
}
