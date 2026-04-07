use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};
use std::process::Command;

pub struct GrepTool;

impl Tool for GrepTool {
    fn name(&self) -> &str {
        "Grep"
    }

    fn description(&self) -> &str {
        "Search file contents using regex patterns. Uses ripgrep if available, falls back to grep."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Regex pattern to search for" },
                "path": { "type": "string", "description": "File or directory to search in" },
                "glob": { "type": "string", "description": "Glob filter for files (e.g. *.rs)" }
            },
            "required": ["pattern"]
        })
    }

    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(pattern) = input["pattern"].as_str() else {
            return ToolOutput::error("Missing required parameter: pattern");
        };

        let path = input["path"].as_str().unwrap_or(".");
        let glob_filter = input["glob"].as_str();

        // Try ripgrep first, fall back to grep
        let mut cmd = if which_exists("rg") {
            let mut c = Command::new("rg");
            c.arg("--no-heading")
                .arg("--line-number")
                .arg("--max-count=50");
            if let Some(g) = glob_filter {
                c.arg("--glob").arg(g);
            }
            c.arg(pattern).arg(path);
            c
        } else {
            let mut c = Command::new("grep");
            c.arg("-rn").arg("--max-count=50");
            if let Some(g) = glob_filter {
                c.arg("--include").arg(g);
            }
            c.arg(pattern).arg(path);
            c
        };

        match cmd.output() {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if stdout.is_empty() {
                    ToolOutput::success("No matches found")
                } else {
                    ToolOutput::success(stdout.to_string())
                }
            }
            Err(e) => ToolOutput::error(format!("Search failed: {e}")),
        }
    }
}

fn which_exists(cmd: &str) -> bool {
    Command::new("which")
        .arg(cmd)
        .output()
        .is_ok_and(|o| o.status.success())
}
