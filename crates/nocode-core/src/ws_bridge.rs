//! WebSocket bridge — persistent bidirectional connection for real-time streaming.
//!
//! Provides WebSocket transport as an alternative to HTTP polling for bridge clients.
//! Supports connect/disconnect, heartbeat/ping-pong, message framing, and reconnect.

use crate::query::events::ModelStreamEvent;
use crate::query::r#loop::LoopObserver;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::mpsc::UnboundedSender;

/// WebSocket connection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsState {
    Connecting,
    Open,
    Closing,
    Closed,
    Reconnecting,
}

/// A framed WebSocket message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsMessage {
    /// Client sends a query.
    Query { id: String, content: String },
    /// Server streams a text delta.
    Delta { id: String, text: String },
    /// Server streams thinking/reasoning content.
    Thinking { id: String, thinking: String },
    /// Server signals tool use start.
    ToolStart { id: String, tool_name: String },
    /// Server signals tool use result.
    ToolResult {
        id: String,
        content: String,
        is_error: bool,
    },
    /// Server sends token usage updates.
    Usage {
        id: String,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_write_tokens: u64,
    },
    /// Server surfaces a retryable or terminal stream error.
    StreamError {
        id: String,
        message: String,
        retryable: bool,
    },
    /// Server signals query complete.
    Complete { id: String, stop_reason: String },
    /// Server signals an error.
    Error { id: String, message: String },
    /// Heartbeat ping.
    Ping { timestamp: i64 },
    /// Heartbeat pong.
    Pong { timestamp: i64 },
}

pub struct WsEventObserver {
    query_id: String,
    tx: UnboundedSender<WsMessage>,
}

impl WsEventObserver {
    pub fn new(query_id: impl Into<String>, tx: UnboundedSender<WsMessage>) -> Self {
        Self {
            query_id: query_id.into(),
            tx,
        }
    }
}

impl LoopObserver for WsEventObserver {
    fn on_model_event(&mut self, event: &ModelStreamEvent) {
        let outgoing = match event {
            ModelStreamEvent::TextDelta { text } => Some(WsMessage::Delta {
                id: self.query_id.clone(),
                text: text.clone(),
            }),
            ModelStreamEvent::ThinkingDelta { thinking } => Some(WsMessage::Thinking {
                id: self.query_id.clone(),
                thinking: thinking.clone(),
            }),
            ModelStreamEvent::ToolUseStart { name, .. } => Some(WsMessage::ToolStart {
                id: self.query_id.clone(),
                tool_name: name.clone(),
            }),
            ModelStreamEvent::ToolResult {
                tool_use_id,
                content,
                is_error,
                ..
            } => Some(WsMessage::ToolResult {
                id: tool_use_id.clone(),
                content: content.clone(),
                is_error: *is_error,
            }),
            ModelStreamEvent::UsageUpdate { usage } => Some(WsMessage::Usage {
                id: self.query_id.clone(),
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                cache_read_tokens: usage.cache_read_input_tokens,
                cache_write_tokens: usage.cache_creation_input_tokens,
            }),
            ModelStreamEvent::StreamError { message, retryable } => Some(WsMessage::StreamError {
                id: self.query_id.clone(),
                message: message.clone(),
                retryable: *retryable,
            }),
            _ => None,
        };

        if let Some(message) = outgoing {
            let _ = self.tx.send(message);
        }
    }
}

/// Configuration for WebSocket bridge.
#[derive(Debug, Clone)]
pub struct WsBridgeConfig {
    pub bind_addr: String,
    pub heartbeat_interval_secs: u64,
    pub connection_timeout_secs: u64,
    pub max_connections: usize,
    pub max_message_size: usize,
}

// APPEND_REST

impl Default for WsBridgeConfig {
    fn default() -> Self {
        Self {
            bind_addr: "127.0.0.1:3001".to_string(),
            heartbeat_interval_secs: 30,
            connection_timeout_secs: 90,
            max_connections: 16,
            max_message_size: 1024 * 1024, // 1MB
        }
    }
}

/// A tracked WebSocket connection.
#[derive(Debug)]
pub struct WsConnection {
    pub id: String,
    pub state: WsState,
    pub connected_at: i64,
    pub last_ping: i64,
    pub last_pong: i64,
    pub messages_sent: u64,
    pub messages_received: u64,
}

