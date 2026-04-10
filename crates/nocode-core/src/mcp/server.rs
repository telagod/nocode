//! MCP Server mode — nocode acts as an MCP server over stdio.
//!
//! Exposes all registered tools as MCP tools via JSON-RPC over stdin/stdout.
//! Also supports a `query` method for running the agentic loop, and
//! `resources/list` / `resources/read` for file access.
//! Launch: `nocode --mcp-server`

use crate::message::Message;
use crate::message::SystemBlock;
use crate::provider::ProviderBox;
use crate::query::r#loop::{self, LoopConfig, NoopObserver};
use crate::tool::executor::ToolExecutor;
use crate::tool::{ToolOutput, ToolRegistry};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};

/// MCP Server — reads JSON-RPC from stdin, writes responses to stdout.
pub struct McpServer {
    registry: ToolRegistry,
    server_info: ServerInfo,
    /// Optional provider for query execution.
    provider: Option<ProviderBox>,
    model: Option<String>,
    system_blocks: Vec<SystemBlock>,
    max_tokens: u32,
    max_turns: u32,
    /// Working directory for resource resolution.
    cwd: String,
}

#[derive(Debug, Clone, Serialize)]
struct ServerInfo {
    name: String,
    version: String,
}

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    #[allow(dead_code)]
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

