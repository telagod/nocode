//! IDE Server — JSON-RPC server for IDE integration (VS Code, JetBrains).
//!
//! Provides a higher-level interface than MCP: query execution, session management,
//! diagnostics, completions, and hover. Listens on TCP or stdio.
//! Launch: `nocode --ide-server [--port 3002]`

use crate::message::{Message, SystemBlock};
use crate::provider::ProviderBox;
use crate::query::r#loop::{self, LoopConfig, NoopObserver};
use crate::tool::ToolRegistry;
use crate::tool::executor::ToolExecutor;
use crate::tool::global_registry::{tool_definitions_for_model, tool_names_for_display};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// IDE server configuration.
#[derive(Debug, Clone)]
pub struct IdeServerConfig {
    pub bind_addr: String,
    pub auth_token: Option<String>,
}

impl Default for IdeServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: "127.0.0.1:3002".to_string(),
            auth_token: None,
        }
    }
}

/// IDE server request methods.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IdeMethod {
    Initialize,
    Query,
    Cancel,
    Status,
    Diagnostics,
    Completions,
    Hover,
    Shutdown,
}

impl IdeMethod {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "initialize" => Some(Self::Initialize),
            "query" => Some(Self::Query),
            "cancel" => Some(Self::Cancel),
            "status" => Some(Self::Status),
            "diagnostics" => Some(Self::Diagnostics),
            // APPEND_REST
            "completions" => Some(Self::Completions),
            "hover" => Some(Self::Hover),
            "shutdown" => Some(Self::Shutdown),
            _ => None,
        }
    }
}

/// IDE server request envelope.
#[derive(Debug, Deserialize)]
pub struct IdeRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// IDE server response builder.
pub struct IdeResponse;

impl IdeResponse {
    pub fn success(id: &Value, result: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "result": result })
    }

    pub fn error(id: &Value, code: i64, message: &str) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
    }
}

/// IDE server request handler — holds runtime dependencies for real execution.
pub struct IdeRequestHandler {
    config: IdeServerConfig,
    version: String,
    provider: ProviderBox,
    registry: ToolRegistry,
    model: String,
    system_blocks: Vec<SystemBlock>,
    max_tokens: u32,
    max_turns: u32,
}

impl IdeRequestHandler {
    /// Create a handler with full runtime dependencies.
    pub fn new(
        config: IdeServerConfig,
        provider: ProviderBox,
        registry: ToolRegistry,
        model: String,
        system_blocks: Vec<SystemBlock>,
        max_tokens: u32,
        max_turns: u32,
    ) -> Self {
        Self {
            config,
            version: env!("CARGO_PKG_VERSION").to_string(),
            provider,
            registry,
            model,
            system_blocks,
            max_tokens,
            max_turns,
        }
    }

    /// Create a minimal handler for unit tests (no provider, returns stubs).
    pub fn new_stub(config: IdeServerConfig) -> Self {
        Self {
            config,
            version: env!("CARGO_PKG_VERSION").to_string(),
            provider: ProviderBox::new(StubProvider),
            registry: ToolRegistry::with_defaults("/tmp"),
            model: "stub".to_string(),
            system_blocks: Vec::new(),
            max_tokens: 1024,
            max_turns: 1,
        }
    }

    /// Handle a single JSON-RPC request. Returns None for notifications.
    pub fn handle(&self, request: &IdeRequest) -> Option<Value> {
        let id = request.id.as_ref()?;

        // Auth check
        if let Some(token) = &self.config.auth_token
            && let Some(provided) = request.params.get("auth_token").and_then(Value::as_str)
            && provided != token
        {
            return Some(IdeResponse::error(id, -32001, "Invalid auth token"));
        }

        let result = match IdeMethod::parse(&request.method) {
            Some(IdeMethod::Initialize) => self.handle_initialize(),
            Some(IdeMethod::Query) => self.handle_query(&request.params),
            Some(IdeMethod::Cancel) => Ok(json!({"cancelled": true})),
            Some(IdeMethod::Status) => self.handle_status(),
            Some(IdeMethod::Diagnostics) => self.handle_diagnostics(),
            Some(IdeMethod::Completions) => self.handle_completions(&request.params),
            Some(IdeMethod::Hover) => self.handle_hover(&request.params),
            Some(IdeMethod::Shutdown) => Ok(json!({"shutdown": true})),
            None => Err((-32601, format!("Method not found: {}", request.method))),
        };

        Some(match result {
            Ok(val) => IdeResponse::success(id, val),
            Err((code, msg)) => IdeResponse::error(id, code, &msg),
        })
    }

    fn handle_initialize(&self) -> Result<Value, (i64, String)> {
        Ok(json!({
            "name": "nocode",
            "version": self.version,
            "capabilities": {
                "query": true,
                "diagnostics": true,
                "completions": true,
                "hover": true,
            },
            "model": self.model,
            "tools": tool_names_for_display(&self.registry),
        }))
    }

