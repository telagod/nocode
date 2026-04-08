use crate::tool::file_safety;
use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};
use std::fs;

pub struct ReadTool;

impl Tool for ReadTool {
    fn name(&self) -> &str {
        "FileRead"
    }

    fn description(&self) -> &str {
        "Read a file from the filesystem. Returns the file contents with line numbers."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": { "type": "string", "description": "The absolute path to the file to read" },
                "offset": { "type": "number", "description": "The line number to start reading from. Only provide if the file is too large to read at once" },
                "limit": { "type": "number", "description": "The number of lines to read. Only provide if the file is too large to read at once." },
                "pages": { "type": "string", "description": "Page range for PDF files (e.g., \"1-5\", \"3\", \"10-20\"). Only applicable to PDF files. Maximum 20 pages per request." }
            },
            "required": ["file_path"]
        })
    }

    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(path) = input["file_path"].as_str() else {
            return ToolOutput::error("Missing required parameter: file_path");
        };

        // PDF handling
        if path.ends_with(".pdf") {
            let _pages = input["pages"].as_str();
            return ToolOutput::error(
                "PDF reading requires a PDF extraction library. Use an external tool to convert PDF to text first.",
            );
        }

        // Size limit check (10 MB)
        if let Err(e) = file_safety::check_file_size(path) {
            return ToolOutput::error(e);
        }

        // Binary detection
        if file_safety::is_binary_file(path) {
            return ToolOutput::error(format!(
                "{path} appears to be a binary file and cannot be displayed as text"
            ));
        }

        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => return ToolOutput::error(format!("Failed to read {path}: {e}")),
        };

        let offset = input["offset"].as_u64().unwrap_or(0) as usize;
        let limit = input["limit"].as_u64().unwrap_or(2000) as usize;

        let lines: Vec<&str> = content.lines().collect();
        let end = (offset + limit).min(lines.len());
        let selected = &lines[offset.min(lines.len())..end];

        let numbered: String = selected
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{}\t{line}", offset + i + 1))
            .collect::<Vec<_>>()
            .join("\n");

        ToolOutput::success(numbered)
    }
}