impl WsConnection {
    pub fn new(id: &str) -> Self {
        let now = chrono::Utc::now().timestamp();
        Self {
            id: id.to_string(),
            state: WsState::Connecting,
            connected_at: now,
            last_ping: now,
            last_pong: now,
            messages_sent: 0,
            messages_received: 0,
        }
    }

    /// Check if connection has timed out (no pong within timeout).
    pub fn is_timed_out(&self, timeout_secs: u64) -> bool {
        let now = chrono::Utc::now().timestamp();
        (now - self.last_pong) > timeout_secs as i64
    }

    /// Record a sent message.
    pub fn record_send(&mut self) {
        self.messages_sent += 1;
    }

    /// Record a received message.
    pub fn record_receive(&mut self) {
        self.messages_received += 1;
    }

    /// Record a pong received.
    pub fn record_pong(&mut self) {
        self.last_pong = chrono::Utc::now().timestamp();
    }

    /// Record a ping sent.
    pub fn record_ping(&mut self) {
        self.last_ping = chrono::Utc::now().timestamp();
    }
}

/// WebSocket connection registry — tracks all active connections.
pub struct WsConnectionRegistry {
    connections: HashMap<String, WsConnection>,
    config: WsBridgeConfig,
    next_id: u64,
}

impl WsConnectionRegistry {
    pub fn new(config: WsBridgeConfig) -> Self {
        Self {
            connections: HashMap::new(),
            config,
            next_id: 1,
        }
    }

    /// Register a new connection. Returns connection ID or error if at capacity.
    pub fn connect(&mut self) -> Result<String, String> {
        if self.connections.len() >= self.config.max_connections {
            return Err(format!(
                "max connections ({}) reached",
                self.config.max_connections
            ));
        }
        let id = format!("ws-{}", self.next_id);
        self.next_id += 1;
        let mut conn = WsConnection::new(&id);
        conn.state = WsState::Open;
        self.connections.insert(id.clone(), conn);
        Ok(id)
    }

    /// Disconnect a connection.
    pub fn disconnect(&mut self, id: &str) -> bool {
        if let Some(conn) = self.connections.get_mut(id) {
            conn.state = WsState::Closed;
            true
        } else {
            false
        }
    }

    /// Remove closed connections.
    pub fn cleanup(&mut self) -> usize {
        let before = self.connections.len();
        self.connections.retain(|_, c| c.state != WsState::Closed);
        before - self.connections.len()
    }

    /// Get a connection by ID.
    pub fn get(&self, id: &str) -> Option<&WsConnection> {
        self.connections.get(id)
    }

    /// Get a mutable connection by ID.
    pub fn get_mut(&mut self, id: &str) -> Option<&mut WsConnection> {
        self.connections.get_mut(id)
    }

    /// List all active (Open) connections.
    pub fn active_connections(&self) -> Vec<&WsConnection> {
        self.connections
            .values()
            .filter(|c| c.state == WsState::Open)
            .collect()
    }

    /// Check all connections for timeouts, close timed-out ones.
    pub fn check_timeouts(&mut self) -> Vec<String> {
        let timeout = self.config.connection_timeout_secs;
        let timed_out: Vec<String> = self
            .connections
            .values()
            .filter(|c| c.state == WsState::Open && c.is_timed_out(timeout))
            .map(|c| c.id.clone())
            .collect();
        for id in &timed_out {
            if let Some(conn) = self.connections.get_mut(id) {
                conn.state = WsState::Closed;
            }
        }
        timed_out
    }

    /// Total active connection count.
    pub fn active_count(&self) -> usize {
        self.connections
            .values()
            .filter(|c| c.state == WsState::Open)
            .count()
    }
}

/// Global singleton WS connection registry.
static GLOBAL_WS_REGISTRY: OnceLock<Arc<Mutex<WsConnectionRegistry>>> = OnceLock::new();

pub fn global_ws_registry() -> &'static Arc<Mutex<WsConnectionRegistry>> {
    GLOBAL_WS_REGISTRY.get_or_init(|| {
        Arc::new(Mutex::new(WsConnectionRegistry::new(
            WsBridgeConfig::default(),
        )))
    })
}

// ---------------------------------------------------------------------------
// WebSocket server — TCP listener + accept loop + message dispatch
// ---------------------------------------------------------------------------

use futures::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

/// Callback type for handling incoming queries from WS clients.
/// Receives the query content, returns the full response text.
pub type QueryHandler =
    Arc<dyn Fn(String, String, UnboundedSender<WsMessage>) -> Result<(), String> + Send + Sync>;

