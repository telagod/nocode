//! Minimal MCP client — JSON-RPC over stdio.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

/// MCP client that communicates via JSON-RPC over stdio.
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

/// An MCP resource descriptor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResource {
    pub uri: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub mime_type: String,
}

#[derive(Debug, Clone)]
pub struct McpToolResult {
    pub content: String,
    pub is_error: bool,
    pub structured_content: Option<Value>,
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

        let _init_response = client.request(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "nocode", "version": env!("CARGO_PKG_VERSION")}
            }),
        )?;

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
    pub fn call_tool(&mut self, name: &str, arguments: &Value) -> Result<McpToolResult, String> {
        let response = self.request(
            "tools/call",
            json!({"name": name, "arguments": arguments.clone()}),
        )?;

        let content = response
            .get("content")
            .and_then(Value::as_array)
            .map(|arr| {
                let text_parts: Vec<&str> = arr
                    .iter()
                    .filter_map(|item| item.get("text").and_then(Value::as_str))
                    .collect();
                if !text_parts.is_empty() {
                    text_parts.join("\n")
                } else {
                    serde_json::to_string_pretty(arr).unwrap_or_default()
                }
            })
            .unwrap_or_default();
        let structured_content = response.get("content").cloned();

        let is_error = response
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        Ok(McpToolResult {
            content,
            is_error,
            structured_content,
        })
    }

    /// Read a resource from the MCP server by URI.
    pub fn read_resource(&mut self, uri: &str) -> Result<String, String> {
        let response = self.request("resources/read", json!({"uri": uri}))?;
        let contents = response
            .get("contents")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        item.get("text")
                            .and_then(Value::as_str)
                            .or_else(|| item.get("blob").and_then(Value::as_str))
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        if contents.is_empty() {
            Err(format!("Resource '{uri}' returned empty content"))
        } else {
            Ok(contents)
        }
    }

    /// List available resources from the MCP server.
    pub fn list_resources(&mut self) -> Result<Vec<McpResource>, String> {
        let response = self.request("resources/list", json!({}))?;
        let resources = response
            .get("resources")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| {
                        Some(McpResource {
                            uri: v.get("uri")?.as_str()?.to_string(),
                            name: v
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                            description: v
                                .get("description")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                            mime_type: v
                                .get("mimeType")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(resources)
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
    use tempfile::tempdir;

    #[test]
    fn mcp_tool_serialization_roundtrip() {
        let tool = McpTool {
            name: String::from("read_file"),
            description: String::from("Read a file"),
            input_schema: json!({"type": "object", "properties": {"path": {"type": "string"}}}),
        };
        let s = serde_json::to_string(&tool).unwrap();
        let d: McpTool = serde_json::from_str(&s).unwrap();
        assert_eq!(d.name, "read_file");
    }

    #[test]
    fn json_rpc_request_serializes() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0",
            id: 42,
            method: String::from("tools/list"),
            params: json!({}),
        };
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["jsonrpc"], "2.0");
        assert_eq!(v["id"], 42);
    }

    #[test]
    fn json_rpc_response_parses_result() {
        let raw = r#"{"id": 1, "result": {"tools": []}, "error": null}"#;
        let resp: JsonRpcResponse = serde_json::from_str(raw).unwrap();
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
    }

    #[test]
    fn json_rpc_response_parses_error() {
        let raw = r#"{"id": 2, "result": null, "error": {"code": -32601, "message": "not found"}}"#;
        let resp: JsonRpcResponse = serde_json::from_str(raw).unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().message, "not found");
    }

    #[test]
    fn spawn_fails_nonexistent() {
        let result = McpClient::spawn("__nonexistent_mcp_12345__", &[]);
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(err.contains("failed to spawn"));
    }

    #[cfg(unix)]
    #[test]
    fn call_tool_preserves_nested_json_arguments() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().unwrap();
        let script_path = dir.path().join("mock_mcp.py");
        std::fs::write(
            &script_path,
            r#"#!/usr/bin/env python3
import json, sys
for line in sys.stdin:
    req = json.loads(line)
    method = req.get("method")
    rid = req.get("id")
    if method == "initialize":
        print(json.dumps({"jsonrpc":"2.0","id":rid,"result":{}}), flush=True)
    elif method == "notifications/initialized":
        continue
    elif method == "tools/list":
        print(json.dumps({"jsonrpc":"2.0","id":rid,"result":{"tools":[{"name":"echo_args","description":"Echo nested args","inputSchema":{"type":"object"}}]}}), flush=True)
    elif method == "tools/call":
        args = req.get("params", {}).get("arguments", {})
        print(json.dumps({"jsonrpc":"2.0","id":rid,"result":{"content":[{"type":"text","text":json.dumps(args, sort_keys=True)}],"isError":False}}), flush=True)
    else:
        print(json.dumps({"jsonrpc":"2.0","id":rid,"error":{"code":-32601,"message":"unknown"}}), flush=True)
"#,
        )
        .unwrap();
        let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o644);
        std::fs::set_permissions(&script_path, perms).unwrap();

        let mut client = McpClient::spawn("python3", &[script_path.to_str().unwrap()]).unwrap();
        let tools = client.list_tools().unwrap();
        assert_eq!(tools.len(), 1);

        let nested_args = json!({
            "title": "issue",
            "labels": ["bug", "p1"],
            "meta": {"priority": 1, "urgent": true}
        });
        let result = client.call_tool("echo_args", &nested_args).unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains(r#""labels": ["bug", "p1"]"#));
        assert!(
            result
                .content
                .contains(r#""meta": {"priority": 1, "urgent": true}"#)
        );
        assert!(
            result
                .structured_content
                .as_ref()
                .is_some_and(Value::is_array)
        );
    }

    #[cfg(unix)]
    #[test]
    fn call_tool_preserves_non_text_structured_content() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().unwrap();
        let script_path = dir.path().join("mock_mcp_structured.py");
        std::fs::write(
            &script_path,
            r#"#!/usr/bin/env python3
import json, sys
for line in sys.stdin:
    req = json.loads(line)
    method = req.get("method")
    rid = req.get("id")
    if method == "initialize":
        print(json.dumps({"jsonrpc":"2.0","id":rid,"result":{}}), flush=True)
    elif method == "notifications/initialized":
        continue
    elif method == "tools/list":
        print(json.dumps({"jsonrpc":"2.0","id":rid,"result":{"tools":[{"name":"structured","description":"Structured","inputSchema":{"type":"object"}}]}}), flush=True)
    elif method == "tools/call":
        print(json.dumps({"jsonrpc":"2.0","id":rid,"result":{"content":[{"type":"resource","uri":"file:///tmp/demo.json","mimeType":"application/json"}],"isError":False}}), flush=True)
"#,
        )
        .unwrap();
        let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o644);
        std::fs::set_permissions(&script_path, perms).unwrap();

        let mut client = McpClient::spawn("python3", &[script_path.to_str().unwrap()]).unwrap();
        let result = client.call_tool("structured", &json!({})).unwrap();
        assert!(result.content.contains("\"resource\""));
        assert!(
            result
                .structured_content
                .as_ref()
                .is_some_and(Value::is_array)
        );
    }
}
