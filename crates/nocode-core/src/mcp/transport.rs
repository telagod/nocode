//! MCP Transport abstraction — stdio and in-process transports.
//!
//! Allows MCP clients to communicate via stdio (default) or directly
//! via Rust function calls (in-process, for embedded MCP servers).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A JSON-RPC request envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcRequest {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    pub params: Value,
}

/// A JSON-RPC response envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcResponse {
    pub jsonrpc: String,
    pub id: Option<u64>,
    pub result: Option<Value>,
    pub error: Option<RpcError>,
}

/// A JSON-RPC error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default)]
    pub data: Option<Value>,
}

impl RpcResponse {
    pub fn success(id: u64, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id: Some(id),
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: u64, code: i64, message: &str) -> Self {
        // APPEND_REST
        Self {
            jsonrpc: "2.0".to_string(),
            id: Some(id),
            result: None,
            error: Some(RpcError {
                code,
                message: message.to_string(),
                data: None,
            }),
        }
    }
}

/// Transport trait — abstracts how JSON-RPC messages are sent/received.
pub trait McpTransport: Send + Sync {
    /// Send a request and receive a response.
    fn request(&mut self, req: RpcRequest) -> Result<RpcResponse, String>;

    /// Send a notification (no response expected).
    fn notify(&mut self, method: &str, params: Value) -> Result<(), String>;

    /// Check if the transport is still alive.
    fn is_alive(&self) -> bool;

    /// Shut down the transport.
    fn shutdown(&mut self);
}

// ---------------------------------------------------------------------------
// InProcessTransport — direct Rust function calls
// ---------------------------------------------------------------------------

/// Handler function type for in-process MCP servers.
pub type InProcessHandler = Box<dyn Fn(RpcRequest) -> RpcResponse + Send + Sync>;

/// In-process transport — calls a Rust function directly instead of stdio.
pub struct InProcessTransport {
    handler: InProcessHandler,
    alive: bool,
}

impl InProcessTransport {
    pub fn new(handler: InProcessHandler) -> Self {
        Self {
            handler,
            alive: true,
        }
    }
}

impl McpTransport for InProcessTransport {
    fn request(&mut self, req: RpcRequest) -> Result<RpcResponse, String> {
        if !self.alive {
            return Err("transport is shut down".to_string());
        }
        Ok((self.handler)(req))
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        if !self.alive {
            return Err("transport is shut down".to_string());
        }
        let req = RpcRequest {
            jsonrpc: "2.0".to_string(),
            id: 0,
            method: method.to_string(),
            params,
        };
        let _ = (self.handler)(req);
        Ok(())
    }

    fn is_alive(&self) -> bool {
        self.alive
    }

    fn shutdown(&mut self) {
        self.alive = false;
    }
}

// ---------------------------------------------------------------------------
// EchoTransport — for testing
// ---------------------------------------------------------------------------

/// Echo transport — returns the request params as the result (for testing).
pub struct EchoTransport {
    alive: bool,
    call_count: u64,
}

impl EchoTransport {
    pub fn new() -> Self {
        Self {
            alive: true,
            call_count: 0,
        }
    }

    pub fn call_count(&self) -> u64 {
        self.call_count
    }
}

impl Default for EchoTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl McpTransport for EchoTransport {
    fn request(&mut self, req: RpcRequest) -> Result<RpcResponse, String> {
        if !self.alive {
            return Err("transport is shut down".to_string());
        }
        self.call_count += 1;
        Ok(RpcResponse::success(
            req.id,
            serde_json::json!({
                "echo_method": req.method,
                "echo_params": req.params,
            }),
        ))
    }

    fn notify(&mut self, _method: &str, _params: Value) -> Result<(), String> {
        if !self.alive {
            return Err("transport is shut down".to_string());
        }
        self.call_count += 1;
        Ok(())
    }

    fn is_alive(&self) -> bool {
        self.alive
    }

    fn shutdown(&mut self) {
        self.alive = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rpc_response_success() {
        let resp = RpcResponse::success(1, json!({"tools": []}));
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
        assert_eq!(resp.id, Some(1));
    }

    #[test]
    fn rpc_response_error() {
        let resp = RpcResponse::error(2, -32601, "method not found");
        assert!(resp.result.is_none());
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32601);
    }

    #[test]
    fn rpc_request_serde_roundtrip() {
        let req = RpcRequest {
            jsonrpc: "2.0".to_string(),
            id: 42,
            method: "tools/list".to_string(),
            params: json!({}),
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: RpcRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, 42);
        assert_eq!(parsed.method, "tools/list");
    }

    #[test]
    fn echo_transport_echoes() {
        let mut transport = EchoTransport::new();
        let req = RpcRequest {
            jsonrpc: "2.0".to_string(),
            id: 1,
            method: "tools/list".to_string(),
            params: json!({"key": "value"}),
        };
        let resp = transport.request(req).unwrap();
        assert_eq!(resp.result.as_ref().unwrap()["echo_method"], "tools/list");
        assert_eq!(transport.call_count(), 1);
    }

    #[test]
    fn echo_transport_shutdown() {
        let mut transport = EchoTransport::new();
        assert!(transport.is_alive());
        transport.shutdown();
        assert!(!transport.is_alive());
        let req = RpcRequest {
            jsonrpc: "2.0".to_string(),
            id: 1,
            method: "test".to_string(),
            params: json!({}),
        };
        assert!(transport.request(req).is_err());
    }

    #[test]
    fn in_process_transport_calls_handler() {
        let handler: InProcessHandler =
            Box::new(|req| RpcResponse::success(req.id, json!({"handled": true})));
        let mut transport = InProcessTransport::new(handler);
        let req = RpcRequest {
            jsonrpc: "2.0".to_string(),
            id: 5,
            method: "test/call".to_string(),
            params: json!({}),
        };
        let resp = transport.request(req).unwrap();
        assert_eq!(resp.result.as_ref().unwrap()["handled"], true);
    }

    #[test]
    fn in_process_transport_shutdown() {
        let handler: InProcessHandler = Box::new(|req| RpcResponse::success(req.id, json!({})));
        let mut transport = InProcessTransport::new(handler);
        transport.shutdown();
        assert!(!transport.is_alive());
        assert!(transport.notify("test", json!({})).is_err());
    }

    #[test]
    fn in_process_transport_notify() {
        let handler: InProcessHandler = Box::new(|req| RpcResponse::success(req.id, json!({})));
        let mut transport = InProcessTransport::new(handler);
        assert!(transport.notify("initialized", json!({})).is_ok());
    }
}
