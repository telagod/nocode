use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};

pub struct GlobTool;

impl Tool for GlobTool {
    fn name(&self) -> &str {
        "Glob"
    }

    fn description(&self) -> &str {
        "Find files matching a glob pattern. Returns matching file paths sorted by modification time."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "The glob pattern to match files against" },
                "path": { "type": "string", "description": "The directory to search in. If not specified, the current working directory will be used." }
            },
            "required": ["pattern"]
        })
    }

    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(pattern) = input["pattern"].as_str() else {
            return ToolOutput::error("Missing required parameter: pattern");
        };

        let base = input["path"].as_str().unwrap_or(".");
        const MAX_RESULTS: usize = 250;

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
        for path in entries.flatten() {
            let mtime = path
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            files.push((mtime, path.display().to_string()));
        }

        files.sort_by(|a, b| b.0.cmp(&a.0));

        if files.is_empty() {
            return ToolOutput::success("No files matched the pattern.");
        }

        let total = files.len();
        let limited: Vec<&str> = files
            .iter()
            .take(MAX_RESULTS)
            .map(|(_, p)| p.as_str())
            .collect();

        let mut result = limited.join("\n");
        if total > MAX_RESULTS {
            result.push_str(&format!("\n... and {} more files", total - MAX_RESULTS));
        }

        ToolOutput::success(result)
    }
}
