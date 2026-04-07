//! Tool executor — validation → permission → execute → result pipeline.

use crate::message::ContentBlock;
use crate::tool::ToolRegistry;
use crate::tool::bash_validation;
use crate::tool::permission::PermissionMode;
use crate::tool::tool_validation::validate_tool_input;
use serde_json::Value;

/// Executes tool calls through the full pipeline:
/// 1. JSON Schema validation
/// 2. Permission check
/// 3. Bash command validation (for Bash tool)
/// 4. Execute
/// 5. Return ContentBlock::ToolResult
pub struct ToolExecutor<'a> {
    registry: &'a ToolRegistry,
    permission_mode: PermissionMode,
}

impl<'a> ToolExecutor<'a> {
    pub fn new(registry: &'a ToolRegistry) -> Self {
        Self {
            registry,
            permission_mode: PermissionMode::Auto,
        }
    }

    pub fn with_permission_mode(mut self, mode: PermissionMode) -> Self {
        self.permission_mode = mode;
        self
    }

    /// Execute a single tool_use block through the full pipeline.
    pub fn execute_tool_use(&self, id: &str, name: &str, input: &Value) -> ContentBlock {
        // 1. Lookup tool
        let Some(tool) = self.registry.get(name) else {
            return ContentBlock::tool_error(id, format!("Tool '{name}' not found"));
        };

        // 2. Validate input against schema
        let schema = tool.input_schema();
        if let Err(e) = validate_tool_input(input, &schema) {
            return ContentBlock::tool_error(id, format!("Validation error: {e}"));
        }

        // 3. Permission check
        if !self.check_permission(name, input) {
            return ContentBlock::tool_error(id, format!("Permission denied for tool '{name}'"));
        }

        // 4. Bash-specific validation
        if name == "Bash"
            && let Some(cmd) = input["command"].as_str()
            && let Err(e) = bash_validation::validate_bash_command(cmd)
        {
            return ContentBlock::tool_error(id, format!("Bash validation: {e}"));
        }

        // 5. Execute
        let output = tool.execute(input);
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
        match self.permission_mode {
            PermissionMode::Auto => true,
            PermissionMode::Ask => {
                // In Ask mode, read-only tools are auto-approved
                match name {
                    "Read" | "Glob" | "Grep" | "TaskGet" | "TaskList" | "TaskOutput"
                    | "MemoryList" | "MemorySearch" | "CronList" | "ToolSearch" => true,
                    "Bash" => {
                        let cmd = input["command"].as_str().unwrap_or("");
                        bash_validation::is_read_only_command(cmd)
                    }
                    _ => {
                        // TODO: prompt user via PermissionPrompter
                        true
                    }
                }
            }
            PermissionMode::Deny => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
