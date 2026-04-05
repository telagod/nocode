use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

/// Minimal MCP client that communicates via JSON-RPC over stdio.
pub struct McpClient {
    child: Child,
    next_id: AtomicU64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone)]
pub struct McpToolResult {
    pub content: String,
    pub is_error: bool,
}

#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    id: u64,
    method: String,
    params: Value,
}

#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    #[allow(dead_code)]
    id: Option<u64>,
    result: Option<Value>,
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    #[allow(dead_code)]
    code: i64,
    message: String,
}

impl McpClient {
    /// Spawn an MCP server process and initialize the connection.
    pub fn spawn(command: &str, args: &[&str]) -> Result<Self, String> {
        let child = Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("failed to spawn MCP server: {e}"))?;

        let mut client = Self {
            child,
            next_id: AtomicU64::new(1),
        };

        // Send initialize request.
        let _init_response = client.request(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "nocode", "version": "0.2.0"}
            }),
        )?;

        // Send initialized notification.
        client.notify("notifications/initialized", json!({}))?;

        Ok(client)
    }

    /// List available tools from the MCP server.
    pub fn list_tools(&mut self) -> Result<Vec<McpTool>, String> {
        let response = self.request("tools/list", json!({}))?;
        let tools = response
            .get("tools")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| {
                        Some(McpTool {
                            name: v.get("name")?.as_str()?.to_string(),
                            description: v
                                .get("description")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                            input_schema: v
                                .get("inputSchema")
                                .cloned()
                                .unwrap_or(json!({"type": "object"})),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(tools)
    }

    /// Call a tool on the MCP server.
    pub fn call_tool(
        &mut self,
        name: &str,
        arguments: &HashMap<String, String>,
    ) -> Result<McpToolResult, String> {
        let args_value: Value = arguments
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect::<serde_json::Map<String, Value>>()
            .into();

        let response =
            self.request("tools/call", json!({"name": name, "arguments": args_value}))?;

        let content = response
            .get("content")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();

        let is_error = response
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        Ok(McpToolResult { content, is_error })
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            id,
            method: method.to_string(),
            params,
        };

        let stdin = self
            .child
            .stdin
            .as_mut()
            .ok_or("MCP server stdin unavailable")?;
        let payload =
            serde_json::to_string(&request).map_err(|e| format!("failed to serialize: {e}"))?;
        writeln!(stdin, "{payload}").map_err(|e| format!("failed to write to MCP server: {e}"))?;
        stdin
            .flush()
            .map_err(|e| format!("failed to flush MCP server stdin: {e}"))?;

        let stdout = self
            .child
            .stdout
            .as_mut()
            .ok_or("MCP server stdout unavailable")?;
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|e| format!("failed to read from MCP server: {e}"))?;

        let response: JsonRpcResponse =
            serde_json::from_str(&line).map_err(|e| format!("invalid JSON-RPC response: {e}"))?;

        if let Some(error) = response.error {
            return Err(format!("MCP error: {}", error.message));
        }

        response
            .result
            .ok_or_else(|| String::from("MCP response missing result"))
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        let notification = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });

        let stdin = self
            .child
            .stdin
            .as_mut()
            .ok_or("MCP server stdin unavailable")?;
        let payload = serde_json::to_string(&notification)
            .map_err(|e| format!("failed to serialize: {e}"))?;
        writeln!(stdin, "{payload}").map_err(|e| format!("failed to write notification: {e}"))?;
        stdin
            .flush()
            .map_err(|e| format!("failed to flush notification: {e}"))?;
        Ok(())
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn mcp_tool_serialization_roundtrip() {
        let tool = McpTool {
            name: String::from("read_file"),
            description: String::from("Read a file from disk"),
            input_schema: json!({"type": "object", "properties": {"path": {"type": "string"}}}),
        };
        let serialized = serde_json::to_string(&tool).expect("serialize");
        let deserialized: McpTool = serde_json::from_str(&serialized).expect("deserialize");
        assert_eq!(deserialized.name, "read_file");
        assert_eq!(deserialized.description, "Read a file from disk");
        assert_eq!(
            deserialized.input_schema["properties"]["path"]["type"],
            "string"
        );
    }

    #[test]
    fn mcp_tool_result_fields() {
        let result = McpToolResult {
            content: String::from("file contents here"),
            is_error: false,
        };
        assert_eq!(result.content, "file contents here");
        assert!(!result.is_error);

        let err_result = McpToolResult {
            content: String::from("not found"),
            is_error: true,
        };
        assert!(err_result.is_error);
    }

    #[test]
    fn json_rpc_request_serializes_correctly() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            id: 42,
            method: String::from("tools/list"),
            params: json!({}),
        };
        let serialized = serde_json::to_value(&request).expect("serialize");
        assert_eq!(serialized["jsonrpc"], "2.0");
        assert_eq!(serialized["id"], 42);
        assert_eq!(serialized["method"], "tools/list");
    }

    #[test]
    fn json_rpc_response_parses_result() {
        let raw = r#"{"id": 1, "result": {"tools": []}, "error": null}"#;
        let response: JsonRpcResponse = serde_json::from_str(raw).expect("parse");
        assert_eq!(response.id, Some(1));
        assert!(response.result.is_some());
        assert!(response.error.is_none());
    }

    #[test]
    fn json_rpc_response_parses_error() {
        let raw = r#"{"id": 2, "result": null, "error": {"code": -32601, "message": "method not found"}}"#;
        let response: JsonRpcResponse = serde_json::from_str(raw).expect("parse");
        assert_eq!(response.id, Some(2));
        assert!(response.result.is_none());
        let err = response.error.expect("should have error");
        assert_eq!(err.code, -32601);
        assert_eq!(err.message, "method not found");
    }

    #[test]
    fn spawn_fails_with_nonexistent_command() {
        let result = McpClient::spawn("__nonexistent_mcp_binary_12345__", &[]);
        match result {
            Err(msg) => assert!(msg.contains("failed to spawn"), "unexpected error: {msg}"),
            Ok(_) => panic!("expected spawn to fail for nonexistent binary"),
        }
    }
}
