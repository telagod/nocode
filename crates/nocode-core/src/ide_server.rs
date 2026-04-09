//! IDE Server — JSON-RPC server for IDE integration (VS Code, JetBrains).
//!
//! Provides a higher-level interface than MCP: query execution, session management,
//! diagnostics. Listens on TCP or stdio.
//! Launch: `nocode --ide-server [--port 3002]`

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

/// IDE server request handler — stateless, processes one request at a time.
pub struct IdeRequestHandler {
    config: IdeServerConfig,
    version: String,
}

impl IdeRequestHandler {
    pub fn new(config: IdeServerConfig) -> Self {
        Self {
            config,
            version: env!("CARGO_PKG_VERSION").to_string(),
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
            }
        }))
    }

    fn handle_query(&self, params: &Value) -> Result<Value, (i64, String)> {
        let prompt = params["prompt"].as_str()
            .ok_or((-32602, "Missing 'prompt' parameter".to_string()))?;
        // Stub: in production, this would invoke the query engine
        Ok(json!({
            "status": "accepted",
            "prompt": prompt,
            "message": "Query execution requires full runtime — use bridge mode for execution",
        }))
    }

    fn handle_status(&self) -> Result<Value, (i64, String)> {
        Ok(json!({
            "server": "nocode-ide",
            "version": self.version,
            "uptime_secs": 0,
            "active_queries": 0,
            "bind_addr": self.config.bind_addr,
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
        let _prefix = params["prefix"].as_str().unwrap_or("");
        // Stub: return slash command completions
        let commands: Vec<Value> = [
            "help", "status", "model", "clear", "quit", "compact",
            "review", "plan", "agents", "mcp", "memory",
        ].iter().map(|c| json!({"label": format!("/{c}"), "kind": "command"})).collect();
        Ok(json!({"items": commands}))
    }

    fn handle_hover(&self, params: &Value) -> Result<Value, (i64, String)> {
        let _file = params["file"].as_str().unwrap_or("");
        let _line = params["line"].as_u64().unwrap_or(0);
        // Stub: hover info would come from LSP integration
        Ok(json!({"contents": "Hover info not available — LSP integration pending"}))
    }
}

/// Parse a raw JSON-RPC string into an IdeRequest.
pub fn parse_ide_request(input: &str) -> Result<IdeRequest, String> {
    serde_json::from_str(input).map_err(|e| format!("parse error: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handler() -> IdeRequestHandler {
        IdeRequestHandler::new(IdeServerConfig::default())
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
        assert_eq!(resp["result"]["status"], "accepted");
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
    fn hover_returns_stub() {
        let h = handler();
        let req = make_req("hover", json!({"file": "main.rs", "line": 10}));
        let resp = h.handle(&req).unwrap();
        assert!(resp["result"]["contents"].is_string());
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
