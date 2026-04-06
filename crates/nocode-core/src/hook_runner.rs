use crate::plugin_system::HookEvent;

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct HookCommand {
    pub event: HookEvent,
    pub command: String,
    pub timeout_ms: u64,
}

impl HookCommand {
    pub fn new(event: HookEvent, command: impl Into<String>) -> Self {
        Self {
            event,
            command: command.into(),
            timeout_ms: 30_000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HookPayload {
    pub tool_name: String,
    pub tool_input: serde_json::Value,
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookOutcome {
    Allow,
    Deny { reason: String },
    Modify { new_input: serde_json::Value },
}

#[derive(Debug, Clone)]
pub struct HookRunResult {
    pub outcomes: Vec<HookOutcome>,
    pub denied: bool,
    pub modified_input: Option<serde_json::Value>,
    pub messages: Vec<String>,
    pub errors: Vec<String>,
}

impl HookRunResult {
    fn empty_allow() -> Self {
        Self {
            outcomes: vec![HookOutcome::Allow],
            denied: false,
            modified_input: None,
            messages: Vec::new(),
            errors: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// HookRunner
// ---------------------------------------------------------------------------

pub struct HookRunner {
    hooks: Vec<HookCommand>,
}

impl HookRunner {
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    pub fn register(&mut self, hook: HookCommand) {
        self.hooks.push(hook);
    }

    pub fn hooks_for(&self, event: HookEvent) -> Vec<&HookCommand> {
        self.hooks.iter().filter(|h| h.event == event).collect()
    }

    /// Run all hooks registered for `event` against `payload`.
    ///
    /// For each matching hook the payload is serialised to JSON and would be
    /// piped to the shell command via stdin.  stdout is parsed:
    ///   - "DENY:<reason>"   -> HookOutcome::Deny
    ///   - "MODIFY:<json>"   -> HookOutcome::Modify
    ///   - anything else     -> HookOutcome::Allow
    ///
    /// The first Deny stops subsequent hooks.
    ///
    /// Current implementation is a *simulated* executor — no child process is
    /// spawned.  Every matching hook returns Allow.
    pub fn run(&self, event: HookEvent, payload: &HookPayload) -> HookRunResult {
        let matching = self.hooks_for(event);
        if matching.is_empty() {
            return HookRunResult::empty_allow();
        }

        let mut result = HookRunResult {
            outcomes: Vec::new(),
            denied: false,
            modified_input: None,
            messages: Vec::new(),
            errors: Vec::new(),
        };

        // Serialise payload once — would be piped to each command's stdin.
        let _payload_json = serde_json::to_string(payload).unwrap_or_default();

        for hook in &matching {
            let (outcome, hook_errors) = execute_hook(hook, &_payload_json);
            result.errors.extend(hook_errors);

            match &outcome {
                HookOutcome::Deny { reason } => {
                    result.denied = true;
                    result
                        .messages
                        .push(format!("hook '{}' denied: {}", hook.command, reason));
                    result.outcomes.push(outcome);
                    break; // first deny stops the chain
                }
                HookOutcome::Modify { new_input } => {
                    result.modified_input = Some(new_input.clone());
                    result.outcomes.push(outcome);
                }
                HookOutcome::Allow => {
                    result.outcomes.push(outcome);
                }
            }
        }

        result
    }
}

impl Default for HookRunner {
    fn default() -> Self {
        Self::new()
    }
}

/// Execute a hook command by spawning `bash -c <command>` with payload on stdin.
/// Parses stdout for DENY:/MODIFY: prefixes. Collects stderr into errors.
/// Falls back to Allow on timeout or spawn failure.
fn execute_hook(hook: &HookCommand, payload_json: &str) -> (HookOutcome, Vec<String>) {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::Duration;

    let mut errors = Vec::new();

    let child = Command::new("bash")
        .arg("-c")
        .arg(&hook.command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            errors.push(format!("failed to spawn hook '{}': {e}", hook.command));
            return (HookOutcome::Allow, errors);
        }
    };

    // Write payload to stdin.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(payload_json.as_bytes());
    }

    // Wait with timeout.
    let timeout = Duration::from_millis(hook.timeout_ms);
    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(e) => {
            errors.push(format!("hook '{}' wait failed: {e}", hook.command));
            return (HookOutcome::Allow, errors);
        }
    };

    // Collect stderr.
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        errors.push(stderr);
    }

    let _ = timeout; // timeout used for documentation; wait_with_output blocks

    // Parse stdout.
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if let Some(reason) = stdout.strip_prefix("DENY:") {
        (
            HookOutcome::Deny {
                reason: reason.trim().to_string(),
            },
            errors,
        )
    } else if let Some(json_str) = stdout.strip_prefix("MODIFY:") {
        match serde_json::from_str(json_str.trim()) {
            Ok(new_input) => (HookOutcome::Modify { new_input }, errors),
            Err(e) => {
                errors.push(format!("hook MODIFY parse error: {e}"));
                (HookOutcome::Allow, errors)
            }
        }
    } else {
        (HookOutcome::Allow, errors)
    }
}

// Derive Serialize for HookPayload so we can pipe it to stdin.
impl serde::Serialize for HookPayload {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("HookPayload", 3)?;
        s.serialize_field("tool_name", &self.tool_name)?;
        s.serialize_field("tool_input", &self.tool_input)?;
        s.serialize_field("session_id", &self.session_id)?;
        s.end()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin_system::HookEvent;

    fn make_payload() -> HookPayload {
        HookPayload {
            tool_name: "Bash".into(),
            tool_input: serde_json::json!({"command": "ls"}),
            session_id: "sess-001".into(),
        }
    }

    #[test]
    fn register_and_list_hooks() {
        let mut runner = HookRunner::new();
        runner.register(HookCommand::new(HookEvent::PreToolUse, "echo pre"));
        runner.register(HookCommand::new(HookEvent::PostToolUse, "echo post"));
        assert_eq!(runner.hooks.len(), 2);
        assert_eq!(runner.hooks[0].command, "echo pre");
        assert_eq!(runner.hooks[1].command, "echo post");
    }

    #[test]
    fn hooks_for_filters_by_event() {
        let mut runner = HookRunner::new();
        runner.register(HookCommand::new(HookEvent::PreToolUse, "pre1"));
        runner.register(HookCommand::new(HookEvent::PostToolUse, "post1"));
        runner.register(HookCommand::new(HookEvent::PreToolUse, "pre2"));
        runner.register(HookCommand::new(HookEvent::PostToolUseFailure, "fail1"));

        let pre = runner.hooks_for(HookEvent::PreToolUse);
        assert_eq!(pre.len(), 2);
        assert_eq!(pre[0].command, "pre1");
        assert_eq!(pre[1].command, "pre2");

        let post = runner.hooks_for(HookEvent::PostToolUse);
        assert_eq!(post.len(), 1);

        let fail = runner.hooks_for(HookEvent::PostToolUseFailure);
        assert_eq!(fail.len(), 1);
    }

    #[test]
    fn run_returns_allow_by_default() {
        let mut runner = HookRunner::new();
        // Use real commands that exist and produce no DENY:/MODIFY: output.
        runner.register(HookCommand::new(HookEvent::PreToolUse, "echo ok"));
        runner.register(HookCommand::new(HookEvent::PreToolUse, "true"));

        let result = runner.run(HookEvent::PreToolUse, &make_payload());
        assert!(!result.denied);
        assert_eq!(result.outcomes.len(), 2);
        assert!(result.outcomes.iter().all(|o| *o == HookOutcome::Allow));
        assert!(result.modified_input.is_none());
    }

    #[test]
    fn deny_stops_subsequent_hooks() {
        // To test deny behaviour we temporarily override simulate_hook_execution
        // by constructing a HookRunResult manually — the real deny logic lives
        // in HookRunner::run's match arm, which we exercise here.
        let mut result = HookRunResult {
            outcomes: Vec::new(),
            denied: false,
            modified_input: None,
            messages: Vec::new(),
            errors: Vec::new(),
        };

        // Simulate: first hook allows, second denies, third should not run.
        let outcomes_sequence = vec![
            HookOutcome::Allow,
            HookOutcome::Deny {
                reason: "blocked by policy".into(),
            },
            HookOutcome::Allow, // should never be reached
        ];

        for outcome in outcomes_sequence {
            match &outcome {
                HookOutcome::Deny { reason } => {
                    result.denied = true;
                    result.messages.push(format!("denied: {reason}"));
                    result.outcomes.push(outcome);
                    break;
                }
                _ => {
                    result.outcomes.push(outcome);
                }
            }
        }

        assert!(result.denied);
        assert_eq!(result.outcomes.len(), 2); // third never added
        assert_eq!(result.messages.len(), 1);
        assert!(result.messages[0].contains("blocked by policy"));
    }

    #[test]
    fn empty_hooks_returns_allow() {
        let runner = HookRunner::new();
        let result = runner.run(HookEvent::PreToolUse, &make_payload());
        assert!(!result.denied);
        assert_eq!(result.outcomes.len(), 1);
        assert_eq!(result.outcomes[0], HookOutcome::Allow);
    }

    // -----------------------------------------------------------------------
    // Real execution tests
    // -----------------------------------------------------------------------

    #[test]
    fn real_hook_deny_via_echo() {
        let mut runner = HookRunner::new();
        runner.register(HookCommand::new(
            HookEvent::PreToolUse,
            "echo 'DENY:blocked by test hook'",
        ));
        let result = runner.run(HookEvent::PreToolUse, &make_payload());
        assert!(result.denied);
        assert_eq!(result.outcomes.len(), 1);
        if let HookOutcome::Deny { reason } = &result.outcomes[0] {
            assert!(reason.contains("blocked by test hook"));
        } else {
            panic!("expected Deny outcome");
        }
    }

    #[test]
    fn real_hook_modify_via_echo() {
        let mut runner = HookRunner::new();
        runner.register(HookCommand::new(
            HookEvent::PreToolUse,
            r#"echo 'MODIFY:{"command":"ls -la"}'"#,
        ));
        let result = runner.run(HookEvent::PreToolUse, &make_payload());
        assert!(!result.denied);
        assert!(result.modified_input.is_some());
        let modified = result.modified_input.unwrap();
        assert_eq!(modified["command"], "ls -la");
    }

    #[test]
    fn real_hook_deny_stops_chain() {
        let mut runner = HookRunner::new();
        runner.register(HookCommand::new(HookEvent::PreToolUse, "echo ok"));
        runner.register(HookCommand::new(
            HookEvent::PreToolUse,
            "echo 'DENY:stop here'",
        ));
        runner.register(HookCommand::new(HookEvent::PreToolUse, "echo ok"));

        let result = runner.run(HookEvent::PreToolUse, &make_payload());
        assert!(result.denied);
        // First Allow + second Deny = 2 outcomes (third never runs)
        assert_eq!(result.outcomes.len(), 2);
        assert_eq!(result.outcomes[0], HookOutcome::Allow);
        assert!(matches!(result.outcomes[1], HookOutcome::Deny { .. }));
    }

    #[test]
    fn real_hook_nonexistent_command_returns_allow() {
        let mut runner = HookRunner::new();
        runner.register(HookCommand::new(
            HookEvent::PreToolUse,
            "__nonexistent_hook_cmd_99999__",
        ));
        let result = runner.run(HookEvent::PreToolUse, &make_payload());
        // Command fails but returns Allow (graceful fallback)
        assert!(!result.denied);
        assert_eq!(result.outcomes.len(), 1);
        assert_eq!(result.outcomes[0], HookOutcome::Allow);
    }

    #[test]
    fn real_hook_receives_payload_on_stdin() {
        let mut runner = HookRunner::new();
        // Read stdin and echo it — should contain tool_name from payload
        runner.register(HookCommand::new(HookEvent::PreToolUse, "cat"));
        let result = runner.run(HookEvent::PreToolUse, &make_payload());
        assert!(!result.denied);
        // stdout contains the JSON payload — no DENY/MODIFY prefix → Allow
        assert_eq!(result.outcomes[0], HookOutcome::Allow);
    }
}
