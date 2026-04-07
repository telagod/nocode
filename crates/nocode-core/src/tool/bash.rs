use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};
use std::process::Command;

pub struct BashTool {
    cwd: String,
}

impl BashTool {
    pub fn new(cwd: impl Into<String>) -> Self {
        Self { cwd: cwd.into() }
    }
}

impl Tool for BashTool {
    fn name(&self) -> &str { "Bash" }

    fn description(&self) -> &str {
        "Execute a shell command and return its output."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "The shell command to execute" },
                "timeout": { "type": "integer", "description": "Timeout in milliseconds (max 600000)" }
            },
            "required": ["command"]
        })
    }

    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(command) = input["command"].as_str() else {
            return ToolOutput::error("Missing required parameter: command");
        };

        let timeout_ms = input["timeout"].as_u64().unwrap_or(120_000).min(600_000);

        let result = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(&self.cwd)
            .output();

        match result {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let mut result = String::new();
                if !stdout.is_empty() {
                    result.push_str(&stdout);
                }
                if !stderr.is_empty() {
                    if !result.is_empty() {
                        result.push('\n');
                    }
                    result.push_str("STDERR:\n");
                    result.push_str(&stderr);
                }
                if result.is_empty() {
                    result = format!("(exit code {})", output.status.code().unwrap_or(-1));
                }
                if output.status.success() {
                    ToolOutput::success(result)
                } else {
                    ToolOutput::error(format!(
                        "Exit code {}\n{result}",
                        output.status.code().unwrap_or(-1)
                    ))
                }
            }
            Err(e) => ToolOutput::error(format!("Failed to execute command: {e}")),
        }
    }
}
