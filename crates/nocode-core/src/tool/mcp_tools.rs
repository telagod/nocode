//! MCP tools — ListMcpResources, ReadMcpResource, Mcp (generic call).

use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// ListMcpResources
// ---------------------------------------------------------------------------

pub struct ListMcpResourcesTool;

impl Tool for ListMcpResourcesTool {
    fn name(&self) -> &str {
        "ListMcpResources"
    }
    fn description(&self) -> &str {
        "List available MCP resources, optionally filtered by server name."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "server": { "type": "string", "description": "Optional server name to filter resources by" }
            }
        })
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        let server_filter = input["server"].as_str();
        let mgr = crate::mcp::manager::global_mcp_manager();
        let mut guard = mgr.lock().unwrap();

        // List actual resources from connected servers
        let resources = guard.all_resources();
        let filtered: Vec<Value> = resources
            .iter()
            .filter(|(srv, _)| server_filter.is_none_or(|f| srv == f))
            .map(|(srv, r)| {
                json!({
                    "server": srv,
                    "uri": r.uri,
                    "name": r.name,
                    "description": r.description,
                    "mimeType": r.mime_type,
                })
            })
            .collect();

        // If no resources found, fall back to listing tools as resource-like entries
        if filtered.is_empty() {
            let tools = guard.all_tools();
            let tool_list: Vec<Value> = tools
                .iter()
                .filter(|(srv, _)| server_filter.is_none_or(|f| *srv == f))
                .map(|(srv, t)| {
                    json!({
                        "server": srv,
                        "type": "tool",
                        "name": t.name,
                        "description": t.description,
                    })
                })
                .collect();
            return ToolOutput::success(serde_json::to_string(&tool_list).unwrap_or_default());
        }

        ToolOutput::success(serde_json::to_string(&filtered).unwrap_or_default())
    }
}

// ---------------------------------------------------------------------------
// ReadMcpResource
// ---------------------------------------------------------------------------

pub struct ReadMcpResourceTool;

impl Tool for ReadMcpResourceTool {
    fn name(&self) -> &str {
        "ReadMcpResource"
    }
    fn description(&self) -> &str {
        "Read a specific resource from an MCP server by URI."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "server": { "type": "string", "description": "The MCP server name" },
                "uri": { "type": "string", "description": "The resource URI to read" }
            },
            "required": ["server", "uri"]
        })
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(server) = input["server"].as_str() else {
            return ToolOutput::error("Missing required parameter: server");
        };
        let Some(uri) = input["uri"].as_str() else {
            return ToolOutput::error("Missing required parameter: uri");
        };
        let mgr = crate::mcp::manager::global_mcp_manager();
        let mut guard = mgr.lock().unwrap();
        match guard.read_resource(server, uri) {
            Ok(content) => ToolOutput::success(content),
            Err(e) => ToolOutput::error(e),
        }
    }
}

// ---------------------------------------------------------------------------
// Mcp (generic MCP tool call)
// ---------------------------------------------------------------------------

pub struct McpTool;

impl Tool for McpTool {
    fn name(&self) -> &str {
        "Mcp"
    }
    fn description(&self) -> &str {
        "Execute a tool on an MCP server. Tool calls are routed via mcp:server:tool prefix."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "additionalProperties": true
        })
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        // Generic MCP tool dispatch — the GlobalToolRegistry handles mcp: prefix routing
        ToolOutput::error(format!(
            "Direct Mcp tool calls should be routed through GlobalToolRegistry with mcp:server:tool prefix. Input: {}",
            serde_json::to_string(input).unwrap_or_default()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn list_mcp_resources_succeeds() {
        let tool = ListMcpResourcesTool;
        let result = tool.execute(&json!({}));
        assert!(!result.is_error);
    }

    #[test]
    fn read_mcp_resource_missing_server() {
        let tool = ReadMcpResourceTool;
        assert!(tool.execute(&json!({"uri": "test://x"})).is_error);
    }

    #[test]
    fn read_mcp_resource_missing_uri() {
        let tool = ReadMcpResourceTool;
        assert!(tool.execute(&json!({"server": "test"})).is_error);
    }

    #[test]
    fn read_mcp_resource_nonexistent_server() {
        let tool = ReadMcpResourceTool;
        let result = tool.execute(&json!({"server": "nonexistent_xyz", "uri": "test://x"}));
        assert!(result.is_error);
    }

    #[test]
    fn mcp_generic_returns_error() {
        let tool = McpTool;
        let result = tool.execute(&json!({"name": "test"}));
        assert!(result.is_error);
    }
}
