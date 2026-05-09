//! Hook runner — executes shell commands before/after tool calls.
//!
//! Hooks are configured in config.toml under `hooks.pre_tool_use` / `hooks.post_tool_use`.
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
    /// `tool_input` is the JSON input passed to the tool (exposed as `NOCODE_TOOL_INPUT`).
    pub fn run_pre_tool_use(
        &self,
        tool_name: &str,
        tool_input: Option<&str>,
    ) -> Result<Vec<HookResult>, HookResult> {
        let mut extra_env = Vec::new();
        if let Some(input) = tool_input {
            extra_env.push(("NOCODE_TOOL_INPUT", input));
        }
        let mut results = Vec::new();
        for hook in &self.pre_tool_use {
            if !matches_filter(hook, tool_name) {
                continue;
            }
            let result = execute_hook(hook, tool_name, &extra_env);
            if !result.allowed() {
                return Err(result);
            }
            results.push(result);
        }
        Ok(results)
    }

    /// Run post_tool_use hooks (informational — cannot deny).
    /// `tool_output` is the tool result content (exposed as `NOCODE_TOOL_OUTPUT`).
    pub fn run_post_tool_use(&self, tool_name: &str, tool_output: Option<&str>) -> Vec<HookResult> {
        let mut extra_env = Vec::new();
        if let Some(output) = tool_output {
            extra_env.push(("NOCODE_TOOL_OUTPUT", output));
        }
        self.post_tool_use
            .iter()
            .filter(|h| matches_filter(h, tool_name))
            .map(|h| execute_hook(h, tool_name, &extra_env))
            .collect()
    }

    /// Run on_submit hooks.
    pub fn run_on_submit(&self) -> Vec<HookResult> {
        self.on_submit
            .iter()
            .map(|h| execute_hook(h, "submit", &[]))
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

fn execute_hook(hook: &HookEntry, tool_name: &str, extra_env: &[(&str, &str)]) -> HookResult {
    let timeout = Duration::from_millis(hook.timeout_ms.unwrap_or(10_000));

    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(&hook.command)
        .env("NOCODE_TOOL_NAME", tool_name)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    // Extra env vars (NOCODE_TOOL_INPUT / NOCODE_TOOL_OUTPUT)
    for (key, val) in extra_env {
        cmd.env(key, val);
    }

    // User-defined env from HookEntry
    for (key, val) in &hook.env {
        cmd.env(key, val);
    }

    let child = cmd.spawn();

    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            return HookResult {
                hook_command: hook.command.clone(),
                exit_code: -1,
                stdout: String::new(),
                stderr: format!("Hook execution failed: {e}"),
            };
        }
    };

    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = child
                    .stdout
                    .take()
                    .map(|mut s| {
                        let mut buf = String::new();
                        use std::io::Read;
                        let _ = s.read_to_string(&mut buf);
                        buf
                    })
                    .unwrap_or_default();
                let stderr = child
                    .stderr
                    .take()
                    .map(|mut s| {
                        let mut buf = String::new();
                        use std::io::Read;
                        let _ = s.read_to_string(&mut buf);
                        buf
                    })
                    .unwrap_or_default();
                return HookResult {
                    hook_command: hook.command.clone(),
                    exit_code: status.code().unwrap_or(-1),
                    stdout,
                    stderr,
                };
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    return HookResult {
                        hook_command: hook.command.clone(),
                        exit_code: -1,
                        stdout: String::new(),
                        stderr: format!("Hook timed out after {}ms", timeout.as_millis()),
                    };
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => {
                return HookResult {
                    hook_command: hook.command.clone(),
                    exit_code: -1,
                    stdout: String::new(),
                    stderr: format!("Hook wait failed: {e}"),
                };
            }
        }
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
        assert!(runner.run_pre_tool_use("Bash", None).is_ok());
        assert!(runner.run_post_tool_use("Bash", None).is_empty());
    }

    #[test]
    fn pre_hook_allows_on_success() {
        let runner = HookRunner::new(
            vec![HookEntry {
                command: "true".to_string(),
                tool_filter: None,
                timeout_ms: None,
                env: Default::default(),
            }],
            Vec::new(),
            Vec::new(),
        );
        let result = runner.run_pre_tool_use("Bash", None);
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
                env: Default::default(),
            }],
            Vec::new(),
            Vec::new(),
        );
        let result = runner.run_pre_tool_use("Bash", None);
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
                env: Default::default(),
            }],
            Vec::new(),
            Vec::new(),
        );
        // Should not match Bash — so no hooks run, allowed
        assert!(runner.run_pre_tool_use("Bash", None).is_ok());
        // Should match Write — hook fails, denied
        assert!(runner.run_pre_tool_use("Write", None).is_err());
    }

    #[test]
    fn wildcard_filter_matches_all() {
        let runner = HookRunner::new(
            vec![HookEntry {
                command: "true".to_string(),
                tool_filter: Some("*".to_string()),
                timeout_ms: None,
                env: Default::default(),
            }],
            Vec::new(),
            Vec::new(),
        );
        assert!(runner.run_pre_tool_use("Bash", None).is_ok());
        assert!(runner.run_pre_tool_use("Read", None).is_ok());
    }

    #[test]
    fn post_hooks_run_regardless() {
        let runner = HookRunner::new(
            Vec::new(),
            vec![HookEntry {
                command: "echo post".to_string(),
                tool_filter: None,
                timeout_ms: None,
                env: Default::default(),
            }],
            Vec::new(),
        );
        let results = runner.run_post_tool_use("Bash", None);
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
                env: Default::default(),
            }],
        );
        let results = runner.run_on_submit();
        assert_eq!(results.len(), 1);
        assert!(results[0].stdout.contains("submitted"));
    }

    #[test]
    fn pre_hook_receives_tool_input() {
        let runner = HookRunner::new(
            vec![HookEntry {
                command: "echo $NOCODE_TOOL_INPUT".to_string(),
                tool_filter: None,
                timeout_ms: None,
                env: Default::default(),
            }],
            Vec::new(),
            Vec::new(),
        );
        let result = runner
            .run_pre_tool_use("Bash", Some(r#"{"command":"ls"}"#))
            .unwrap();
        assert!(result[0].stdout.contains(r#"{"command":"ls"}"#));
    }

    #[test]
    fn post_hook_receives_tool_output() {
        let runner = HookRunner::new(
            Vec::new(),
            vec![HookEntry {
                command: "echo $NOCODE_TOOL_OUTPUT".to_string(),
                tool_filter: None,
                timeout_ms: None,
                env: Default::default(),
            }],
            Vec::new(),
        );
        let results = runner.run_post_tool_use("Bash", Some("file1.rs\nfile2.rs"));
        assert!(results[0].stdout.contains("file1.rs"));
    }

    #[test]
    fn hook_entry_custom_env() {
        let mut env = std::collections::HashMap::new();
        env.insert("MY_VAR".to_string(), "hello_world".to_string());
        let runner = HookRunner::new(
            vec![HookEntry {
                command: "echo $MY_VAR".to_string(),
                tool_filter: None,
                timeout_ms: None,
                env,
            }],
            Vec::new(),
            Vec::new(),
        );
        let result = runner.run_pre_tool_use("Bash", None).unwrap();
        assert!(result[0].stdout.contains("hello_world"));
    }
}
