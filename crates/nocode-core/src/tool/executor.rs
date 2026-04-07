use crate::message::ContentBlock;
use crate::tool::{ToolOutput, ToolRegistry};
use serde_json::Value;

/// Executes tool calls from model responses, producing tool_result content blocks.
pub struct ToolExecutor<'a> {
    registry: &'a ToolRegistry,
}

impl<'a> ToolExecutor<'a> {
    pub fn new(registry: &'a ToolRegistry) -> Self {
        Self { registry }
    }

    /// Execute a single tool_use block and return a tool_result content block.
    pub fn execute_tool_use(
        &self,
        id: &str,
        name: &str,
        input: &Value,
    ) -> ContentBlock {
        let Some(tool) = self.registry.get(name) else {
            return ContentBlock::tool_error(id, format!("Tool '{name}' not found"));
        };

        let output = tool.execute(input);
        if output.is_error {
            ContentBlock::tool_error(id, output.content)
        } else {
            ContentBlock::tool_result(id, output.content)
        }
    }

    /// Execute all tool_use blocks from a response, returning tool_result blocks.
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
}
