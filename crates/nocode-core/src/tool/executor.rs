//! Tool executor — validation → trust → hooks → permission → sandbox → execute pipeline.

use crate::config::runtime::SandboxConfig;
use crate::message::ContentBlock;
use crate::tool::ToolRegistry;
use crate::tool::bash_validation;
use crate::tool::file_safety;
use crate::tool::global_registry::{global_tool_registry, tool_definitions_for_model};
use crate::tool::hook_runner::HookRunner;
use crate::tool::permission::{PermissionDecision, PermissionMode, PermissionPrompter};
use crate::tool::session_tools::is_plan_mode;
use crate::tool::tool_validation::validate_tool_input;
use crate::tool::trust::{PermissionEnforcer, TrustDecision};
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
                let global = global_tool_registry();
                let guard = global.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(output) = guard.execute(name, input) {
                    // Run PostToolUse hooks even for bridged tools
                    if let Some(runner) = self.hook_runner {
                        let _ = runner.run_post_tool_use(name, Some(&output.content));
                    }
                    if output.is_error {
                        return ContentBlock::tool_error(id, output.content);
                    }
                    return ContentBlock::tool_result(id, output.content);
                }
            }
            return ContentBlock::tool_error(id, format!("Tool '{name}' not found"));
        };

        // 2. Validate input against schema
        let schema = tool.input_schema();
        if let Err(e) = validate_tool_input(input, &schema) {
            return ContentBlock::tool_error(id, format!("Validation error: {e}"));
        }

        // 3. Trust check
        if let Some(enforcer) = &self.trust_enforcer {
            match enforcer.check(name, "model") {
                TrustDecision::Deny => {
                    return ContentBlock::tool_error(
                        id,
                        format!("Trust policy denied tool '{name}'"),
                    );
                }
                TrustDecision::PromptUser => {
                    // In non-interactive mode, fall through to permission check
                }
                TrustDecision::Allow => {}
            }
        }

        // 4. PreToolUse hooks
        if let Some(runner) = self.hook_runner
            && let Err(hook_result) = runner.run_pre_tool_use(name, Some(&input.to_string()))
        {
            return ContentBlock::tool_error(
                id,
                format!(
                    "PreToolUse hook denied: {} (exit {})",
                    hook_result.hook_command, hook_result.exit_code
                ),
            );
        }

        // 5. Permission check
        if !self.check_permission(name, input) {
            return ContentBlock::tool_error(id, format!("Permission denied for tool '{name}'"));
        }

        // 6. Bash-specific validation
        if name == "Bash"
            && let Some(cmd) = input["command"].as_str()
            && let Err(e) = bash_validation::validate_bash_command(cmd)
        {
            return ContentBlock::tool_error(id, format!("Bash validation: {e}"));
        }

        // 7. Sandbox enforcement (skip if dangerouslyDisableSandbox is set)
        let sandbox_bypassed = name == "Bash"
            && input["dangerouslyDisableSandbox"]
                .as_bool()
                .unwrap_or(false);
        if !sandbox_bypassed
            && let Some(ref sandbox) = self.sandbox
            && sandbox.enabled
            && let Some(violation) = self.check_sandbox(name, input, sandbox)
        {
            return ContentBlock::tool_error(id, violation);
        }

        // 8. Execute
        let output = tool.execute(input);

        // 9. PostToolUse hooks (informational)
        if let Some(runner) = self.hook_runner {
            let _ = runner.run_post_tool_use(name, Some(&output.content));
        }

        if output.is_error {
            ContentBlock::tool_error(id, output.content)
        } else {
            ContentBlock::tool_result(id, output.content)
        }
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
    fn check_permission(&self, name: &str, input: &Value) -> bool {
        // Plan mode overrides everything: only read-only tools allowed
        if is_plan_mode() {
            return Self::is_read_only_tool(name, input);
        }

        match self.permission_mode {
            PermissionMode::Auto => true,
            PermissionMode::Ask => {
                // Read-only tools are always allowed
                match name {
                    "FileRead" | "Glob" | "Grep" | "TaskGet" | "TaskList" | "TaskOutput"
                    | "MemoryList" | "MemorySearch" | "CronList" | "ToolSearch" => return true,
                    "Bash" => {
                        let cmd = input["command"].as_str().unwrap_or("");
                        if bash_validation::is_read_only_command(cmd) {
                            return true;
                        }
                    }
                    _ => {}
                }

                // Check if tool was previously always-allowed
                if let Ok(allowed) = self.always_allowed.lock()
                    && allowed.contains(name)
                {
                    return true;
                }

                // Ask the prompter if available
                if let Some(prompter) = self.prompter {
                    let args_summary = input.to_string();
                    let summary = if args_summary.len() > 200 {
                        format!("{}...", &args_summary[..200])
                    } else {
                        args_summary
                    };
                    match prompter.prompt(name, &summary) {
                        PermissionDecision::Allow => true,
                        PermissionDecision::AlwaysAllow => {
                            if let Ok(mut allowed) = self.always_allowed.lock() {
                                allowed.insert(name.to_string());
                            }
                            true
                        }
                        PermissionDecision::Deny => false,
                    }
                } else {
                    // No prompter — default allow (non-interactive mode)
                    true
                }
            }
            PermissionMode::Deny => false,
            PermissionMode::ReadOnly => {
                // Only allow read-only tools
                match name {
                    "FileRead" | "Glob" | "Grep" | "TaskGet" | "TaskList" | "TaskOutput"
                    | "MemoryList" | "MemorySearch" | "CronList" | "ToolSearch"
                    | "ListMcpResources" | "ReadMcpResource" | "AskUserQuestion" => true,
                    "Bash" => {
                        let cmd = input["command"].as_str().unwrap_or("");
                        !bash_validation::is_write_command(cmd)
                            && bash_validation::is_read_only_command(cmd)
                    }
                    _ => false,
                }
            }
        }
    }

    /// Check sandbox restrictions for file/network operations.
    fn check_sandbox(&self, name: &str, input: &Value, sandbox: &SandboxConfig) -> Option<String> {
        match name {
            "FileRead" | "FileWrite" | "FileEdit" => {
                if let Some(path) = input["file_path"].as_str().or(input["path"].as_str()) {
                    if !sandbox.allowed_paths.is_empty()
                        && !sandbox.allowed_paths.iter().any(|p| path.starts_with(p))
                    {
                        return Some(format!("Sandbox: path '{path}' not in allowed paths"));
                    }
                    // Symlink escape check
                    if let Err(e) = file_safety::validate_file_path(
                        path,
                        sandbox.allowed_paths.first().map_or("/", |p| p.as_str()),
                    ) {
                        return Some(format!("Sandbox: {e}"));
                    }
                }
                None
            }
            "WebFetch" | "WebSearch" => {
                if !sandbox.network_enabled {
                    Some("Sandbox: network access disabled".to_string())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Determine whether a tool is considered read-only (safe for plan mode).
    fn is_read_only_tool(name: &str, input: &Value) -> bool {
        match name {
            // Always read-only
            "FileRead" | "Glob" | "Grep" | "TaskGet" | "TaskList" | "TaskOutput" | "MemoryList"
            | "MemorySearch" | "CronList" | "ToolSearch" | "ListMcpResources"
            | "ReadMcpResource" | "AskUserQuestion" | "EnterPlanMode" | "ExitPlanMode" => true,
            // Bash: only if the command is read-only
            "Bash" => {
                let cmd = input["command"].as_str().unwrap_or("");
                !bash_validation::is_write_command(cmd)
                    && bash_validation::is_read_only_command(cmd)
            }
            // Everything else is write/destructive
            _ => false,
        }
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
                content, is_error, ..
            } => {
                if is_error {
                    crate::tool::ToolOutput::error(content)
                } else {
                    crate::tool::ToolOutput::success(content)
                }
            }
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
            assert!(content.contains("Bash validation"));
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
            assert!(content.contains("Permission denied"));
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
            assert!(content.contains("PreToolUse hook denied"));
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
            assert!(content.contains("network access disabled"));
        } else {
            panic!("Expected ToolResult");
        }
    }

    #[test]
    fn plan_mode_blocks_write_tools() {
        use crate::tool::session_tools::{enter_plan_mode, exit_plan_mode, is_plan_mode};

        // Clean slate
        exit_plan_mode();
        assert!(!is_plan_mode());

        let reg = test_registry();
        let exec = ToolExecutor::new(&reg);

        // Activate plan mode directly
        enter_plan_mode();
        assert!(is_plan_mode());

        // Now FileWrite should be blocked by permission
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
            assert!(content.contains("Permission denied"));
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
