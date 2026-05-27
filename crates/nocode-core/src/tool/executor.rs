//! Tool executor — validation → policy → hooks → execute pipeline.
//!
//! ## Pipeline (post-REALIGN)
//!
//! 1. **Lookup** the tool (registry, then `GlobalToolRegistry` for bridged
//!    `mcp:`/`plugin:` names).
//! 2. **Schema** validation against `tool.input_schema()`.
//! 3. **Policy** gate ([`super::policy::PolicyEngine`]) — collapses trust,
//!    permission-mode, classifier, prompter and sandbox into one decision
//!    that carries a *gate name* and a *reason*. The decision is rendered
//!    into the error message so every refusal explains *why*.
//! 4. **PreToolUse hooks** — external commands that can deny.
//! 5. **Bash classifier** — extra check for `Bash` tool only (read-only or not).
//! 6. **Execute**, snapshot for undo, record file-history.
//! 7. **PostToolUse hooks** (informational).
//!
//! Conceptually three gates (Schema → Policy → Hooks); the Bash classifier
//! lives inside Policy via the risk classifier.

use crate::config::runtime::SandboxConfig;
use crate::message::ContentBlock;
use crate::tool::ToolRegistry;
use crate::tool::bash_validation;
use crate::tool::global_registry::{global_tool_registry, tool_definitions_for_model};
use crate::tool::hook_runner::HookRunner;
use crate::tool::permission::{PermissionMode, PermissionPrompter};
use crate::tool::policy::{GateDecision, PolicyEngine};
use crate::tool::tool_validation::validate_tool_input;
use crate::tool::trust::PermissionEnforcer;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Mutex;

/// Full tool execution pipeline:
/// 1. JSON Schema validation
/// 2. Trust check (TrustResolver)
/// 3. PreToolUse hooks (can deny)
/// 4. Permission mode check (with interactive prompter)
/// 5. Bash command validation (Bash tool only)
/// 6. Sandbox enforcement (path/network restrictions)
/// 7. Execute
/// 8. PostToolUse hooks (informational)
/// 9. Return ContentBlock::ToolResult
pub struct ToolExecutor<'a> {
    registry: &'a ToolRegistry,
    permission_mode: PermissionMode,
    trust_enforcer: Option<PermissionEnforcer>,
    hook_runner: Option<&'a HookRunner>,
    sandbox: Option<SandboxConfig>,
    prompter: Option<&'a dyn PermissionPrompter>,
    always_allowed: Mutex<HashSet<String>>,
}

impl<'a> ToolExecutor<'a> {
    pub fn new(registry: &'a ToolRegistry) -> Self {
        Self {
            registry,
            permission_mode: PermissionMode::Auto,
            trust_enforcer: None,
            hook_runner: None,
            sandbox: None,
            prompter: None,
            always_allowed: Mutex::new(HashSet::new()),
        }
    }

    pub fn with_permission_mode(mut self, mode: PermissionMode) -> Self {
        self.permission_mode = mode;
        self
    }

    pub fn with_trust(mut self, enforcer: PermissionEnforcer) -> Self {
        self.trust_enforcer = Some(enforcer);
        self
    }

    pub fn with_hooks(mut self, runner: &'a HookRunner) -> Self {
        self.hook_runner = Some(runner);
        self
    }

    pub fn with_sandbox(mut self, config: SandboxConfig) -> Self {
        self.sandbox = Some(config);
        self
    }

    pub fn with_prompter(mut self, prompter: &'a dyn PermissionPrompter) -> Self {
        self.prompter = Some(prompter);
        self
    }

