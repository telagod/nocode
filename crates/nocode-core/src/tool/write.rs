use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

pub struct WriteTool;

impl Tool for WriteTool {
    fn name(&self) -> &str {
        "FileWrite"
    }

    fn description(&self) -> &str {
        "Create or overwrite a file with the given content."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": { "type": "string", "description": "The absolute path to the file to write (must be absolute, not relative)" },
                "content": { "type": "string", "description": "The content to write to the file" }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn write_creates_file() {
        let path = "/tmp/nocode_write_unit.txt";
        let _ = std::fs::remove_file(path);
        let tool = WriteTool;
        let result = tool.execute(&json!({"file_path": path, "content": "hello"}));
        assert!(!result.is_error);
        assert_eq!(std::fs::read_to_string(path).unwrap(), "hello");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn write_overwrites_existing() {
        let path = "/tmp/nocode_write_overwrite.txt";
        std::fs::write(path, "old").unwrap();
        let tool = WriteTool;
        let result = tool.execute(&json!({"file_path": path, "content": "new"}));
        assert!(!result.is_error);
        assert_eq!(std::fs::read_to_string(path).unwrap(), "new");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn write_missing_params() {
        let tool = WriteTool;
        assert!(tool.execute(&json!({})).is_error);
        assert!(tool.execute(&json!({"file_path": "/tmp/x"})).is_error);
    }

    #[test]
    fn write_reports_byte_count() {
        let path = "/tmp/nocode_write_bytes.txt";
        let tool = WriteTool;
        let result = tool.execute(&json!({"file_path": path, "content": "12345"}));
        assert!(result.content.contains("5 bytes"));
        let _ = std::fs::remove_file(path);
    }
}
