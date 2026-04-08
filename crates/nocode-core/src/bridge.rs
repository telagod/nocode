//! Bridge HTTP service — lightweight HTTP server for remote query execution.
//!
//! Endpoints:
//! - POST /v1/query — single-turn query execution
//! - GET /v1/sessions — list sessions
//! - POST /v1/sessions/:id/resume — resume a session

use crate::message::SystemBlock;
use crate::provider::Provider;
use crate::query::r#loop::{self, LoopConfig, NoopObserver};
use crate::session::control::Session;
use crate::session::registry::global_session_registry;
use crate::tool::ToolRegistry;
use crate::tool::executor::ToolExecutor;
use std::io::{BufRead, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

/// Bridge service configuration.
pub struct BridgeConfig {
    pub bind_addr: String,
    pub auth_token: Option<String>,
    /// Heartbeat interval in seconds (0 = disabled).
    pub heartbeat_interval_secs: u64,
    /// Connection timeout in seconds (no heartbeat within this = disconnect).
    pub connection_timeout_secs: u64,
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            bind_addr: "127.0.0.1:3000".to_string(),
            auth_token: None,
            heartbeat_interval_secs: 30,
            connection_timeout_secs: 90,
        }
    }
}

/// Runtime context for bridge query execution.
pub struct BridgeRuntime {
    pub provider: Arc<dyn Provider>,
    pub registry: ToolRegistry,
    pub system: Vec<SystemBlock>,
    pub model: String,
    pub max_tokens: u32,
    pub max_turns: u32,
}

/// Tracks a connected client session.
#[derive(Debug, Clone)]
pub struct BridgeConnection {
    pub client_id: String,
    pub session_id: Option<String>,
    pub connected_at: std::time::Instant,
    pub last_heartbeat: std::time::Instant,
    pub authenticated: bool,
}

impl BridgeConnection {
    pub fn new(client_id: &str) -> Self {
        let now = std::time::Instant::now();
        Self {
            client_id: client_id.to_string(),
            session_id: None,
            connected_at: now,
            last_heartbeat: now,
            authenticated: false,
        }
    }

    /// Record a heartbeat from this client.
    pub fn heartbeat(&mut self) {
        self.last_heartbeat = std::time::Instant::now();
    }

    /// Check if the connection has timed out.
    pub fn is_timed_out(&self, timeout_secs: u64) -> bool {
        if timeout_secs == 0 {
            return false;
        }
        self.last_heartbeat.elapsed().as_secs() >= timeout_secs
    }

    /// Bind this connection to a session.
    pub fn bind_session(&mut self, session_id: &str) {
        self.session_id = Some(session_id.to_string());
        self.heartbeat();
    }
}

/// Registry of active bridge connections.
pub struct ConnectionRegistry {
    connections: std::collections::HashMap<String, BridgeConnection>,
    next_id: u64,
}

impl ConnectionRegistry {
    pub fn new() -> Self {
        Self {
            connections: std::collections::HashMap::new(),
            next_id: 1,
        }
    }

    /// Register a new connection, returning its client ID.
    pub fn register(&mut self) -> String {
        let id = format!("client-{}", self.next_id);
        self.next_id += 1;
        self.connections
            .insert(id.clone(), BridgeConnection::new(&id));
        id
    }

    /// Record a heartbeat for a client.
    pub fn heartbeat(&mut self, client_id: &str) -> bool {
        if let Some(conn) = self.connections.get_mut(client_id) {
            conn.heartbeat();
            true
        } else {
            false
        }
    }

    /// Remove timed-out connections. Returns IDs of removed connections.
    pub fn sweep_timeouts(&mut self, timeout_secs: u64) -> Vec<String> {
        let timed_out: Vec<String> = self
            .connections
            .values()
            .filter(|c| c.is_timed_out(timeout_secs))
            .map(|c| c.client_id.clone())
            .collect();
        for id in &timed_out {
            self.connections.remove(id);
        }
        timed_out
    }