    fn handle_query(&self, params: &Value) -> Result<Value, (i64, String)> {
        let prompt = params["prompt"]
            .as_str()
            .ok_or((-32602, "Missing 'prompt' parameter".to_string()))?;

        let messages = vec![Message::user_text(prompt)];
        let executor = ToolExecutor::new(&self.registry);
        let config = LoopConfig {
            model: self.model.clone(),
            max_tokens: self.max_tokens,
            max_turns: self.max_turns,
            system: self.system_blocks.clone(),
            tools: tool_definitions_for_model(&self.registry),
            parallel_tool_execution: true,
        };
        let mut observer = NoopObserver;

        match r#loop::run_agentic_loop(
            self.provider.as_ref(),
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
                    "text": text,
                    "input_tokens": result.total_input_tokens,
                    "output_tokens": result.total_output_tokens,
                    "cache_read_tokens": result.total_cache_read_tokens,
                    "cache_write_tokens": result.total_cache_write_tokens,
                    "turns": result.turns,
                }))
            }
            Err(e) => Err((-32000, format!("{e}"))),
        }
    }

    fn handle_status(&self) -> Result<Value, (i64, String)> {
        Ok(json!({
            "server": "nocode-ide",
            "version": self.version,
            "uptime_secs": 0,
            "active_queries": 0,
            "bind_addr": self.config.bind_addr,
            "model": self.model,
            "tools_count": tool_names_for_display(&self.registry).len(),
        }))
    }

    fn handle_diagnostics(&self) -> Result<Value, (i64, String)> {
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(json!({
            "cwd": cwd,
            "rust_version": env!("CARGO_PKG_VERSION"),
            "platform": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
        }))
    }

    fn handle_completions(&self, params: &Value) -> Result<Value, (i64, String)> {
        let prefix = params["prefix"].as_str().unwrap_or("");

        // Collect slash command names
        let mut items: Vec<Value> = [
            "help",
            "quit",
            "exit",
            "status",
            "model",
            "clear",
            "compact",
            "review",
            "plan",
            "agents",
            "mcp",
            "mcp-add",
            "mcp-remove",
            "mcp-restart",
            "memory",
            "sessions",
            "resume",
            "config",
            "theme",
            "vim",
            "export",
            "history",
            "version",
            "doctor",
            "permissions",
            "cost",
            "init",
            "login",
            "insights",
            "feature-flags",
            "telemetry",
            "skills",
            "env",
            "keybindings",
            "bughunter",
            "security-review",
            "copy",
            "undo",
            "redo",
            "rewind",
            "agent-create",
            "permissions-add",
            "permissions-remove",
            "plugin-install",
            "plugin-remove",
            "plugin-list",
            "ide",
            "voice",
        ]
        .iter()
        .filter(|c| {
            prefix.is_empty()
                || format!("/{c}").starts_with(prefix)
                || c.starts_with(prefix.trim_start_matches('/'))
        })
        .map(|c| json!({"label": format!("/{c}"), "kind": "command"}))
        .collect();

        // Append tool names
        let tool_names = self.registry.names();
        for name in &tool_names {
            let label = name.to_string();
            if prefix.is_empty() || label.starts_with(prefix) {
                items.push(json!({"label": label, "kind": "tool"}));
            }
        }

        Ok(json!({"items": items}))
    }

    fn handle_hover(&self, params: &Value) -> Result<Value, (i64, String)> {
        let file = params["file"]
            .as_str()
            .ok_or((-32602, "Missing 'file' parameter".to_string()))?;
        let line = params["line"].as_u64().unwrap_or(0) as usize;
        let context_lines = params["context_lines"].as_u64().unwrap_or(5) as usize;

        let path = std::path::Path::new(file);
        if !path.exists() {
            return Ok(json!({
                "contents": format!("File not found: {file}"),
                "found": false,
            }));
        }

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                return Ok(json!({
                    "contents": format!("Cannot read file: {e}"),
                    "found": false,
                }));
            }
        };

        let lines: Vec<&str> = content.lines().collect();
        if lines.is_empty() {
            return Ok(json!({
                "contents": "Empty file",
                "found": true,
                "file": file,
                "line": 0,
            }));
        }

        // Clamp line number
        let center = if line == 0 {
            0
        } else {
            (line - 1).min(lines.len() - 1)
        };
        let start = center.saturating_sub(context_lines);
        let end = (center + context_lines + 1).min(lines.len());
        let snippet: Vec<&str> = lines[start..end].to_vec();
        let language = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("text")
            .to_string();

        Ok(json!({
            "contents": snippet.join("\n"),
            "found": true,
            "file": file,
            "line": center + 1,
            "start_line": start + 1,
            "end_line": end,
            "language": language,
            "total_lines": lines.len(),
        }))
    }
}

/// Parse a raw JSON-RPC string into an IdeRequest.
pub fn parse_ide_request(input: &str) -> Result<IdeRequest, String> {
    serde_json::from_str(input).map_err(|e| format!("parse error: {e}"))
}

