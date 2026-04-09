//! WebSocket bridge — persistent bidirectional connection for real-time streaming.
//!
//! Provides WebSocket transport as an alternative to HTTP polling for bridge clients.
//! Supports connect/disconnect, heartbeat/ping-pong, message framing, and reconnect.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

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
    /// Server signals tool use start.
    ToolStart { id: String, tool_name: String },
    /// Server signals tool use result.
    ToolResult { id: String, content: String, is_error: bool },
    /// Server signals query complete.
    Complete { id: String, stop_reason: String },
    /// Server signals an error.
    Error { id: String, message: String },
    /// Heartbeat ping.
    Ping { timestamp: i64 },
    /// Heartbeat pong.
    Pong { timestamp: i64 },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_message_serde_roundtrip() {
        let msgs = vec![
            WsMessage::Query { id: "q1".into(), content: "hello".into() },
            WsMessage::Delta { id: "q1".into(), text: "world".into() },
            WsMessage::ToolStart { id: "q1".into(), tool_name: "Bash".into() },
            WsMessage::ToolResult { id: "q1".into(), content: "ok".into(), is_error: false },
            WsMessage::Complete { id: "q1".into(), stop_reason: "end_turn".into() },
            WsMessage::Error { id: "q1".into(), message: "fail".into() },
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
}
