use std::collections::HashMap;

use super::model::{
    ToolCallInput, ToolCallOutput, ToolCallResult, ToolExecutionTrace, ToolPermissionDecision,
    ToolProgressUpdate,
};
use crate::message::QueryMessage;

/// Metadata for a single MCP tool discovered from a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpToolInfo {
    pub server_name: String,
    pub tool_name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// Registry that maps discovered MCP tools to their originating servers.
///
/// Tools are addressed with the prefix format `mcp:<server>:<tool>`.
#[derive(Debug, Clone, Default)]
pub struct McpToolBridge {
    discovered_tools: HashMap<String, Vec<McpToolInfo>>,
}

impl McpToolBridge {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a batch of tools discovered from a specific MCP server.
    pub fn register_tools(&mut self, server_name: &str, tools: Vec<McpToolInfo>) {
        self.discovered_tools
            .entry(server_name.to_string())
            .or_default()
            .extend(tools);
    }

    /// Look up a tool by its prefixed name (`mcp:<server>:<tool>`).
    ///
    /// Returns `None` if the prefix is malformed or the tool is not registered.
    pub fn find_tool(&self, prefixed_name: &str) -> Option<&McpToolInfo> {
        let (server, tool) = parse_mcp_prefix(prefixed_name)?;
        self.discovered_tools
            .get(server)?
            .iter()
            .find(|t| t.tool_name == tool)
    }

    /// Return references to every registered tool across all servers.
    pub fn list_tools(&self) -> Vec<&McpToolInfo> {
        self.discovered_tools
            .values()
            .flat_map(|tools| tools.iter())
            .collect()
    }
}

/// Parse `mcp:<server>:<tool>` into `(server, tool)`.
fn parse_mcp_prefix(name: &str) -> Option<(&str, &str)> {
    let rest = name.strip_prefix("mcp:")?;
    let colon = rest.find(':')?;
    let server = &rest[..colon];
    let tool = &rest[colon + 1..];
    if server.is_empty() || tool.is_empty() {
        return None;
    }
    Some((server, tool))
}

/// Execute an MCP tool call through the bridge registry.
///
/// If the tool is found, returns a simulated success response (real `McpClient`
/// integration is deferred to a later milestone). If not found, returns `Failed`.
pub fn execute_mcp_tool_bridged(
    call: ToolCallInput,
    bridge: &McpToolBridge,
) -> ToolExecutionTrace {
    let tool_name = call.tool_name.clone();
    let progress = ToolProgressUpdate::new(
        call.tool_use_id.clone(),
        format!("MCP bridge: {tool_name}"),
    );

    match bridge.find_tool(&tool_name) {
        Some(info) => {
            let server = &info.server_name;
            let name = &info.tool_name;
            let msg = format!(
                "MCP tool '{name}' found on server '{server}' (execution pending real MCP client connection)"
            );
            ToolExecutionTrace {
                progress_updates: vec![progress],
                result: ToolPermissionDecision::allow(false).settle(
                    call.clone(),
                    ToolCallOutput {
                        summary: msg.clone(),
                        generated_messages: vec![QueryMessage::assistant(format!(
                            "tool-message: {msg}"
                        ))],
                        context_label: Some(call.context_label.clone()),
                        progress_updates: vec![ToolProgressUpdate::new(
                            call.tool_use_id,
                            format!("MCP complete: {name}"),
                        )],
                    },
                ),
                permission_denial: None,
            }
        }
        None => {
            let servers: Vec<&String> = bridge
                .discovered_tools
                .keys()
                .collect();
            let server_list = if servers.is_empty() {
                String::from("(none)")
            } else {
                servers.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
            };
            ToolExecutionTrace {
                progress_updates: vec![progress],
                result: ToolCallResult::failed(
                    call,
                    format!("MCP tool not found: {tool_name}. Available servers: {server_list}"),
                ),
                permission_denial: None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_tool(server: &str, tool: &str) -> McpToolInfo {
        McpToolInfo {
            server_name: server.to_string(),
            tool_name: tool.to_string(),
            description: format!("{tool} from {server}"),
            input_schema: json!({"type": "object"}),
        }
    }

    #[test]
    fn register_and_find_tool() {
        let mut bridge = McpToolBridge::new();
        bridge.register_tools("fs", vec![sample_tool("fs", "read_file")]);

        let found = bridge.find_tool("mcp:fs:read_file");
        assert!(found.is_some());
        let info = found.unwrap();
        assert_eq!(info.server_name, "fs");
        assert_eq!(info.tool_name, "read_file");
    }

    #[test]
    fn unknown_tool_returns_none() {
        let bridge = McpToolBridge::new();
        assert!(bridge.find_tool("mcp:fs:read_file").is_none());

        let mut bridge = McpToolBridge::new();
        bridge.register_tools("fs", vec![sample_tool("fs", "read_file")]);
        assert!(bridge.find_tool("mcp:fs:write_file").is_none());
        assert!(bridge.find_tool("mcp:other:read_file").is_none());
    }

    #[test]
    fn execute_bridged_found_tool() {
        let mut bridge = McpToolBridge::new();
        bridge.register_tools("fs", vec![sample_tool("fs", "read_file")]);

        let call = ToolCallInput::new("mcp:fs:read_file", "toolu-mcp-1")
            .with_context_label("mcp-test");
        let trace = execute_mcp_tool_bridged(call, &bridge);

        assert_eq!(trace.result.status_label(), "completed");
        assert!(trace.result.message().contains("read_file"));
        assert!(trace.result.message().contains("fs"));
    }

    #[test]
    fn execute_bridged_missing_tool() {
        let bridge = McpToolBridge::new();
        let call = ToolCallInput::new("mcp:ghost:nope", "toolu-mcp-2")
            .with_context_label("mcp-test");
        let trace = execute_mcp_tool_bridged(call, &bridge);

        assert_eq!(trace.result.status_label(), "failed");
        assert!(trace.result.message().contains("MCP tool not found"));
    }

    #[test]
    fn parse_mcp_prefix_format() {
        assert_eq!(parse_mcp_prefix("mcp:fs:read"), Some(("fs", "read")));
        assert_eq!(
            parse_mcp_prefix("mcp:server:tool_name"),
            Some(("server", "tool_name"))
        );
        // Malformed inputs.
        assert_eq!(parse_mcp_prefix("fs:read"), None);
        assert_eq!(parse_mcp_prefix("mcp:"), None);
        assert_eq!(parse_mcp_prefix("mcp::tool"), None);
        assert_eq!(parse_mcp_prefix("mcp:server:"), None);
        assert_eq!(parse_mcp_prefix("plain"), None);
    }

    #[test]
    fn list_tools_returns_all() {
        let mut bridge = McpToolBridge::new();
        bridge.register_tools("a", vec![sample_tool("a", "t1"), sample_tool("a", "t2")]);
        bridge.register_tools("b", vec![sample_tool("b", "t3")]);

        let all = bridge.list_tools();
        assert_eq!(all.len(), 3);
    }
}