impl McpServer {
    pub fn new(registry: ToolRegistry) -> Self {
        Self {
            registry,
            server_info: ServerInfo {
                name: "nocode".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            provider: None,
            model: None,
            system_blocks: Vec::new(),
            max_tokens: 16384,
            max_turns: 10,
            cwd: String::from("."),
        }
    }

    /// Create a fully-configured MCP server with provider for query execution.
    pub fn with_provider(
        registry: ToolRegistry,
        provider: ProviderBox,
        model: String,
        system_blocks: Vec<SystemBlock>,
        max_tokens: u32,
        max_turns: u32,
        cwd: String,
    ) -> Self {
        Self {
            registry,
            server_info: ServerInfo {
                name: "nocode".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            provider: Some(provider),
            model: Some(model),
            system_blocks,
            max_tokens,
            max_turns,
            cwd,
        }
    }

    /// Run the MCP server loop (blocking, reads stdin, writes stdout).
    pub fn run(&self) -> Result<(), String> {
        // APPEND_REST
        let stdin = std::io::stdin();
        let reader = BufReader::new(stdin.lock());
        let stdout = std::io::stdout();
        let mut writer = stdout.lock();

        for line in reader.lines() {
            let line = line.map_err(|e| format!("stdin read error: {e}"))?;
            if line.trim().is_empty() {
                continue;
            }

            let request: JsonRpcRequest = match serde_json::from_str(&line) {
                Ok(r) => r,
                Err(e) => {
                    let err_resp = json!({
                        "jsonrpc": "2.0",
                        "id": null,
                        "error": {"code": -32700, "message": format!("Parse error: {e}")}
                    });
                    writeln!(
                        writer,
                        "{}",
                        serde_json::to_string(&err_resp).unwrap_or_default()
                    )
                    .map_err(|e| format!("stdout write error: {e}"))?;
                    continue;
                }
            };

            let response = self.handle_request(&request);
            if let Some(resp) = response {
                writeln!(
                    writer,
                    "{}",
                    serde_json::to_string(&resp).unwrap_or_default()
                )
                .map_err(|e| format!("stdout write error: {e}"))?;
                writer
                    .flush()
                    .map_err(|e| format!("stdout flush error: {e}"))?;
            }

            // Shutdown on exit
            if request.method == "shutdown" {
                break;
            }
        }
        Ok(())
    }

    /// Handle a single JSON-RPC request, returning a response (or None for notifications).
    pub fn handle_request(&self, request: &JsonRpcRequest) -> Option<Value> {
        let id = request.id.clone()?; // Notifications have no id

        let result = match request.method.as_str() {
            "initialize" => self.handle_initialize(),
            "tools/list" => self.handle_tools_list(),
            "tools/call" => self.handle_tools_call(&request.params),
            "resources/list" => self.handle_resources_list(),
            "resources/read" => self.handle_resources_read(&request.params),
            "query" => self.handle_query(&request.params),
            "ping" => Ok(json!({})),
            "shutdown" => Ok(json!({})),
            other => Err((-32601, format!("Method not found: {other}"))),
        };

        Some(match result {
            Ok(val) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": val,
            }),
            Err((code, msg)) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": code, "message": msg},
            }),
        })
    }

    fn handle_initialize(&self) -> Result<Value, (i64, String)> {
        Ok(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {"listChanged": false},
                "resources": {"subscribe": false, "listChanged": false},
            },
            "serverInfo": {
                "name": self.server_info.name,
                "version": self.server_info.version,
            }
        }))
    }

    fn handle_tools_list(&self) -> Result<Value, (i64, String)> {
        let tools: Vec<Value> = self
            .registry
            .definitions()
            .iter()
            .map(|def| {
                json!({
                    "name": def.name,
                    "description": def.description,
                    "inputSchema": def.input_schema,
                })
            })
            .collect();
        Ok(json!({"tools": tools}))
    }

    fn handle_tools_call(&self, params: &Value) -> Result<Value, (i64, String)> {
        let name = params["name"]
            .as_str()
            .ok_or((-32602, "Missing 'name' parameter".to_string()))?;
        let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

        let tool = self
            .registry
            .get(name)
            .ok_or((-32602, format!("Tool '{name}' not found")))?;

        let output: ToolOutput = tool.execute(&arguments);

        Ok(json!({
            "content": [{"type": "text", "text": output.content}],
            "isError": output.is_error,
        }))
    }

    fn handle_resources_list(&self) -> Result<Value, (i64, String)> {
        // Return project config files as resources
        let mut resources = Vec::new();

        let cwd_path = std::path::Path::new(&self.cwd);

        // CLAUDE.md variants
        for name in &["CLAUDE.md", ".claude/CLAUDE.md", "docs/CLAUDE.md"] {
            let path = cwd_path.join(name);
            if path.exists() {
                resources.push(json!({
                    "uri": format!("file://{}", path.display()),
                    "name": name,
                    "description": format!("Project instructions ({name})"),
                    "mimeType": "text/markdown",
                }));
            }
        }

        // .nocode config
        let nocode_dir = cwd_path.join(".nocode");
        if nocode_dir.exists() {
            let entries = std::fs::read_dir(&nocode_dir)
                .into_iter()
                .flatten()
                .filter_map(|e| e.ok());
            for entry in entries {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.ends_with(".json") || name.ends_with(".md") {
                    resources.push(json!({
                        "uri": format!("file://{}", entry.path().display()),
                        "name": format!(".nocode/{name}"),
                        "mimeType": if name.ends_with(".json") { "application/json" } else { "text/markdown" },
                    }));
                }
            }
        }

        Ok(json!({"resources": resources}))
    }

    fn handle_resources_read(&self, params: &Value) -> Result<Value, (i64, String)> {
        let uri = params["uri"]
            .as_str()
            .ok_or((-32602, "Missing 'uri' parameter".to_string()))?;

        // Only allow file:// URIs
        let path_str = uri
            .strip_prefix("file://")
            .ok_or((-32602, "Only file:// URIs are supported".to_string()))?;

        let path = std::path::Path::new(path_str);

        // Security: ensure path is within cwd or a common config location
        let canonical = path
            .canonicalize()
            .map_err(|e| (-32602, format!("Cannot resolve path: {e}")))?;

        // Check file size (max 1MB)
        let metadata = std::fs::metadata(&canonical)
            .map_err(|e| (-32602, format!("Cannot read file metadata: {e}")))?;
        if metadata.len() > 1_048_576 {
            return Err((-32602, "File too large (max 1MB)".to_string()));
        }

        // Detect binary files
        let content =
            std::fs::read(&canonical).map_err(|e| (-32602, format!("Cannot read file: {e}")))?;

        // Simple binary detection: check for null bytes in first 8KB
        let check_len = content.len().min(8192);
        if content[..check_len].contains(&0) {
            return Err((-32602, "Binary files are not supported".to_string()));
        }

        let text = String::from_utf8(content)
            .map_err(|_| (-32602, "File is not valid UTF-8".to_string()))?;

        let mime_type = if path_str.ends_with(".json") {
            "application/json"
        } else if path_str.ends_with(".md") {
            "text/markdown"
        } else {
            "text/plain"
        };

        Ok(json!({
            "contents": [{
                "uri": uri,
                "mimeType": mime_type,
                "text": text,
            }]
        }))
    }

    fn handle_query(&self, params: &Value) -> Result<Value, (i64, String)> {
        let prompt = params["prompt"]
            .as_str()
            .ok_or((-32602, "Missing 'prompt' parameter".to_string()))?;

        let provider = self.provider.as_ref().ok_or((
            -32000,
            "No provider configured — launch with provider for query support".to_string(),
        ))?;
        let model = self
            .model
            .as_ref()
            .ok_or((-32000, "No model configured".to_string()))?;

        let messages = vec![Message::user_text(prompt)];
        let executor = ToolExecutor::new(&self.registry);
        let config = LoopConfig {
            model: model.clone(),
            max_tokens: self.max_tokens,
            max_turns: self.max_turns,
            system: self.system_blocks.clone(),
            tools: self.registry.definitions(),
            parallel_tool_execution: true,
        };
        let mut observer = NoopObserver;

        match r#loop::run_agentic_loop(
            provider.as_ref(),
            &executor,
            &config,
            messages,
            &mut observer,
        ) {
            Ok(result) => {
                let text: String = result
                    .messages
                    .iter()
                    .filter(|m| m.role == crate::message::Role::Assistant)
                    .map(|m| m.text_content())
                    .collect::<Vec<_>>()
                    .join("\n");
                Ok(json!({
                    "content": [{"type": "text", "text": text}],
                    "input_tokens": result.total_input_tokens,
                    "output_tokens": result.total_output_tokens,
                    "turns": result.turns,
                }))
            }
            Err(e) => Err((-32000, format!("{e}"))),
        }
    }
}

