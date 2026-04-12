//! MCP tool bridge — routes tool calls to MCP servers.

use crate::mcp::client::{McpClient, McpTool, McpToolResult};
use serde_json::Value;
use std::sync::Mutex;

/// Bridges MCP server tools into the native Tool interface.
/// Tool names are prefixed with `mcp:{server}:{tool}` for dispatch.
pub struct McpBridge {
    server_name: String,
    client: Mutex<McpClient>,
    tools: Vec<McpTool>,
}

impl McpBridge {
    /// Connect to an MCP server and discover its tools.
    pub fn connect(server_name: &str, command: &str, args: &[&str]) -> Result<Self, String> {
        let mut client = McpClient::spawn(command, args)?;
        let tools = client.list_tools()?;
        Ok(Self {
            server_name: server_name.to_string(),
            client: Mutex::new(client),
            tools,
        })
    }

    /// Get the discovered tools with prefixed names.
    pub fn tool_definitions(&self) -> Vec<McpBridgedTool> {
        self.tools
            .iter()
            .map(|t| McpBridgedTool {
                prefixed_name: format!("mcp:{}:{}", self.server_name, t.name),
                original_name: t.name.clone(),
                description: t.description.clone(),
                input_schema: t.input_schema.clone(),
                server_name: self.server_name.clone(),
            })
            .collect()
    }

    /// Call a tool by its original (unprefixed) name.
    pub fn call_tool(&self, name: &str, arguments: &Value) -> Result<McpToolResult, String> {
        let mut client = self.client.lock().map_err(|e| format!("lock error: {e}"))?;
        client.call_tool(name, arguments)
    }
}

/// A single MCP tool bridged into the native tool system.
#[derive(Debug, Clone)]
pub struct McpBridgedTool {
    pub prefixed_name: String,
    pub original_name: String,
    pub description: String,
    pub input_schema: Value,
    pub server_name: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bridged_tool_name_format() {
        let tool = McpBridgedTool {
            prefixed_name: "mcp:github:create_issue".to_string(),
            original_name: "create_issue".to_string(),
            description: "Create a GitHub issue".to_string(),
            input_schema: json!({"type": "object"}),
            server_name: "github".to_string(),
        };
        assert!(tool.prefixed_name.starts_with("mcp:"));
        assert_eq!(tool.original_name, "create_issue");
    }
}
