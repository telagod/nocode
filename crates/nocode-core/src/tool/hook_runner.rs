//! Hook runner — executes shell commands before/after tool calls.
//!
//! Hooks are configured in settings.json under `hooks.pre_tool_use` / `hooks.post_tool_use`.
//! PreToolUse hooks can deny execution by returning non-zero exit code.

use crate::config::runtime::HookEntry;
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// Result of running a hook.
#[derive(Debug, Clone)]
pub struct HookResult {
    pub hook_command: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl HookResult {
    pub fn allowed(&self) -> bool {
        self.exit_code == 0
    }
}

/// Runs configured hooks at tool execution boundaries.
pub struct HookRunner {
    pre_tool_use: Vec<HookEntry>,
    post_tool_use: Vec<HookEntry>,
    on_submit: Vec<HookEntry>,
}

impl HookRunner {
    pub fn new(
        pre_tool_use: Vec<HookEntry>,
        post_tool_use: Vec<HookEntry>,
        on_submit: Vec<HookEntry>,
    ) -> Self {
        Self {
            pre_tool_use,
            post_tool_use,
            on_submit,
        }
    }

    pub fn empty() -> Self {
        Self::new(Vec::new(), Vec::new(), Vec::new())
    }

    /// Run pre_tool_use hooks. Returns Err(HookResult) if any hook denies.
    pub fn run_pre_tool_use(&self, tool_name: &str) -> Result<Vec<HookResult>, HookResult> {
        let mut results = Vec::new();
        for hook in &self.pre_tool_use {
            if !matches_filter(hook, tool_name) {
                continue;
            }
            let result = execute_hook(hook, tool_name);
            if !result.allowed() {
                return Err(result);
            }
            results.push(result);
        }
        Ok(results)
    }

    /// Run post_tool_use hooks (informational — cannot deny).
    pub fn run_post_tool_use(&self, tool_name: &str) -> Vec<HookResult> {
        self.post_tool_use
            .iter()
            .filter(|h| matches_filter(h, tool_name))
            .map(|h| execute_hook(h, tool_name))
            .collect()
    }

    /// Run on_submit hooks.
    pub fn run_on_submit(&self) -> Vec<HookResult> {
        self.on_submit
            .iter()
            .map(|h| execute_hook(h, "submit"))
            .collect()
    }

    pub fn has_pre_hooks(&self) -> bool {
        !self.pre_tool_use.is_empty()
    }

    pub fn has_post_hooks(&self) -> bool {
        !self.post_tool_use.is_empty()
    }
}

fn matches_filter(hook: &HookEntry, tool_name: &str) -> bool {
    match &hook.tool_filter {
        None => true,
        Some(filter) => filter == tool_name || filter == "*",
    }
}

fn execute_hook(hook: &HookEntry, tool_name: &str) -> HookResult {
    let timeout = Duration::from_millis(hook.timeout_ms.unwrap_or(10_000));

    let output = Command::new("sh")
        .arg("-c")
        .arg(&hook.command)
        .env("NOCODE_TOOL_NAME", tool_name)
        .output();

    match output {
        Ok(out) => {
            let _ = timeout; // timeout enforcement would need spawn + wait_timeout
            HookResult {
                hook_command: hook.command.clone(),
                exit_code: out.status.code().unwrap_or(-1),
                stdout: String::from_utf8_lossy(&out.stdout).to_string(),
                stderr: String::from_utf8_lossy(&out.stderr).to_string(),
            }
        }
        Err(e) => HookResult {
            hook_command: hook.command.clone(),
            exit_code: -1,
            stdout: String::new(),
            stderr: format!("Hook execution failed: {e}"),
        },
    }
}

/// Global singleton hook runner.
static GLOBAL_HOOK_RUNNER: OnceLock<Arc<Mutex<HookRunner>>> = OnceLock::new();

pub fn global_hook_runner() -> &'static Arc<Mutex<HookRunner>> {
    GLOBAL_HOOK_RUNNER.get_or_init(|| Arc::new(Mutex::new(HookRunner::empty())))
}

/// Initialize the global hook runner with config.
pub fn init_global_hook_runner(runner: HookRunner) {
    let global = global_hook_runner();
    let mut guard = global.lock().unwrap_or_else(|e| e.into_inner());
    *guard = runner;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::runtime::HookEntry;

    #[test]
    fn empty_runner_allows_all() {
        let runner = HookRunner::empty();
        assert!(runner.run_pre_tool_use("Bash").is_ok());
        assert!(runner.run_post_tool_use("Bash").is_empty());
    }

    #[test]
    fn pre_hook_allows_on_success() {
        let runner = HookRunner::new(
            vec![HookEntry {
                command: "true".to_string(),
                tool_filter: None,
                timeout_ms: None,
            }],
            Vec::new(),
            Vec::new(),
        );
        let result = runner.run_pre_tool_use("Bash");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 1);
    }

    #[test]
    fn pre_hook_denies_on_failure() {
        let runner = HookRunner::new(
            vec![HookEntry {
                command: "false".to_string(),
                tool_filter: None,
                timeout_ms: None,
            }],
            Vec::new(),
            Vec::new(),
        );
        let result = runner.run_pre_tool_use("Bash");
        assert!(result.is_err());
        assert!(!result.unwrap_err().allowed());
    }

    #[test]
    fn tool_filter_matches() {
        let runner = HookRunner::new(
            vec![HookEntry {
                command: "false".to_string(),
                tool_filter: Some("Write".to_string()),
                timeout_ms: None,
            }],
            Vec::new(),
            Vec::new(),
        );
        // Should not match Bash — so no hooks run, allowed
        assert!(runner.run_pre_tool_use("Bash").is_ok());
        // Should match Write — hook fails, denied
        assert!(runner.run_pre_tool_use("Write").is_err());
    }

    #[test]
    fn wildcard_filter_matches_all() {
        let runner = HookRunner::new(
            vec![HookEntry {
                command: "true".to_string(),
                tool_filter: Some("*".to_string()),
                timeout_ms: None,
            }],
            Vec::new(),
            Vec::new(),
        );
        assert!(runner.run_pre_tool_use("Bash").is_ok());
        assert!(runner.run_pre_tool_use("Read").is_ok());
    }

    #[test]
    fn post_hooks_run_regardless() {
        let runner = HookRunner::new(
            Vec::new(),
            vec![HookEntry {
                command: "echo post".to_string(),
                tool_filter: None,
                timeout_ms: None,
            }],
            Vec::new(),
        );
        let results = runner.run_post_tool_use("Bash");
        assert_eq!(results.len(), 1);
        assert!(results[0].stdout.contains("post"));
    }

    #[test]
    fn on_submit_hooks() {
        let runner = HookRunner::new(
            Vec::new(),
            Vec::new(),
            vec![HookEntry {
                command: "echo submitted".to_string(),
                tool_filter: None,
                timeout_ms: None,
            }],
        );
        let results = runner.run_on_submit();
        assert_eq!(results.len(), 1);
        assert!(results[0].stdout.contains("submitted"));
    }
}
