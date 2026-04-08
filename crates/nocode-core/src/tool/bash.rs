use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};
use std::process::Command;
use std::time::{Duration, Instant};

pub struct BashTool {
    cwd: String,
}

impl BashTool {
    pub fn new(cwd: impl Into<String>) -> Self {
        Self { cwd: cwd.into() }
    }
}

impl Tool for BashTool {
    fn name(&self) -> &str {
        "Bash"
    }

    fn description(&self) -> &str {
        "Execute a shell command and return its output."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "The command to execute" },
                "timeout": { "type": "number", "description": "Optional timeout in milliseconds (max 600000)" },
                "description": { "type": "string", "description": "Clear, concise description of what this command does in active voice." },
                "run_in_background": { "type": "boolean", "description": "Set to true to run this command in the background. Use Read to read the output later." },
                "dangerouslyDisableSandbox": { "type": "boolean", "description": "Set this to true to dangerously override sandbox mode and run commands without sandboxing." }
            },
            "required": ["command"]
        })
    }

    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(command) = input["command"].as_str() else {
            return ToolOutput::error("Missing required parameter: command");
        };

        let timeout_ms = input["timeout"].as_u64().unwrap_or(120_000).min(600_000);
        let run_in_background = input["run_in_background"].as_bool().unwrap_or(false);

        if run_in_background {
            // Background execution: spawn and return immediately with PID
            let child = match Command::new("sh")
                .arg("-c")
                .arg(command)
                .current_dir(&self.cwd)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                Ok(c) => c,
                Err(e) => return ToolOutput::error(format!("Failed to execute command: {e}")),
            };
            return ToolOutput::success(
                json!({
                    "pid": child.id(),
                    "background": true,
                    "command": command
                })
                .to_string(),
            );
        }

        let timeout = Duration::from_millis(timeout_ms);

        let mut child = match Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(&self.cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => return ToolOutput::error(format!("Failed to execute command: {e}")),
        };

        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_status)) => break,
                Ok(None) => {
                    if start.elapsed() >= timeout {
                        let _ = child.kill();
                        let _ = child.wait();
                        return ToolOutput::error(
                            json!({
                                "stdout": "",
                                "stderr": format!("Command timed out after {timeout_ms}ms"),
                                "exit_code": -1,
                                "timed_out": true
                            })
                            .to_string(),
                        );
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => {
                    return ToolOutput::error(format!("Failed to wait for command: {e}"));
                }
            }
        }

        let output = match child.wait_with_output() {
            Ok(o) => o,
            Err(e) => return ToolOutput::error(format!("Failed to read output: {e}")),
        };

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let exit_code = output.status.code().unwrap_or(-1);

        let result_json = json!({
            "stdout": stdout,
            "stderr": stderr,
            "exit_code": exit_code
        });

        if output.status.success() {
            ToolOutput::success(result_json.to_string())
        } else {
            ToolOutput::error(result_json.to_string())
        }
    }
}
