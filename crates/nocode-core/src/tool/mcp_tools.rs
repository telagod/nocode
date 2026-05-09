//! Unified MCP tool — call tools, list resources, read resources via `action`.

use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};

pub struct McpTool;

impl McpTool {
    /// Execute the "call" action — invoke an MCP tool on a server.
    fn execute_call(input: &Value) -> ToolOutput {
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
            "Mcp 'call' action requires 'server' and 'tool_name' (or 'name') parameters, \
             or use the mcp:server:tool prefix format via GlobalToolRegistry.",
        )
    }

    /// Execute the "list_resources" action — list available MCP resources.
    fn execute_list_resources(input: &Value) -> ToolOutput {
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

    /// Execute the "read_resource" action — read a specific MCP resource by URI.
    fn execute_read_resource(input: &Value) -> ToolOutput {
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

impl Tool for McpTool {
    fn name(&self) -> &str {
        "Mcp"
    }
    fn description(&self) -> &str {
        "Unified MCP interface. Use action 'call' to invoke a tool on an MCP server, \
         'list_resources' to list available resources, or 'read_resource' to read a specific resource."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["call", "list_resources", "read_resource"],
                    "description": "The MCP operation. 'call' invokes a tool, 'list_resources' shows available resources, 'read_resource' reads a resource."
                },
                "server": {
                    "type": "string",
                    "description": "MCP server name (required for 'call' and 'read_resource', optional filter for 'list_resources')"
                },
                "tool_name": {
                    "type": "string",
                    "description": "Tool name on the MCP server (for 'call' action)"
                },
                "name": {
                    "type": "string",
                    "description": "Either the MCP tool name or a prefixed mcp:server:tool name (for 'call' action)"
                },
                "arguments": {
                    "type": "object",
                    "description": "Structured arguments passed through to the MCP tool (for 'call' action)",
                    "additionalProperties": true,
                    "properties": {}
                },
                "uri": {
                    "type": "string",
                    "description": "The resource URI to read (for 'read_resource' action)"
                }
            },
            "required": ["action"],
            "additionalProperties": true
        })
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        let action = input["action"].as_str().unwrap_or("call");
        match action {
            "call" => Self::execute_call(input),
            "list_resources" => Self::execute_list_resources(input),
            "read_resource" => Self::execute_read_resource(input),
            other => ToolOutput::error(format!(
                "Unknown Mcp action '{other}'. Use 'call', 'list_resources', or 'read_resource'."
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn list_resources_succeeds() {
        let tool = McpTool;
        let result = tool.execute(&json!({"action": "list_resources"}));
        assert!(!result.is_error);
    }

    #[test]
    fn read_resource_missing_server() {
        let tool = McpTool;
        assert!(
            tool.execute(&json!({"action": "read_resource", "uri": "test://x"}))
                .is_error
        );
    }

    #[test]
    fn read_resource_missing_uri() {
        let tool = McpTool;
        assert!(
            tool.execute(&json!({"action": "read_resource", "server": "test"}))
                .is_error
        );
    }

    #[test]
    fn read_resource_nonexistent_server() {
        let tool = McpTool;
        let result = tool.execute(
            &json!({"action": "read_resource", "server": "nonexistent_xyz", "uri": "test://x"}),
        );
        assert!(result.is_error);
    }

    #[test]
    fn call_requires_parameters() {
        let tool = McpTool;
        let result = tool.execute(&json!({"action": "call"}));
        assert!(result.is_error);
    }

    #[test]
    fn call_rejects_unknown_server() {
        let tool = McpTool;
        let result = tool
            .execute(&json!({"action": "call", "server": "nonexistent_xyz", "tool_name": "test"}));
        assert!(result.is_error);
        assert!(result.content.contains("not registered"));
    }

    #[test]
    fn schema_has_action_and_properties() {
        let tool = McpTool;
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"].is_object());
        assert!(schema["properties"]["action"].is_object());
        assert!(schema["properties"]["arguments"].is_object());
        assert!(schema["properties"]["uri"].is_object());
        assert_eq!(schema["required"], json!(["action"]));
    }

    #[test]
    fn unknown_action_errors() {
        let tool = McpTool;
        let result = tool.execute(&json!({"action": "bogus"}));
        assert!(result.is_error);
        assert!(result.content.contains("Unknown Mcp action"));
    }

    #[test]
    fn default_action_is_call() {
        // When action is missing, defaults to "call"
        let tool = McpTool;
        let result = tool.execute(&json!({}));
        assert!(result.is_error);
        // Should be the call action's error, not "unknown action"
        assert!(result.content.contains("'call' action requires"));
    }
}
