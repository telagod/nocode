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
            "properties": {
                "server": {
                    "type": "string",
                    "description": "MCP server name when calling the generic MCP dispatcher"
                },
                "tool_name": {
                    "type": "string",
                    "description": "Tool name on the MCP server"
                },
                "name": {
                    "type": "string",
                    "description": "Either the MCP tool name or a prefixed mcp:server:tool name"
                },
                "arguments": {
                    "type": "object",
                    "description": "Structured arguments passed through to the MCP tool",
                    "additionalProperties": true,
                    "properties": {}
                }
            },
            "additionalProperties": true
        })
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        // Route to McpManager for server+tool_name dispatch
        if let Some(server) = input["server"].as_str() {
            let tool_name = input
                .get("tool_name")
                .or_else(|| input.get("name"))
                .and_then(|v| v.as_str());
            if let Some(tool_name) = tool_name {
                let mgr = crate::mcp::manager::global_mcp_manager();
                let mut guard = mgr.lock().unwrap_or_else(|e| e.into_inner());
                let arguments = input.get("arguments").cloned().unwrap_or_else(|| json!({}));
                let entry = guard.get_entry(server);
                let has_entry = entry.is_some();
                let is_connected =
                    entry.is_some_and(|e| e.phase == crate::mcp::manager::McpPhase::Connected);
                if !has_entry {
                    return ToolOutput::error(format!("MCP server '{server}' not registered"));
                }
                if !is_connected {
                    return ToolOutput::error(format!("MCP server '{server}' is not connected"));
                }
                return match guard.call_tool(server, tool_name, &arguments) {
                    Ok(result) => match (result.is_error, result.structured_content) {
                        (true, Some(structured)) => {
                            ToolOutput::error_with_structured(result.content, structured)
                        }
                        (true, None) => ToolOutput::error(result.content),
                        (false, Some(structured)) => {
                            ToolOutput::success_with_structured(result.content, structured)
                        }
                        (false, None) => ToolOutput::success(result.content),
                    },
                    Err(e) => ToolOutput::error(e),
                };
            }
        }

        // Try GlobalToolRegistry if input has a prefixed name
        if let Some(prefixed) = input["name"].as_str()
            && prefixed.starts_with("mcp:")
        {
            let global = crate::tool::global_registry::global_tool_registry();
            let guard = global.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(output) = guard.execute(prefixed, input) {
                return output;
            }
        }

        ToolOutput::error(
            "Mcp tool requires 'server' and 'tool_name' (or 'name') parameters, \
             or use the mcp:server:tool prefix format via GlobalToolRegistry."
                .to_string(),
        )
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
    fn mcp_generic_requires_parameters() {
        let tool = McpTool;
        let result = tool.execute(&json!({}));
        assert!(result.is_error);
    }

    #[test]
    fn mcp_generic_rejects_unknown_server() {
        let tool = McpTool;
        let result = tool.execute(&json!({"server": "nonexistent_xyz", "tool_name": "test"}));
        assert!(result.is_error);
        assert!(result.content.contains("not registered"));
    }

    #[test]
    fn mcp_generic_schema_has_properties() {
        let tool = McpTool;
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"].is_object());
        assert!(schema["properties"]["arguments"].is_object());
    }
}