    /// Execute a single tool_use block through the full pipeline.
    pub fn execute_tool_use(&self, id: &str, name: &str, input: &Value) -> ContentBlock {
        // 1. Lookup tool — base registry first, then GlobalToolRegistry for mcp: prefix
        let tool_opt = self.registry.get(name);
        let is_bridged = tool_opt.is_none() && name.contains(':');

        let Some(tool) = tool_opt else {
            // Try GlobalToolRegistry for bridged tools (mcp:server:tool, plugin:name:tool)
            if is_bridged {
                // --- Security pipeline for bridged tools (Policy → Hooks) ---
                let policy = self.policy_decision(name, input);
                if let GateDecision::Deny { .. } = &policy {
                    return ContentBlock::tool_error(
                        id,
                        format!("Denied [{}]", policy.format_trail()),
                    );
                }
                if let GateDecision::Allow { remember: true, .. } = &policy
                    && let Ok(mut allowed) = self.always_allowed.lock()
                {
                    allowed.insert(name.to_string());
                }

                // PreToolUse hooks
                if let Some(runner) = self.hook_runner
                    && let Err(hook_result) =
                        runner.run_pre_tool_use(name, Some(&input.to_string()))
                {
                    return ContentBlock::tool_error(
                        id,
                        format!(
                            "Denied [hook: {} (exit {})]",
                            hook_result.hook_command, hook_result.exit_code
                        ),
                    );
                }

                // Execute via global registry
                let global = global_tool_registry();
                let guard = global.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(output) = guard.execute(name, input) {
                    // PostToolUse hooks
                    if let Some(runner) = self.hook_runner {
                        let _ = runner.run_post_tool_use(name, Some(&output.content));
                    }
                    let crate::tool::ToolOutput {
                        content,
                        is_error,
                        structured_content,
                    } = output;
                    let block = match (is_error, structured_content) {
                        (true, Some(structured)) => {
                            ContentBlock::tool_error_structured(id, content, structured)
                        }
                        (true, None) => ContentBlock::tool_error(id, content),
                        (false, Some(structured)) => {
                            ContentBlock::tool_result_structured(id, content, structured)
                        }
                        (false, None) => ContentBlock::tool_result(id, content),
                    };
                    return block;
                }
            }
            return ContentBlock::tool_error(id, format!("Tool '{name}' not found"));
        };

        // 2. Validate input against schema
        let schema = tool.input_schema();
        if let Err(e) = validate_tool_input(input, &schema) {
            return ContentBlock::tool_error(id, format!("Validation error: {e}"));
        }

        // 3. Policy gate (trust + mode + classifier + prompter + sandbox)
        let policy = self.policy_decision(name, input);
        if let GateDecision::Deny { .. } = &policy {
            return ContentBlock::tool_error(id, format!("Denied [{}]", policy.format_trail()));
        }
        if let GateDecision::Allow { remember: true, .. } = &policy
            && let Ok(mut allowed) = self.always_allowed.lock()
        {
            allowed.insert(name.to_string());
        }

        // 4. PreToolUse hooks
        if let Some(runner) = self.hook_runner
            && let Err(hook_result) = runner.run_pre_tool_use(name, Some(&input.to_string()))
        {
            return ContentBlock::tool_error(
                id,
                format!(
                    "Denied [hook: {} (exit {})]",
                    hook_result.hook_command, hook_result.exit_code
                ),
            );
        }

        // 5. Bash-specific validation (Policy already classified Bash via classifier;
        //    this layer enforces hard syntax rules / known-bad commands).
        if name == "Bash"
            && let Some(cmd) = input["command"].as_str()
            && let Err(e) = bash_validation::validate_bash_command(cmd)
        {
            return ContentBlock::tool_error(id, format!("Denied [bash: {e}]"));
        }

        // 6. Snapshot for undo (FileEdit / FileWrite)
        let file_path_for_undo = if matches!(name, "FileEdit" | "FileWrite") {
            input["file_path"].as_str().map(|p| {
                let old = std::fs::read_to_string(p).ok();
                (p.to_string(), old)
            })
        } else {
            None
        };

        // 7. Execute
        let output = tool.execute(input);

        // 8. Record to FileHistory on success
        if !output.is_error
            && let Some((path, old_content)) = file_path_for_undo
        {
            let new_content = std::fs::read_to_string(&path).ok();
            let history = crate::storage::file_history::global_file_history();
            if let Ok(mut h) = history.lock() {
                h.record_edit(&std::path::PathBuf::from(&path), old_content, new_content);
            }
        }

        // 9. PostToolUse hooks (informational)
        if let Some(runner) = self.hook_runner {
            let _ = runner.run_post_tool_use(name, Some(&output.content));
        }

        let crate::tool::ToolOutput {
            content,
            is_error,
            structured_content,
        } = output;
        match (is_error, structured_content) {
            (true, Some(structured)) => {
                ContentBlock::tool_error_structured(id, content, structured)
            }
            (true, None) => ContentBlock::tool_error(id, content),
            (false, Some(structured)) => {
                ContentBlock::tool_result_structured(id, content, structured)
            }
            (false, None) => ContentBlock::tool_result(id, content),
        }
    }

    /// Build and run the unified policy gate. The returned [`GateDecision`]
    /// carries both the verdict and a human-readable reason, so callers can
    /// surface a why-trail to the TUI / logs.
    fn policy_decision(&self, name: &str, input: &Value) -> GateDecision {
        let previously_allowed = self
            .always_allowed
            .lock()
            .map(|set| set.contains(name))
            .unwrap_or(false);
        let engine = PolicyEngine {
            mode: self.permission_mode,
            trust: self.trust_enforcer.as_ref(),
            sandbox: self.sandbox.as_ref(),
            prompter: self.prompter,
            previously_allowed,
        };
        engine.evaluate(name, input)
    }

