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
