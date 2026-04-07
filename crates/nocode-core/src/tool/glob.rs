use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};
use std::path::Path;

pub struct GlobTool;

impl Tool for GlobTool {
    fn name(&self) -> &str { "Glob" }

    fn description(&self) -> &str {
        "Find files matching a glob pattern. Returns matching file paths sorted by modification time."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Glob pattern (e.g. **/*.rs)" },
                "path": { "type": "string", "description": "Directory to search in (defaults to cwd)" }
            },
            "required": ["pattern"]
        })
    }

    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(pattern) = input["pattern"].as_str() else {
            return ToolOutput::error("Missing required parameter: pattern");
        };

        let base = input["path"].as_str().unwrap_or(".");
        let full_pattern = if pattern.starts_with('/') {
            pattern.to_string()
        } else {
            format!("{base}/{pattern}")
        };

        let entries = match glob::glob(&full_pattern) {
            Ok(paths) => paths,
            Err(e) => return ToolOutput::error(format!("Invalid glob pattern: {e}")),
        };

        let mut files: Vec<(std::time::SystemTime, String)> = Vec::new();
        for entry in entries {
            if let Ok(path) = entry {
                let mtime = path
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                files.push((mtime, path.display().to_string()));
            }
        }

        files.sort_by(|a, b| b.0.cmp(&a.0));

        if files.is_empty() {
            return ToolOutput::success("No files matched the pattern.");
        }

        let result = files
            .iter()
            .map(|(_, p)| p.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        ToolOutput::success(result)
    }
}