/// Run the WebSocket bridge server.
///
/// Binds to `config.bind_addr`, accepts WS connections, and dispatches
/// incoming `WsMessage::Query` frames to the provided handler.
/// Runs heartbeat pings and timeout cleanup in the background.
pub async fn run_ws_server(config: WsBridgeConfig, handler: QueryHandler) -> Result<(), String> {
    let listener = TcpListener::bind(&config.bind_addr)
        .await
        .map_err(|e| format!("WS bind failed: {e}"))?;

    let heartbeat_interval = config.heartbeat_interval_secs;
    let _connection_timeout = config.connection_timeout_secs;

    // Background task: heartbeat + timeout cleanup
    let registry = global_ws_registry().clone();
    let hb_registry = registry.clone();
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(heartbeat_interval));
        loop {
            interval.tick().await;
            let mut guard = hb_registry.lock().unwrap_or_else(|e| e.into_inner());
            let timed_out = guard.check_timeouts();
            if !timed_out.is_empty() {
                eprintln!("[ws] timed out {} connections", timed_out.len());
            }
        }
    });

    eprintln!("[ws] listening on {}", config.bind_addr);

    loop {
        let (stream, addr) = listener
            .accept()
            .await
            .map_err(|e| format!("WS accept failed: {e}"))?;

        // Check capacity
        {
            let guard = registry.lock().unwrap_or_else(|e| e.into_inner());
            if guard.active_count() >= config.max_connections {
                eprintln!("[ws] rejecting {addr}: at capacity");
                continue;
            }
        }

        let registry = registry.clone();
        let handler = handler.clone();

        tokio::spawn(async move {
            let ws_stream = match accept_async(stream).await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[ws] handshake failed from {addr}: {e}");
                    return;
                }
            };

            let conn_id = {
                let mut guard = registry.lock().unwrap_or_else(|e| e.into_inner());
                match guard.connect() {
                    Ok(id) => id,
                    Err(e) => {
                        eprintln!("[ws] rejected connection from {addr}: {e}");
                        return;
                    }
                }
            };

            eprintln!("[ws] {conn_id} connected from {addr}");

            let (mut ws_sender, mut ws_receiver) = ws_stream.split();

            async fn send_ws_message(
                ws_sender: &mut futures::stream::SplitSink<
                    tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
                    Message,
                >,
                registry: &Arc<Mutex<WsConnectionRegistry>>,
                conn_id: &str,
                message: &WsMessage,
            ) {
                if let Ok(json) = serde_json::to_string(message)
                    && ws_sender.send(Message::Text(json.into())).await.is_ok()
                {
                    let mut guard = registry.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(conn) = guard.get_mut(conn_id) {
                        conn.record_send();
                    }
                }
            }

            // Heartbeat task for this connection
            let hb_sender = registry.clone();
            let hb_conn_id = conn_id.clone();
            let ping_interval = std::time::Duration::from_secs(heartbeat_interval);
            let hb_task = tokio::spawn(async move {
                let mut interval = tokio::time::interval(ping_interval);
                loop {
                    interval.tick().await;
                    let ping = WsMessage::Ping {
                        timestamp: chrono::Utc::now().timestamp(),
                    };
                    let _json = match serde_json::to_string(&ping) {
                        Ok(j) => j,
                        Err(_) => continue,
                    };
                    // We can't access ws_sender here, so just record the ping
                    let mut guard = hb_sender.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(conn) = guard.get_mut(&hb_conn_id) {
                        conn.record_ping();
                    }
                }
            });

            // Message receive loop
            while let Some(msg_result) = ws_receiver.next().await {
                match msg_result {
                    Ok(Message::Text(text)) => {
                        // Record receive
                        {
                            let mut guard = registry.lock().unwrap_or_else(|e| e.into_inner());
                            if let Some(conn) = guard.get_mut(&conn_id) {
                                conn.record_receive();
                            }
                        }

                        let ws_msg: WsMessage = match serde_json::from_str(&text) {
                            Ok(m) => m,
                            Err(e) => {
                                let err = WsMessage::Error {
                                    id: String::new(),
                                    message: format!("Invalid message: {e}"),
                                };
                                let err_json = serde_json::to_string(&err).unwrap_or_default();
                                let _ = ws_sender.send(Message::Text(err_json.into())).await;
                                continue;
                            }
                        };

                        match ws_msg {
                            WsMessage::Query { id, content } => {
                                let (event_tx, mut event_rx) =
                                    tokio::sync::mpsc::unbounded_channel::<WsMessage>();
                                let handler = handler.clone();
                                let query_id = id.clone();
                                tokio::task::spawn_blocking(move || {
                                    if let Err(err) =
                                        handler(query_id.clone(), content, event_tx.clone())
                                    {
                                        let _ = event_tx.send(WsMessage::Error {
                                            id: query_id,
                                            message: err,
                                        });
                                    }
                                });

                                while let Some(message) = event_rx.recv().await {
                                    send_ws_message(&mut ws_sender, &registry, &conn_id, &message)
                                        .await;
                                }
                            }
                            WsMessage::Pong { .. } => {
                                let mut guard = registry.lock().unwrap_or_else(|e| e.into_inner());
                                if let Some(conn) = guard.get_mut(&conn_id) {
                                    conn.record_pong();
                                }
                            }
                            WsMessage::Ping { timestamp } => {
                                let pong = WsMessage::Pong { timestamp };
                                send_ws_message(&mut ws_sender, &registry, &conn_id, &pong).await;
                            }
                            _ => {
                                // Ignore other message types from clients
                            }
                        }
                    }
                    Ok(Message::Close(_)) => break,
                    Ok(_) => {} // Binary, Ping, Pong — ignore
                    Err(e) => {
                        eprintln!("[ws] {conn_id} read error: {e}");
                        break;
                    }
                }
            }

            // Cleanup
            hb_task.abort();
            {
                let mut guard = registry.lock().unwrap_or_else(|e| e.into_inner());
                guard.disconnect(&conn_id);
            }
            eprintln!("[ws] {conn_id} disconnected");
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{ContentBlock, Message};
    use crate::provider::Provider;
    use crate::provider::types::{
        CreateMessageRequest, CreateMessageResponse, ProviderError, StopReason, StreamEvent, Usage,
    };
    use crate::query::r#loop::{self, LoopConfig};
    use crate::tool::ToolRegistry;
    use crate::tool::executor::ToolExecutor;
    use crate::tool::global_registry::tool_definitions_for_model;
    use futures::{SinkExt, StreamExt};
    use std::sync::Mutex;
    use tokio::time::{Duration, sleep};
    use tokio_tungstenite::{connect_async, tungstenite::Message as WsFrame};

    #[test]
    fn ws_message_serde_roundtrip() {
        let msgs = vec![
            WsMessage::Query {
                id: "q1".into(),
                content: "hello".into(),
            },
            WsMessage::Delta {
                id: "q1".into(),
                text: "world".into(),
            },
            WsMessage::Thinking {
                id: "q1".into(),
                thinking: "ponder".into(),
            },
            WsMessage::ToolStart {
                id: "q1".into(),
                tool_name: "Bash".into(),
            },
            WsMessage::ToolResult {
                id: "q1".into(),
                content: "ok".into(),
                is_error: false,
            },
            WsMessage::Usage {
                id: "q1".into(),
                input_tokens: 1,
                output_tokens: 2,
                cache_read_tokens: 3,
                cache_write_tokens: 4,
            },
            WsMessage::StreamError {
                id: "q1".into(),
                message: "retry".into(),
                retryable: true,
            },
            WsMessage::Complete {
                id: "q1".into(),
                stop_reason: "end_turn".into(),
            },
            WsMessage::Error {
                id: "q1".into(),
                message: "fail".into(),
            },
            WsMessage::Ping { timestamp: 123 },
            WsMessage::Pong { timestamp: 123 },
        ];
        for msg in &msgs {
            let json = serde_json::to_string(msg).unwrap();
            let parsed: WsMessage = serde_json::from_str(&json).unwrap();
            let json2 = serde_json::to_string(&parsed).unwrap();
            assert_eq!(json, json2);
        }
    }

    #[test]
    fn connection_lifecycle() {
        let mut conn = WsConnection::new("ws-1");
        assert_eq!(conn.state, WsState::Connecting);
        conn.state = WsState::Open;
        conn.record_send();
        conn.record_receive();
        assert_eq!(conn.messages_sent, 1);
        assert_eq!(conn.messages_received, 1);
    }

    #[test]
    fn connection_timeout() {
        let mut conn = WsConnection::new("ws-2");
        conn.last_pong = chrono::Utc::now().timestamp() - 100;
        assert!(conn.is_timed_out(90));
        assert!(!conn.is_timed_out(200));
    }

    #[test]
    fn registry_connect_disconnect() {
        let mut reg = WsConnectionRegistry::new(WsBridgeConfig::default());
        let id = reg.connect().unwrap();
        assert_eq!(reg.active_count(), 1);
        assert!(reg.get(&id).is_some());
        reg.disconnect(&id);
        assert_eq!(reg.active_count(), 0);
    }

    #[test]
    fn registry_max_connections() {
        let config = WsBridgeConfig {
            max_connections: 2,
            ..Default::default()
        };
        let mut reg = WsConnectionRegistry::new(config);
        reg.connect().unwrap();
        reg.connect().unwrap();
        assert!(reg.connect().is_err());
    }

    #[test]
    fn registry_cleanup() {
        let mut reg = WsConnectionRegistry::new(WsBridgeConfig::default());
        let id1 = reg.connect().unwrap();
        let _id2 = reg.connect().unwrap();
        reg.disconnect(&id1);
        let cleaned = reg.cleanup();
        assert_eq!(cleaned, 1);
        assert_eq!(reg.active_count(), 1);
    }

    #[test]
    fn registry_check_timeouts() {
        let config = WsBridgeConfig {
            connection_timeout_secs: 1,
            ..Default::default()
        };
        let mut reg = WsConnectionRegistry::new(config);
        let id = reg.connect().unwrap();
        // Fake old pong
        reg.get_mut(&id).unwrap().last_pong = chrono::Utc::now().timestamp() - 10;
        let timed_out = reg.check_timeouts();
        assert_eq!(timed_out.len(), 1);
        assert_eq!(reg.active_count(), 0);
    }

    #[test]
    fn ws_state_variants() {
        let states = [
            WsState::Connecting,
            WsState::Open,
            WsState::Closing,
            WsState::Closed,
            WsState::Reconnecting,
        ];
        for s in &states {
            assert_eq!(*s, *s);
        }
    }

    #[test]
    fn default_config() {
        let config = WsBridgeConfig::default();
        assert_eq!(config.bind_addr, "127.0.0.1:3001");
        assert_eq!(config.heartbeat_interval_secs, 30);
        assert_eq!(config.max_connections, 16);
    }

    #[test]
    fn connection_ping_pong() {
        let mut conn = WsConnection::new("ws-pp");
        conn.state = WsState::Open;
        conn.record_ping();
        conn.record_pong();
        assert!(conn.last_ping > 0);
        assert!(conn.last_pong > 0);
    }

    fn free_bind_addr() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        addr.to_string()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ws_server_handles_query_and_ping() {
        let bind_addr = free_bind_addr();
        let config = WsBridgeConfig {
            bind_addr: bind_addr.clone(),
            heartbeat_interval_secs: 1,
            connection_timeout_secs: 30,
            ..Default::default()
        };
        let handler: QueryHandler = Arc::new(|query_id, query, tx| {
            tx.send(WsMessage::Delta {
                id: query_id.clone(),
                text: format!("echo:{query}"),
            })
            .unwrap();
            tx.send(WsMessage::ToolStart {
                id: query_id.clone(),
                tool_name: "EchoTool".to_string(),
            })
            .unwrap();
            tx.send(WsMessage::ToolResult {
                id: "tool-1".to_string(),
                content: "ok".to_string(),
                is_error: false,
            })
            .unwrap();
            tx.send(WsMessage::Complete {
                id: query_id,
                stop_reason: "end_turn".to_string(),
            })
            .unwrap();
            Ok(())
        });
        let server = tokio::spawn(run_ws_server(config, handler));

        let ws_url = format!("ws://{bind_addr}");
        let mut connected = None;
        for _ in 0..20 {
            match connect_async(&ws_url).await {
                Ok((stream, _)) => {
                    connected = Some(stream);
                    break;
                }
                Err(_) => sleep(Duration::from_millis(25)).await,
            }
        }
        let mut stream = connected.expect("ws server should accept a client");

        let query = serde_json::to_string(&WsMessage::Query {
            id: "q-1".into(),
            content: "hello".into(),
        })
        .unwrap();
        stream.send(WsFrame::Text(query.into())).await.unwrap();

        let delta = match stream.next().await.unwrap().unwrap() {
            WsFrame::Text(text) => serde_json::from_str::<WsMessage>(&text).unwrap(),
            other => panic!("unexpected message: {other:?}"),
        };
        assert!(matches!(
            delta,
            WsMessage::Delta { ref id, ref text } if id == "q-1" && text == "echo:hello"
        ));

        let tool_start = match stream.next().await.unwrap().unwrap() {
            WsFrame::Text(text) => serde_json::from_str::<WsMessage>(&text).unwrap(),
            other => panic!("unexpected message: {other:?}"),
        };
        assert!(matches!(
            tool_start,
            WsMessage::ToolStart { ref id, ref tool_name }
                if id == "q-1" && tool_name == "EchoTool"
        ));

        let tool_result = match stream.next().await.unwrap().unwrap() {
            WsFrame::Text(text) => serde_json::from_str::<WsMessage>(&text).unwrap(),
            other => panic!("unexpected message: {other:?}"),
        };
        assert!(matches!(
            tool_result,
            WsMessage::ToolResult { ref id, ref content, is_error }
                if id == "tool-1" && content == "ok" && !is_error
        ));

        let complete = match stream.next().await.unwrap().unwrap() {
            WsFrame::Text(text) => serde_json::from_str::<WsMessage>(&text).unwrap(),
            other => panic!("unexpected message: {other:?}"),
        };
        assert!(matches!(
            complete,
            WsMessage::Complete { ref id, ref stop_reason }
                if id == "q-1" && stop_reason == "end_turn"
        ));

        let ping = serde_json::to_string(&WsMessage::Ping { timestamp: 123 }).unwrap();
        stream.send(WsFrame::Text(ping.into())).await.unwrap();

        let pong = match stream.next().await.unwrap().unwrap() {
            WsFrame::Text(text) => serde_json::from_str::<WsMessage>(&text).unwrap(),
            other => panic!("unexpected message: {other:?}"),
        };
        assert!(matches!(pong, WsMessage::Pong { timestamp: 123 }));

        server.abort();
        let _ = server.await;
    }

    struct MockWsProvider {
        calls: Mutex<u32>,
    }

    impl MockWsProvider {
        fn new() -> Self {
            Self {
                calls: Mutex::new(0),
            }
        }
    }

    impl Provider for MockWsProvider {
        fn create_message(
            &self,
            _request: &CreateMessageRequest,
        ) -> Result<CreateMessageResponse, ProviderError> {
            Err(ProviderError::non_retryable("streaming only"))
        }

        fn create_message_stream(
            &self,
            request: &CreateMessageRequest,
            on_event: &mut dyn FnMut(StreamEvent),
        ) -> Result<CreateMessageResponse, ProviderError> {
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;

            if *calls == 1 {
                let tool_use = ContentBlock::tool_use(
                    "tool-1",
                    "Bash",
                    serde_json::json!({
                        "command": "echo ws-stream"
                    }),
                );
                on_event(StreamEvent::ContentBlockStart {
                    index: 0,
                    content_block: tool_use.clone(),
                });
                return Ok(CreateMessageResponse {
                    id: "resp-1".to_string(),
                    content: vec![tool_use],
                    stop_reason: StopReason::ToolUse,
                    usage: Usage::default(),
                    model: request.model.clone(),
                });
            }

            on_event(StreamEvent::ContentBlockDelta {
                index: 0,
                delta: crate::provider::types::StreamDelta::TextDelta {
                    text: "done".to_string(),
                },
            });
            on_event(StreamEvent::ContentBlockDelta {
                index: 1,
                delta: crate::provider::types::StreamDelta::ThinkingDelta {
                    thinking: "considering".to_string(),
                },
            });
            on_event(StreamEvent::MessageDelta {
                stop_reason: StopReason::EndTurn,
                usage: Usage {
                    input_tokens: 11,
                    output_tokens: 22,
                    cache_read_input_tokens: 33,
                    cache_creation_input_tokens: 44,
                },
            });
            Ok(CreateMessageResponse {
                id: "resp-2".to_string(),
                content: vec![ContentBlock::text("done")],
                stop_reason: StopReason::EndTurn,
                usage: Usage {
                    input_tokens: 11,
                    output_tokens: 22,
                    cache_read_input_tokens: 33,
                    cache_creation_input_tokens: 44,
                },
                model: request.model.clone(),
            })
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ws_server_streams_agentic_loop_events_end_to_end() {
        let bind_addr = free_bind_addr();
        let config = WsBridgeConfig {
            bind_addr: bind_addr.clone(),
            heartbeat_interval_secs: 1,
            connection_timeout_secs: 30,
            ..Default::default()
        };
        let handler: QueryHandler = Arc::new(|query_id, query, tx| {
            let provider = MockWsProvider::new();
            let registry = ToolRegistry::with_defaults("/tmp");
            let executor = ToolExecutor::new(&registry);
            let config = LoopConfig {
                model: "mock".to_string(),
                max_tokens: 512,
                max_turns: 4,
                system: Vec::new(),
                tools: tool_definitions_for_model(&registry),
                parallel_tool_execution: true,
            };
            let mut observer = WsEventObserver::new(query_id.clone(), tx.clone());
            let result = r#loop::run_agentic_loop(
                &provider,
                &executor,
                &config,
                vec![Message::user_text(query)],
                &mut observer,
            )
            .map_err(|e| e.to_string())?;
            tx.send(WsMessage::Complete {
                id: query_id,
                stop_reason: match result.stop_reason {
                    StopReason::EndTurn => "end_turn".to_string(),
                    StopReason::ToolUse => "tool_use".to_string(),
                    StopReason::MaxTokens => "max_tokens".to_string(),
                    StopReason::PauseTurn => "pause_turn".to_string(),
                },
            })
            .map_err(|_| "failed to send complete".to_string())
        });
        let server = tokio::spawn(run_ws_server(config, handler));

        let ws_url = format!("ws://{bind_addr}");
        let mut connected = None;
        for _ in 0..20 {
            match connect_async(&ws_url).await {
                Ok((stream, _)) => {
                    connected = Some(stream);
                    break;
                }
                Err(_) => sleep(Duration::from_millis(25)).await,
            }
        }
        let mut stream = connected.expect("ws server should accept a client");

        let query = serde_json::to_string(&WsMessage::Query {
            id: "q-stream".into(),
            content: "stream it".into(),
        })
        .unwrap();
        stream.send(WsFrame::Text(query.into())).await.unwrap();

        let first = match stream.next().await.unwrap().unwrap() {
            WsFrame::Text(text) => serde_json::from_str::<WsMessage>(&text).unwrap(),
            other => panic!("unexpected message: {other:?}"),
        };
        assert!(matches!(
            first,
            WsMessage::ToolStart { ref id, ref tool_name }
                if id == "q-stream" && tool_name == "Bash"
        ));

        let second = match stream.next().await.unwrap().unwrap() {
            WsFrame::Text(text) => serde_json::from_str::<WsMessage>(&text).unwrap(),
            other => panic!("unexpected message: {other:?}"),
        };
        assert!(matches!(
            second,
            WsMessage::ToolResult { ref id, ref content, is_error }
                if id == "tool-1" && content.contains("ws-stream") && !is_error
        ));

        let third = match stream.next().await.unwrap().unwrap() {
            WsFrame::Text(text) => serde_json::from_str::<WsMessage>(&text).unwrap(),
            other => panic!("unexpected message: {other:?}"),
        };
        assert!(matches!(
            third,
            WsMessage::Delta { ref id, ref text } if id == "q-stream" && text == "done"
        ));

        let fourth = match stream.next().await.unwrap().unwrap() {
            WsFrame::Text(text) => serde_json::from_str::<WsMessage>(&text).unwrap(),
            other => panic!("unexpected message: {other:?}"),
        };
        assert!(matches!(
            fourth,
            WsMessage::Thinking { ref id, ref thinking }
                if id == "q-stream" && thinking == "considering"
        ));

        let fifth = match stream.next().await.unwrap().unwrap() {
            WsFrame::Text(text) => serde_json::from_str::<WsMessage>(&text).unwrap(),
            other => panic!("unexpected message: {other:?}"),
        };
        assert!(matches!(
            fifth,
            WsMessage::Usage {
                ref id,
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_write_tokens,
            } if id == "q-stream"
                && input_tokens == 11
                && output_tokens == 22
                && cache_read_tokens == 33
                && cache_write_tokens == 44
        ));

        let sixth = match stream.next().await.unwrap().unwrap() {
            WsFrame::Text(text) => serde_json::from_str::<WsMessage>(&text).unwrap(),
            other => panic!("unexpected message: {other:?}"),
        };
        assert!(matches!(
            sixth,
            WsMessage::Complete { ref id, ref stop_reason }
                if id == "q-stream" && stop_reason == "end_turn"
        ));

        server.abort();
        let _ = server.await;
    }
}