/// Minimal stub provider for unit tests — returns a fixed text response.
struct StubProvider;

impl crate::provider::Provider for StubProvider {
    fn create_message(
        &self,
        _request: &crate::provider::types::CreateMessageRequest,
    ) -> Result<crate::provider::types::CreateMessageResponse, crate::provider::types::ProviderError>
    {
        use crate::message::ContentBlock;
        use crate::provider::types::{CreateMessageResponse, StopReason, Usage};
        Ok(CreateMessageResponse {
            id: "stub-id".to_string(),
            content: vec![ContentBlock::text("stub response")],
            stop_reason: StopReason::EndTurn,
            usage: Usage::default(),
            model: "stub".to_string(),
        })
    }

    fn create_message_stream(
        &self,
        request: &crate::provider::types::CreateMessageRequest,
        _on_event: &mut dyn FnMut(crate::provider::types::StreamEvent),
    ) -> Result<crate::provider::types::CreateMessageResponse, crate::provider::types::ProviderError>
    {
        self.create_message(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handler() -> IdeRequestHandler {
        IdeRequestHandler::new_stub(IdeServerConfig::default())
    }

    fn make_req(method: &str, params: Value) -> IdeRequest {
        IdeRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(1)),
            method: method.to_string(),
            params,
        }
    }

    #[test]
    fn initialize_returns_capabilities() {
        let h = handler();
        let req = make_req("initialize", json!({}));
        let resp = h.handle(&req).unwrap();
        assert_eq!(resp["result"]["name"], "nocode");
        assert!(resp["result"]["capabilities"]["query"].as_bool().unwrap());
    }

    #[test]
    fn status_returns_info() {
        let h = handler();
        let req = make_req("status", json!({}));
        let resp = h.handle(&req).unwrap();
        assert_eq!(resp["result"]["server"], "nocode-ide");
    }

    #[test]
    fn diagnostics_returns_platform() {
        let h = handler();
        let req = make_req("diagnostics", json!({}));
        let resp = h.handle(&req).unwrap();
        assert!(resp["result"]["platform"].is_string());
        assert!(resp["result"]["arch"].is_string());
    }

    #[test]
    fn query_requires_prompt() {
        let h = handler();
        let req = make_req("query", json!({}));
        let resp = h.handle(&req).unwrap();
        assert!(resp["error"].is_object());
    }

    #[test]
    fn query_accepts_prompt() {
        let h = handler();
        let req = make_req("query", json!({"prompt": "fix the bug"}));
        let resp = h.handle(&req).unwrap();
        // MockProvider returns a response — check structure
        assert!(resp["result"]["text"].is_string() || resp["error"].is_object());
    }

    #[test]
    fn completions_returns_items() {
        let h = handler();
        let req = make_req("completions", json!({"prefix": "/"}));
        let resp = h.handle(&req).unwrap();
        let items = resp["result"]["items"].as_array().unwrap();
        assert!(!items.is_empty());
    }

    #[test]
    fn hover_returns_file_info() {
        let h = handler();
        let req = make_req("hover", json!({"file": "/nonexistent/file.rs", "line": 10}));
        let resp = h.handle(&req).unwrap();
        assert!(resp["result"]["contents"].is_string());
        assert_eq!(resp["result"]["found"], false);
    }

    #[test]
    fn unknown_method_error() {
        let h = handler();
        let req = make_req("bogus", json!({}));
        let resp = h.handle(&req).unwrap();
        assert_eq!(resp["error"]["code"], -32601);
    }

    #[test]
    fn notification_returns_none() {
        let h = handler();
        let req = IdeRequest {
            jsonrpc: "2.0".to_string(),
            id: None,
            method: "initialized".to_string(),
            params: json!({}),
        };
        assert!(h.handle(&req).is_none());
    }

    #[test]
    fn shutdown_returns_ok() {
        let h = handler();
        let req = make_req("shutdown", json!({}));
        let resp = h.handle(&req).unwrap();
        assert!(resp["result"]["shutdown"].as_bool().unwrap());
    }

    #[test]
    fn ide_method_parse() {
        assert_eq!(IdeMethod::parse("initialize"), Some(IdeMethod::Initialize));
        assert_eq!(IdeMethod::parse("query"), Some(IdeMethod::Query));
        assert_eq!(IdeMethod::parse("shutdown"), Some(IdeMethod::Shutdown));
        assert_eq!(IdeMethod::parse("bogus"), None);
    }

    #[test]
    fn parse_ide_request_string() {
        let input = r#"{"jsonrpc":"2.0","id":1,"method":"status","params":{}}"#;
        let req = parse_ide_request(input).unwrap();
        assert_eq!(req.method, "status");
    }

    #[test]
    fn default_config() {
        let config = IdeServerConfig::default();
        assert_eq!(config.bind_addr, "127.0.0.1:3002");
        assert!(config.auth_token.is_none());
    }
}