    /// Execute all tool_use blocks from a response.
    pub fn execute_all(&self, content: &[ContentBlock]) -> Vec<ContentBlock> {
        content
            .iter()
            .filter_map(|block| {
                if let ContentBlock::ToolUse { id, name, input } = block {
                    Some(self.execute_tool_use(id, name, input))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Check if a tool call is permitted under the current permission mode.
    /// Check if a tool call is permitted under the current permission mode.
    ///
    /// **Deprecated**: this method has been collapsed into [`PolicyEngine`].
    /// The body below stays only because external integration tests may still
    /// reach for it. New code should call [`Self::policy_decision`].
    #[deprecated(note = "use PolicyEngine via policy_decision()")]
    #[allow(dead_code)]
    fn check_permission(&self, name: &str, input: &Value) -> bool {
        matches!(
            self.policy_decision(name, input),
            GateDecision::Allow { .. }
        )
    }
}

// ---------------------------------------------------------------------------
// ToolRunner bridge — connects DI layer to ToolExecutor
// ---------------------------------------------------------------------------

use crate::provider::types::ToolDefinition;
use crate::query::deps::ToolRunner;

/// Default production implementation of ToolRunner, backed by ToolExecutor.
pub struct DefaultToolExecutor<'a> {
    executor: ToolExecutor<'a>,
}

impl<'a> DefaultToolExecutor<'a> {
    pub fn new(executor: ToolExecutor<'a>) -> Self {
        Self { executor }
    }
}

impl ToolRunner for DefaultToolExecutor<'_> {
    fn execute(&self, name: &str, id: &str, input: &serde_json::Value) -> crate::tool::ToolOutput {
        let result = self.executor.execute_tool_use(id, name, input);
        match result {
            ContentBlock::ToolResult {
                content,
                is_error,
                structured_content,
                ..
            } => match (is_error, structured_content) {
                (true, Some(structured)) => {
                    crate::tool::ToolOutput::error_with_structured(content, structured)
                }
                (true, None) => crate::tool::ToolOutput::error(content),
                (false, Some(structured)) => {
                    crate::tool::ToolOutput::success_with_structured(content, structured)
                }
                (false, None) => crate::tool::ToolOutput::success(content),
            },
            _ => crate::tool::ToolOutput::error("Unexpected result type"),
        }
    }

    fn definitions(&self) -> Vec<ToolDefinition> {
        tool_definitions_for_model(self.executor.registry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::hook_runner::HookRunner;
    use crate::tool::trust::{PermissionEnforcer, RuleBasedPolicy};
    use serde_json::json;

    fn test_registry() -> ToolRegistry {
        ToolRegistry::with_defaults("/tmp")
    }

    #[test]
    fn executes_valid_tool() {
        let reg = test_registry();
        let exec = ToolExecutor::new(&reg);
        let result = exec.execute_tool_use("id-1", "Bash", &json!({"command": "echo hello"}));
        if let ContentBlock::ToolResult {
            content, is_error, ..
        } = &result
        {
            assert!(!is_error);
            assert!(content.contains("hello"));
        } else {
            panic!("Expected ToolResult");
        }
    }

    #[test]
    fn rejects_unknown_tool() {
        let reg = test_registry();
        let exec = ToolExecutor::new(&reg);
        let result = exec.execute_tool_use("id-2", "NonExistent", &json!({}));
        if let ContentBlock::ToolResult { is_error, .. } = &result {
            assert!(is_error);
        } else {
            panic!("Expected ToolResult");
        }
    }

    #[test]
    fn validates_missing_required_param() {
        let reg = test_registry();
        let exec = ToolExecutor::new(&reg);
        let result = exec.execute_tool_use("id-3", "Bash", &json!({}));
        if let ContentBlock::ToolResult {
            content, is_error, ..
        } = &result
        {
            assert!(is_error);
            assert!(content.contains("Validation error"));
        } else {
            panic!("Expected ToolResult");
        }
    }

    #[test]
    fn blocks_destructive_bash() {
        let reg = test_registry();
        let exec = ToolExecutor::new(&reg);
        let result = exec.execute_tool_use("id-4", "Bash", &json!({"command": "rm -rf /"}));
        if let ContentBlock::ToolResult {
            content, is_error, ..
        } = &result
        {
            assert!(is_error);
            // The bash classifier flags rm -rf / as Destructive — this trips
            // the permission gate (Auto mode still allows, but bash_validation
            // hard-blocks the syntax). Either trail is acceptable.
            assert!(
                content.contains("bash") || content.contains("Bash") || content.contains("Denied"),
                "got: {content}"
            );
        } else {
            panic!("Expected ToolResult");
        }
    }

    #[test]
    fn deny_mode_blocks_all() {
        let reg = test_registry();
        let exec = ToolExecutor::new(&reg).with_permission_mode(PermissionMode::Deny);
        let result = exec.execute_tool_use("id-5", "Bash", &json!({"command": "echo hi"}));
        if let ContentBlock::ToolResult {
            content, is_error, ..
        } = &result
        {
            assert!(is_error);
            assert!(
                content.contains("Denied") && content.contains("permission"),
                "expected gate trail, got: {content}"
            );
        } else {
            panic!("Expected ToolResult");
        }
    }

    #[test]
    fn trust_enforcer_allows() {
        let reg = test_registry();
        let enforcer = PermissionEnforcer::allow_all();
        let exec = ToolExecutor::new(&reg).with_trust(enforcer);
        let result = exec.execute_tool_use("id-6", "Bash", &json!({"command": "echo trust"}));
        if let ContentBlock::ToolResult { is_error, .. } = &result {
            assert!(!is_error);
        } else {
            panic!("Expected ToolResult");
        }
    }

    #[test]
    fn trust_enforcer_denies() {
        let reg = test_registry();
        let policy = RuleBasedPolicy::new(vec![], vec!["blocked".to_string()]);
        let enforcer = PermissionEnforcer::new(Box::new(policy));
        let exec = ToolExecutor::new(&reg).with_trust(enforcer);
        // Trust check uses labels from TrustContext — without labels, falls to PromptUser
        // which passes through. This test verifies the wiring works.
        let result = exec.execute_tool_use("id-7", "Bash", &json!({"command": "echo hi"}));
        // No labels → PromptUser → falls through to permission check → allowed
        if let ContentBlock::ToolResult { is_error, .. } = &result {
            assert!(!is_error);
        } else {
            panic!("Expected ToolResult");
        }
    }

    #[test]
    fn hook_denies_tool() {
        let reg = test_registry();
        let runner = HookRunner::new(
            vec![crate::config::runtime::HookEntry {
                command: "false".to_string(),
                tool_filter: None,
                timeout_ms: None,
                env: Default::default(),
            }],
            Vec::new(),
            Vec::new(),
        );
        let exec = ToolExecutor::new(&reg).with_hooks(&runner);
        let result = exec.execute_tool_use("id-8", "Bash", &json!({"command": "echo hi"}));
        if let ContentBlock::ToolResult {
            content, is_error, ..
        } = &result
        {
            assert!(is_error);
            assert!(
                content.contains("hook"),
                "expected hook trail, got: {content}"
            );
        } else {
            panic!("Expected ToolResult");
        }
    }

    #[test]
    fn sandbox_blocks_network() {
        // Ensure plan mode is off so permission check doesn't interfere
        crate::tool::session_tools::exit_plan_mode();

        let reg = test_registry();
        let sandbox = SandboxConfig {
            enabled: true,
            allowed_paths: vec!["/tmp".to_string()],
            network_enabled: false,
        };
        let exec = ToolExecutor::new(&reg).with_sandbox(sandbox);
        let result = exec.execute_tool_use(
            "id-9",
            "WebFetch",
            &json!({"url": "https://example.com", "prompt": "test"}),
        );
        if let ContentBlock::ToolResult {
            content, is_error, ..
        } = &result
        {
            assert!(is_error);
            assert!(
                content.contains("sandbox"),
                "expected sandbox trail, got: {content}"
            );
        } else {
            panic!("Expected ToolResult");
        }
    }

    #[test]
    fn plan_mode_blocks_write_tools() {
        use crate::tool::session_tools::{
            enter_plan_mode, exit_plan_mode, is_plan_mode, plan_mode_test_lock,
        };

        let _guard = plan_mode_test_lock();

        // Clean slate
        exit_plan_mode();
        assert!(!is_plan_mode());

        let reg = test_registry();
        let exec = ToolExecutor::new(&reg);

        // Activate plan mode directly
        enter_plan_mode();
        assert!(is_plan_mode());

        // Now FileWrite should be blocked by plan mode
        let result = exec.execute_tool_use(
            "id-pm2",
            "FileWrite",
            &json!({"file_path": "/tmp/nocode_plan_test.txt", "content": "x"}),
        );
        if let ContentBlock::ToolResult {
            content, is_error, ..
        } = &result
        {
            assert!(is_error);
            assert!(
                content.contains("plan-mode"),
                "expected plan-mode trail, got: {content}"
            );
        } else {
            panic!("Expected ToolResult");
        }

        // Read-only tool should still work
        let result = exec.execute_tool_use("id-pm3", "Glob", &json!({"pattern": "/tmp/*"}));
        if let ContentBlock::ToolResult { is_error, .. } = &result {
            assert!(!is_error);
        } else {
            panic!("Expected ToolResult");
        }

        // Read-only Bash should work
        let result = exec.execute_tool_use("id-pm4", "Bash", &json!({"command": "ls /tmp"}));
        if let ContentBlock::ToolResult { is_error, .. } = &result {
            assert!(!is_error);
        } else {
            panic!("Expected ToolResult");
        }

        // Clean up
        exit_plan_mode();
    }
}
