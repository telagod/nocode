//! Session control tools — EnterPlanMode, ExitPlanMode, EnterWorktree, ExitWorktree.

use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// EnterPlanMode
// ---------------------------------------------------------------------------

pub struct EnterPlanModeTool;

impl Tool for EnterPlanModeTool {
    fn name(&self) -> &str {
        "EnterPlanMode"
    }
    fn description(&self) -> &str {
        "Enter plan mode to explore the codebase and design an implementation approach before writing code."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }
    fn execute(&self, _input: &Value) -> ToolOutput {
        ToolOutput::success(
            "Entered plan mode. Explore the codebase and present your plan for approval.",
        )
    }
}

// ---------------------------------------------------------------------------
// ExitPlanMode
// ---------------------------------------------------------------------------

pub struct ExitPlanModeTool;

impl Tool for ExitPlanModeTool {
    fn name(&self) -> &str {
        "ExitPlanMode"
    }
    fn description(&self) -> &str {
        "Exit plan mode and begin implementing the approved plan."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "allowedPrompts": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "tool": { "type": "string" },
                            "prompt": { "type": "string" }
                        }
                    },
                    "description": "Prompt-based permissions needed to implement the plan."
                }
            }
        })
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        let prompts = input["allowedPrompts"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|p| {
                        let tool = p["tool"].as_str()?;
                        let prompt = p["prompt"].as_str()?;
                        Some(format!("{tool}: {prompt}"))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        if prompts.is_empty() {
            ToolOutput::success("Exited plan mode. Ready to implement.")
        } else {
            ToolOutput::success(format!(
                "Exited plan mode. Allowed actions:\n{}",
                prompts.join("\n")
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// EnterWorktree
// ---------------------------------------------------------------------------

pub struct EnterWorktreeTool;

impl Tool for EnterWorktreeTool {
    fn name(&self) -> &str {
        "EnterWorktree"
    }
    fn description(&self) -> &str {
        "Create a temporary git worktree for isolated work."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }
    fn execute(&self, _input: &Value) -> ToolOutput {
        // Create a git worktree in a temp directory
        let tmp = std::env::temp_dir().join(format!("nocode-worktree-{}", std::process::id()));
        let tmp_str = tmp.to_string_lossy().to_string();
        let output = std::process::Command::new("git")
            .args(["worktree", "add", &tmp_str, "HEAD", "--detach"])
            .output();
        match output {
            Ok(o) if o.status.success() => {
                ToolOutput::success(format!("Worktree created at {tmp_str}"))
            }
            Ok(o) => ToolOutput::error(format!(
                "Failed to create worktree: {}",
                String::from_utf8_lossy(&o.stderr)
            )),
            Err(e) => ToolOutput::error(format!("Failed to run git: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// ExitWorktree
// ---------------------------------------------------------------------------

pub struct ExitWorktreeTool;

impl Tool for ExitWorktreeTool {
    fn name(&self) -> &str {
        "ExitWorktree"
    }
    fn description(&self) -> &str {
        "Remove the temporary git worktree and return to the main working directory."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to the worktree to remove" }
            },
            "required": ["path"]
        })
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(path) = input["path"].as_str() else {
            return ToolOutput::error("Missing required parameter: path");
        };
        let output = std::process::Command::new("git")
            .args(["worktree", "remove", path, "--force"])
            .output();
        match output {
            Ok(o) if o.status.success() => ToolOutput::success(format!("Worktree removed: {path}")),
            Ok(o) => ToolOutput::error(format!(
                "Failed to remove worktree: {}",
                String::from_utf8_lossy(&o.stderr)
            )),
            Err(e) => ToolOutput::error(format!("Failed to run git: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_plan_mode_succeeds() {
        let tool = EnterPlanModeTool;
        let result = tool.execute(&json!({}));
        assert!(!result.is_error);
        assert!(result.content.contains("plan mode"));
    }

    #[test]
    fn exit_plan_mode_no_prompts() {
        let tool = ExitPlanModeTool;
        let result = tool.execute(&json!({}));
        assert!(!result.is_error);
        assert!(result.content.contains("Exited plan mode"));
    }

    #[test]
    fn exit_plan_mode_with_allowed_prompts() {
        let tool = ExitPlanModeTool;
        let result = tool.execute(&json!({
            "allowedPrompts": [
                {"tool": "Bash", "prompt": "run tests"},
                {"tool": "Bash", "prompt": "install dependencies"}
            ]
        }));
        assert!(!result.is_error);
        assert!(result.content.contains("Bash: run tests"));
        assert!(result.content.contains("Bash: install dependencies"));
    }

    #[test]
    fn exit_worktree_missing_path() {
        let tool = ExitWorktreeTool;
        let result = tool.execute(&json!({}));
        assert!(result.is_error);
        assert!(result.content.contains("path"));
    }
}