    /// Get a connection by client ID.
    pub fn get(&self, client_id: &str) -> Option<&BridgeConnection> {
        self.connections.get(client_id)
    }

    /// Get a mutable connection by client ID.
    pub fn get_mut(&mut self, client_id: &str) -> Option<&mut BridgeConnection> {
        self.connections.get_mut(client_id)
    }

    /// Remove a connection.
    pub fn remove(&mut self, client_id: &str) {
        self.connections.remove(client_id);
    }

    /// Number of active connections.
    pub fn len(&self) -> usize {
        self.connections.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.connections.is_empty()
    }
}

impl Default for ConnectionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// A parsed HTTP request.
struct HttpRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: String,
}

/// Parse an HTTP request from a TCP stream.
fn parse_request(stream: &mut TcpStream) -> Option<HttpRequest> {
    let mut reader = std::io::BufReader::new(stream.try_clone().ok()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).ok()?;

    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }
    let method = parts[0].to_string();
    let path = parts[1].to_string();

    let mut headers = Vec::new();
    let mut content_length: usize = 0;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            break;
        }
        let line = line.trim().to_string();
        if line.is_empty() {
            break;
        }
        if let Some((key, val)) = line.split_once(':') {
            let key = key.trim().to_lowercase();
            let val = val.trim().to_string();
            if key == "content-length" {
                content_length = val.parse().unwrap_or(0);
            }
            headers.push((key, val));
        }
    }

    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body).ok()?;
    }

    Some(HttpRequest {
        method,
        path,
        headers,
        body: String::from_utf8_lossy(&body).to_string(),
    })
}

/// Send an HTTP response.
fn send_response(stream: &mut TcpStream, status: u16, status_text: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status} {status_text}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn send_ok(stream: &mut TcpStream, body: &str) {
    send_response(stream, 200, "OK", body);
}

fn send_error(stream: &mut TcpStream, status: u16, message: &str) {
    let body = serde_json::json!({"error": message}).to_string();
    send_response(stream, status, "Error", &body);
}

/// Check auth token if configured.
fn check_auth(req: &HttpRequest, expected: &Option<String>) -> bool {
    let Some(token) = expected else {
        return true;
    };
    req.headers
        .iter()
        .any(|(k, v)| k == "authorization" && v.strip_prefix("Bearer ").unwrap_or("") == token)
}