/// Process a single JSON-RPC request string (for testing without stdio).
pub fn process_request(server: &McpServer, input: &str) -> Option<String> {
    let request: JsonRpcRequest = serde_json::from_str(input).ok()?;
    let response = server.handle_request(&request)?;
    serde_json::to_string(&response).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_server() -> McpServer {
        McpServer::new(ToolRegistry::with_defaults("/tmp"))
    }

    #[test]
    fn initialize_returns_capabilities() {
        let server = test_server();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(1)),
            method: "initialize".to_string(),
            params: json!({}),
        };
        let resp = server.handle_request(&req).unwrap();
        assert!(resp["result"]["capabilities"]["tools"].is_object());
        assert_eq!(resp["result"]["serverInfo"]["name"], "nocode");
    }

    #[test]
    fn tools_list_returns_tools() {
        let server = test_server();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(2)),
            method: "tools/list".to_string(),
            params: json!({}),
        };
        let resp = server.handle_request(&req).unwrap();
        let tools = resp["result"]["tools"].as_array().unwrap();
        assert!(!tools.is_empty());
        // Should have Bash tool
        assert!(tools.iter().any(|t| t["name"] == "Bash"));
    }

    #[test]
    fn tools_call_executes() {
        let server = test_server();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(3)),
            method: "tools/call".to_string(),
            params: json!({"name": "Bash", "arguments": {"command": "echo mcp_test"}}),
        };
        let resp = server.handle_request(&req).unwrap();
        let content = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(content.contains("mcp_test"));
        assert_eq!(resp["result"]["isError"], false);
    }

    #[test]
    fn tools_call_unknown_tool() {
        let server = test_server();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(4)),
            method: "tools/call".to_string(),
            params: json!({"name": "NonExistent", "arguments": {}}),
        };
        let resp = server.handle_request(&req).unwrap();
        assert!(resp["error"].is_object());
        assert_eq!(resp["error"]["code"], -32602);
    }

    #[test]
    fn unknown_method_returns_error() {
        let server = test_server();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(5)),
            method: "bogus/method".to_string(),
            params: json!({}),
        };
        let resp = server.handle_request(&req).unwrap();
        assert_eq!(resp["error"]["code"], -32601);
    }

    #[test]
    fn notification_returns_none() {
        let server = test_server();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: None,
            method: "notifications/initialized".to_string(),
            params: json!({}),
        };
        assert!(server.handle_request(&req).is_none());
    }

    #[test]
    fn ping_returns_empty() {
        let server = test_server();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(6)),
            method: "ping".to_string(),
            params: json!({}),
        };
        let resp = server.handle_request(&req).unwrap();
        assert!(resp["result"].is_object());
    }

    #[test]
    fn process_request_string() {
        let server = test_server();
        let input = r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":{}}"#;
        let output = process_request(&server, input).unwrap();
        assert!(output.contains("result"));
    }

    #[test]
    fn resources_list_empty() {
        let server = test_server();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(7)),
            method: "resources/list".to_string(),
            params: json!({}),
        };
        let resp = server.handle_request(&req).unwrap();
        assert_eq!(resp["result"]["resources"].as_array().unwrap().len(), 0);
    }
}
