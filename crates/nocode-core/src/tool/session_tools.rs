//! Session control tools — EnterPlanMode, ExitPlanMode, EnterWorktree, ExitWorktree.
//!
//! Plan mode restricts the model to read-only operations while it explores the
//! codebase and designs an approach. Exiting plan mode registers any
//! `allowedPrompts` as temporary permission rules so the model can implement
//! its plan.

use crate::tool::permission::{PermissionRule, RuleAction, global_permission_rules};
use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

// ---------------------------------------------------------------------------
// Plan mode global state
// ---------------------------------------------------------------------------

/// Global plan-mode flag. When true, the ToolExecutor restricts all tool
/// calls to read-only operations (same as `PermissionMode::ReadOnly`).
static PLAN_MODE_ACTIVE: OnceLock<AtomicBool> = OnceLock::new();

/// Returns the global plan-mode flag.
pub fn plan_mode_active() -> &'static AtomicBool {
    PLAN_MODE_ACTIVE.get_or_init(|| AtomicBool::new(false))
}

/// Check whether plan mode is currently active.
pub fn is_plan_mode() -> bool {
    plan_mode_active().load(Ordering::Relaxed)
}

/// Activate plan mode.
pub fn enter_plan_mode() {
    plan_mode_active().store(true, Ordering::Relaxed);
}

/// Deactivate plan mode.
pub fn exit_plan_mode() {
    plan_mode_active().store(false, Ordering::Relaxed);
}

/// Mutex guard for tests that touch plan mode global state.
/// Prevents concurrent test threads from interfering with each other.
#[cfg(test)]
pub(crate) fn plan_mode_test_lock() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::{Mutex, OnceLock};
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

// ---------------------------------------------------------------------------
// EnterPlanMode
// ---------------------------------------------------------------------------

pub struct EnterPlanModeTool;

impl Tool for EnterPlanModeTool {
    fn name(&self) -> &str {
        "EnterPlanMode"
    }
    fn description(&self) -> &str {
        "Enter plan mode to explore the codebase and design an implementation approach before writing code. In plan mode, only read-only tools are available."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }
    fn execute(&self, _input: &Value) -> ToolOutput {
        enter_plan_mode();
        ToolOutput::success(
            "Entered plan mode. You can now explore the codebase with read-only tools. \
             Present your plan for approval, then use ExitPlanMode to begin implementation.",
        )
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
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
        "Exit plan mode and begin implementing the approved plan. Optionally specify allowedPrompts for temporary permission rules."
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
        exit_plan_mode();

        let prompts = input["allowedPrompts"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|p| {
                        let tool = p["tool"].as_str()?;
                        let prompt = p["prompt"].as_str()?;
                        Some((tool.to_string(), prompt.to_string()))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        if prompts.is_empty() {
            return ToolOutput::success("Exited plan mode. Ready to implement.");
        }

        // Register each allowedPrompt as a permission rule
        let rules = global_permission_rules();
        let mut guard = rules.lock().unwrap_or_else(|e| e.into_inner());
        let mut registered = Vec::new();
        for (tool, prompt) in &prompts {
            let _ = guard.add(PermissionRule {
                tool_name: tool.clone(),
                action: RuleAction::Allow,
                argument_pattern: Some(prompt.clone()),
            });
            registered.push(format!("{tool}: {prompt}"));
        }

        ToolOutput::success(format!(
            "Exited plan mode. Registered {} permission rule(s):\n{}",
            registered.len(),
            registered.join("\n")
        ))
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
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

    fn test_lock() -> std::sync::MutexGuard<'static, ()> {
        plan_mode_test_lock()
    }

    #[test]
    fn enter_exit_plan_mode_roundtrip() {
        let _guard = test_lock();
        // Clean slate
        exit_plan_mode();
        assert!(!is_plan_mode());

        // Enter
        let tool = EnterPlanModeTool;
        let result = tool.execute(&json!({}));
        assert!(!result.is_error);
        assert!(result.content.contains("plan mode"));
        assert!(is_plan_mode());

        // Exit
        let tool = ExitPlanModeTool;
        let result = tool.execute(&json!({}));
        assert!(!result.is_error);
        assert!(!is_plan_mode());
    }

    #[test]
    fn exit_plan_mode_with_allowed_prompts() {
        let _guard = test_lock();
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
        assert!(result.content.contains("2 permission rule"));
    }

    #[test]
    fn exit_worktree_missing_path() {
        let tool = ExitWorktreeTool;
        let result = tool.execute(&json!({}));
        assert!(result.is_error);
        assert!(result.content.contains("path"));
    }
}