/// Handle a single HTTP request.
fn handle_request(
    stream: &mut TcpStream,
    req: &HttpRequest,
    config: &BridgeConfig,
    project_root: &str,
    connections: &mut ConnectionRegistry,
    runtime: Option<&BridgeRuntime>,
) {
    if !check_auth(req, &config.auth_token) {
        send_error(stream, 401, "Unauthorized");
        return;
    }

    // Sweep timed-out connections on each request
    connections.sweep_timeouts(config.connection_timeout_secs);

    match (req.method.as_str(), req.path.as_str()) {
        ("POST", "/v1/query") => handle_query(stream, req, runtime),
        ("GET", "/v1/sessions") => handle_list_sessions(stream, project_root),
        ("GET", "/v1/health") => {
            let body = serde_json::json!({
                "status": "ok",
                "connections": connections.len(),
            })
            .to_string();
            send_ok(stream, &body);
        }
        ("POST", "/v1/connect") => {
            let client_id = connections.register();
            if let Some(conn) = connections.get_mut(&client_id) {
                conn.authenticated = true;
            }
            let body = serde_json::json!({
                "client_id": client_id,
                "heartbeat_interval_secs": config.heartbeat_interval_secs,
            })
            .to_string();
            send_ok(stream, &body);
        }
        ("POST", "/v1/heartbeat") => {
            let body: serde_json::Value = serde_json::from_str(&req.body).unwrap_or_default();
            let client_id = body["client_id"].as_str().unwrap_or("");
            if connections.heartbeat(client_id) {
                send_ok(stream, r#"{"status":"ok"}"#);
            } else {
                send_error(stream, 404, "Unknown client_id — reconnect required");
            }
        }
        ("POST", "/v1/disconnect") => {
            let body: serde_json::Value = serde_json::from_str(&req.body).unwrap_or_default();
            let client_id = body["client_id"].as_str().unwrap_or("");
            connections.remove(client_id);
            send_ok(stream, r#"{"status":"disconnected"}"#);
        }
        _ => {
            // Check for /v1/sessions/:id pattern
            if req.method == "GET"
                && req.path.starts_with("/v1/sessions/")
                && !req.path.ends_with("/resume")
            {
                let id = &req.path["/v1/sessions/".len()..];
                handle_get_session(stream, id, project_root);
            } else {
                send_error(stream, 404, "Not found");
            }
        }
    }
}

/// POST /v1/query — execute a single-turn query via agentic loop.
fn handle_query(stream: &mut TcpStream, req: &HttpRequest, runtime: Option<&BridgeRuntime>) {
    let body: serde_json::Value = match serde_json::from_str(&req.body) {
        Ok(v) => v,
        Err(e) => {
            send_error(stream, 400, &format!("Invalid JSON: {e}"));
            return;
        }
    };

    let prompt = body["prompt"].as_str().unwrap_or("");
    if prompt.is_empty() {
        send_error(stream, 400, "Missing 'prompt' field");
        return;
    }

    let Some(rt) = runtime else {
        // No runtime configured — return acknowledgment only
        let response = serde_json::json!({
            "status": "received",
            "prompt": prompt,
            "message": "No provider configured. Bridge running in stub mode."
        });
        send_ok(stream, &response.to_string());
        return;
    };

    let model_override = body["model"].as_str().unwrap_or(&rt.model);
    let messages = vec![crate::message::Message::user_text(prompt)];
    let tool_defs = rt.registry.definitions();

    let cfg = LoopConfig {
        model: model_override.to_string(),
        max_tokens: rt.max_tokens,
        max_turns: rt.max_turns,
        system: rt.system.clone(),
        tools: tool_defs,
        parallel_tool_execution: true,
    };

    let executor = ToolExecutor::new(&rt.registry);
    let mut observer = NoopObserver;

    match r#loop::run_agentic_loop(
        rt.provider.as_ref(),
        &executor,
        &cfg,
        messages,
        &mut observer,
    ) {
        Ok(result) => {
            let text: String = result
                .messages
                .iter()
                .filter(|m| m.role == crate::message::Role::Assistant)
                .flat_map(|m| &m.content)
                .filter_map(|b| {
                    if let crate::message::ContentBlock::Text { text } = b {
                        Some(text.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("");

            let response = serde_json::json!({
                "text": text,
                "model": model_override,
                "input_tokens": result.total_input_tokens,
                "output_tokens": result.total_output_tokens,
                "turns": result.turns,
                "stop_reason": format!("{:?}", result.stop_reason),
            });
            send_ok(stream, &response.to_string());
        }
        Err(e) => {
            send_error(stream, 500, &format!("Query execution failed: {e}"));
        }
    }
}

/// GET /v1/sessions — list all sessions.
fn handle_list_sessions(stream: &mut TcpStream, project_root: &str) {
    let reg = global_session_registry();
    let mut reg = reg.lock().unwrap_or_else(|e| e.into_inner());

    // Auto-discover from disk if empty
    if reg.is_empty() {
        reg.load_from_disk(project_root);
    }

    let sessions: Vec<serde_json::Value> = reg
        .list()
        .iter()
        .map(|m| {
            serde_json::json!({
                "id": m.id,
                "state": format!("{:?}", m.state),
                "model": m.model,
                "message_count": m.message_count,
                "created_at": m.created_at,
                "updated_at": m.updated_at,
                "parent_id": m.parent_id,
            })
        })
        .collect();

    let body = serde_json::json!({ "sessions": sessions }).to_string();
    send_ok(stream, &body);
}

/// GET /v1/sessions/:id — get session details.
fn handle_get_session(stream: &mut TcpStream, id: &str, project_root: &str) {
    match Session::load_meta(project_root, id) {
        Ok(meta) => {
            let body = serde_json::json!({
                "id": meta.id,
                "state": format!("{:?}", meta.state),
                "model": meta.model,
                "message_count": meta.message_count,
                "created_at": meta.created_at,
                "updated_at": meta.updated_at,
                "parent_id": meta.parent_id,
            })
            .to_string();
            send_ok(stream, &body);
        }
        Err(e) => {
            send_error(stream, 404, &format!("Session not found: {e}"));
        }
    }
}

/// Start the bridge HTTP server. Blocks until shutdown.
pub fn run_bridge_server(
    config: BridgeConfig,
    project_root: &str,
    runtime: Option<BridgeRuntime>,
) -> Result<(), String> {
    let listener = TcpListener::bind(&config.bind_addr)
        .map_err(|e| format!("Failed to bind {}: {e}", config.bind_addr))?;

    eprintln!("Bridge server listening on {}", config.bind_addr);

    let mut connections = ConnectionRegistry::new();

    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Connection error: {e}");
                continue;
            }
        };

        if let Some(req) = parse_request(&mut stream) {
            handle_request(
                &mut stream,
                &req,
                &config,
                project_root,
                &mut connections,
                runtime.as_ref(),
            );
        } else {
            send_error(&mut stream, 400, "Malformed request");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpStream;

    fn start_test_server(max_conns: usize) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let handle = std::thread::spawn(move || {
            let mut connections = ConnectionRegistry::new();
            for mut stream in listener.incoming().take(max_conns).flatten() {
                let config = BridgeConfig::default();
                if let Some(req) = parse_request(&mut stream) {
                    handle_request(&mut stream, &req, &config, "/tmp", &mut connections, None);
                }
            }
        });
        (addr, handle)
    }

    #[test]
    fn health_endpoint() {
        let (addr, handle) = start_test_server(1);
        std::thread::sleep(std::time::Duration::from_millis(50));

        let mut stream = TcpStream::connect(&addr).unwrap();
        stream
            .write_all(b"GET /v1/health HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.contains("200 OK"));
        assert!(response.contains(r#""status":"ok""#));

        let _ = handle.join();
    }

    #[test]
    fn query_endpoint() {
        let (addr, handle) = start_test_server(1);
        std::thread::sleep(std::time::Duration::from_millis(50));

        let body = r#"{"prompt":"hello","model":"test"}"#;
        let req = format!(
            "POST /v1/query HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let mut stream = TcpStream::connect(&addr).unwrap();
        stream.write_all(req.as_bytes()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.contains("200 OK"));
        assert!(response.contains("received"));

        let _ = handle.join();
    }

    #[test]
    fn connect_and_heartbeat() {
        let (addr, handle) = start_test_server(2);
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Connect
        let req = "POST /v1/connect HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\n\r\n";
        let mut stream = TcpStream::connect(&addr).unwrap();
        stream.write_all(req.as_bytes()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.contains("200 OK"));
        assert!(response.contains("client-1"));

        // Heartbeat
        let body = r#"{"client_id":"client-1"}"#;
        let req = format!(
            "POST /v1/heartbeat HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let mut stream = TcpStream::connect(&addr).unwrap();
        stream.write_all(req.as_bytes()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.contains("200 OK"));

        let _ = handle.join();
    }

    #[test]
    fn connection_registry_timeout() {
        let mut reg = ConnectionRegistry::new();
        let id = reg.register();
        assert_eq!(reg.len(), 1);

        // Simulate timeout by checking with 0s timeout (immediate)
        let swept = reg.sweep_timeouts(0);
        assert!(swept.is_empty()); // 0 = disabled

        // Connection should still be there
        assert!(reg.get(&id).is_some());

        // Remove manually
        reg.remove(&id);
        assert!(reg.is_empty());
    }
}
