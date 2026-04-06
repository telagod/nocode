use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::Value;

use crate::mcp_client::McpClient;

/// Status of an MCP server connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpServerStatus {
    Disconnected,
    Connecting,
    Connected,
    Failed,
    /// Server was connected but health check detected it is unresponsive.
    Unhealthy,
}

/// Health check statistics for an MCP server.
#[derive(Debug, Clone, Default)]
pub struct McpHealthStats {
    pub checks_total: u64,
    pub checks_passed: u64,
    pub checks_failed: u64,
    pub last_check_ms: Option<u64>,
    pub last_healthy_ms: Option<u64>,
    pub consecutive_failures: u32,
}

/// A tool discovered from an MCP server during connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpDiscoveredTool {
    pub name: String,
    pub description: String,
    pub server_name: String,
}

/// Entry tracking a registered MCP server and its state.
pub struct McpServerEntry {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub status: McpServerStatus,
    pub tools: Vec<McpDiscoveredTool>,
    pub error: Option<String>,
    pub health: McpHealthStats,
    client: Option<McpClient>,
}

impl fmt::Debug for McpServerEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpServerEntry")
            .field("name", &self.name)
            .field("command", &self.command)
            .field("args", &self.args)
            .field("status", &self.status)
            .field("tools", &self.tools)
            .field("error", &self.error)
            .field("health", &self.health)
            .field("client", &self.client.as_ref().map(|_| "<McpClient>"))
            .finish()
    }
}

/// Manages MCP server lifecycles and tool routing.
pub struct McpManager {
    servers: HashMap<String, McpServerEntry>,
}

impl McpManager {
    pub fn new() -> Self {
        Self {
            servers: HashMap::new(),
        }
    }

    /// Register a server configuration without connecting.
    pub fn register_server(&mut self, name: &str, command: &str, args: Vec<String>) {
        self.servers.insert(
            name.to_string(),
            McpServerEntry {
                name: name.to_string(),
                command: command.to_string(),
                args,
                status: McpServerStatus::Disconnected,
                tools: Vec::new(),
                error: None,
                health: McpHealthStats::default(),
                client: None,
            },
        );
    }

    /// Connect to a registered server and discover its tools.
    ///
    /// Spawns the MCP server process, performs the JSON-RPC initialize
    /// handshake, and discovers available tools via `tools/list`.
    pub fn connect(&mut self, name: &str) -> Result<Vec<McpDiscoveredTool>, String> {
        let entry = self
            .servers
            .get_mut(name)
            .ok_or_else(|| format!("MCP server not registered: {name}"))?;

        entry.status = McpServerStatus::Connecting;

        // Convert Vec<String> args to &[&str] for McpClient::spawn.
        let arg_refs: Vec<&str> = entry.args.iter().map(String::as_str).collect();

        // Spawn the MCP server process (initialize handshake happens inside spawn).
        let mut client = match McpClient::spawn(&entry.command, &arg_refs) {
            Ok(c) => c,
            Err(e) => {
                entry.status = McpServerStatus::Failed;
                entry.error = Some(e.clone());
                return Err(e);
            }
        };

        // Discover tools.
        let tools = match client.list_tools() {
            Ok(t) => t,
            Err(e) => {
                entry.status = McpServerStatus::Failed;
                entry.error = Some(e.clone());
                return Err(e);
            }
        };

        // Convert to McpDiscoveredTool.
        let discovered: Vec<McpDiscoveredTool> = tools
            .iter()
            .map(|t| McpDiscoveredTool {
                name: t.name.clone(),
                description: t.description.clone(),
                server_name: name.to_string(),
            })
            .collect();

        entry.tools = discovered.clone();
        entry.client = Some(client);
        entry.status = McpServerStatus::Connected;
        entry.error = None;

        Ok(discovered)
    }

    /// Disconnect a server, clearing its tools and resetting status.
    pub fn disconnect(&mut self, name: &str) {
        if let Some(entry) = self.servers.get_mut(name) {
            entry.client = None; // Drop closes the child process.
            entry.status = McpServerStatus::Disconnected;
            entry.tools.clear();
            entry.error = None;
        }
    }

    /// Get the current status of a registered server.
    pub fn get_status(&self, name: &str) -> Option<McpServerStatus> {
        self.servers.get(name).map(|e| e.status)
    }

    /// List all registered servers.
    pub fn list_servers(&self) -> Vec<&McpServerEntry> {
        self.servers.values().collect()
    }

    /// Find a discovered tool by name across all connected servers.
    pub fn find_tool(&self, tool_name: &str) -> Option<&McpDiscoveredTool> {
        self.servers
            .values()
            .filter(|e| e.status == McpServerStatus::Connected)
            .flat_map(|e| e.tools.iter())
            .find(|t| t.name == tool_name)
    }

    /// Call a tool on its owning server via the real MCP client.
    ///
    /// Routes the call to the connected server that owns the tool.
    /// Returns the tool result content or an error.
    pub fn call_tool(&mut self, tool_name: &str, arguments: Value) -> Result<String, String> {
        // Find which server owns this tool.
        let server_name = self
            .servers
            .values()
            .filter(|e| e.status == McpServerStatus::Connected)
            .find(|e| e.tools.iter().any(|t| t.name == tool_name))
            .map(|e| e.name.clone())
            .ok_or_else(|| format!("MCP tool not found: {tool_name}"))?;

        let entry = self
            .servers
            .get_mut(&server_name)
            .ok_or_else(|| format!("MCP server '{server_name}' not registered"))?;

        let client = entry
            .client
            .as_mut()
            .ok_or_else(|| format!("MCP server '{server_name}' has no active client"))?;

        // Convert Value arguments to HashMap<String, String> for McpClient.
        let args_map: HashMap<String, String> = match arguments.as_object() {
            Some(obj) => obj
                .iter()
                .map(|(k, v)| {
                    let val = match v {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    (k.clone(), val)
                })
                .collect(),
            None => HashMap::new(),
        };

        let result = client.call_tool(tool_name, &args_map)?;
        if result.is_error {
            return Err(format!("MCP tool error: {}", result.content));
        }
        Ok(result.content)
    }

    // -----------------------------------------------------------------------
    // Health check & reconnection
    // -----------------------------------------------------------------------

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    /// Maximum consecutive health check failures before marking Unhealthy.
    const MAX_CONSECUTIVE_FAILURES: u32 = 3;

    /// Run a health check on a connected server by calling `tools/list`.
    /// Updates health stats and transitions to Unhealthy if needed.
    pub fn health_check(&mut self, name: &str) -> Result<bool, String> {
        let entry = self
            .servers
            .get_mut(name)
            .ok_or_else(|| format!("MCP server not registered: {name}"))?;

        if entry.status != McpServerStatus::Connected
            && entry.status != McpServerStatus::Unhealthy
        {
            return Err(format!(
                "server '{name}' is {:?}, cannot health-check",
                entry.status
            ));
        }

        let now = Self::now_ms();
        entry.health.checks_total += 1;
        entry.health.last_check_ms = Some(now);

        let client = match entry.client.as_mut() {
            Some(c) => c,
            None => {
                entry.health.checks_failed += 1;
                entry.health.consecutive_failures += 1;
                if entry.health.consecutive_failures >= Self::MAX_CONSECUTIVE_FAILURES {
                    entry.status = McpServerStatus::Unhealthy;
                    entry.error = Some("no client handle".to_string());
                }
                return Ok(false);
            }
        };

        match client.list_tools() {
            Ok(_) => {
                entry.health.checks_passed += 1;
                entry.health.consecutive_failures = 0;
                entry.health.last_healthy_ms = Some(now);
                // Recover from Unhealthy if check passes.
                if entry.status == McpServerStatus::Unhealthy {
                    entry.status = McpServerStatus::Connected;
                    entry.error = None;
                }
                Ok(true)
            }
            Err(e) => {
                entry.health.checks_failed += 1;
                entry.health.consecutive_failures += 1;
                if entry.health.consecutive_failures >= Self::MAX_CONSECUTIVE_FAILURES {
                    entry.status = McpServerStatus::Unhealthy;
                    entry.error = Some(e);
                }
                Ok(false)
            }
        }
    }

    /// Run health checks on all connected/unhealthy servers.
    /// Returns a map of server name → healthy (true/false).
    pub fn health_check_all(&mut self) -> HashMap<String, bool> {
        let names: Vec<String> = self
            .servers
            .values()
            .filter(|e| {
                e.status == McpServerStatus::Connected
                    || e.status == McpServerStatus::Unhealthy
            })
            .map(|e| e.name.clone())
            .collect();

        let mut results = HashMap::new();
        for name in names {
            let healthy = self.health_check(&name).unwrap_or(false);
            results.insert(name, healthy);
        }
        results
    }

    /// Attempt to reconnect an unhealthy or failed server.
    /// Disconnects first, then reconnects.
    pub fn reconnect(&mut self, name: &str) -> Result<Vec<McpDiscoveredTool>, String> {
        let status = self
            .get_status(name)
            .ok_or_else(|| format!("MCP server not registered: {name}"))?;

        match status {
            McpServerStatus::Unhealthy | McpServerStatus::Failed => {
                self.disconnect(name);
                self.connect(name)
            }
            McpServerStatus::Disconnected => self.connect(name),
            McpServerStatus::Connected => {
                Err(format!("server '{name}' is already connected"))
            }
            McpServerStatus::Connecting => {
                Err(format!("server '{name}' is currently connecting"))
            }
        }
    }

    /// Get health stats for a server.
    pub fn get_health(&self, name: &str) -> Option<&McpHealthStats> {
        self.servers.get(name).map(|e| &e.health)
    }
}

impl Default for McpManager {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Global singleton
// ---------------------------------------------------------------------------

static MCP_MANAGER: OnceLock<Arc<Mutex<McpManager>>> = OnceLock::new();

/// Return the process-wide `McpManager` singleton.
pub fn global_mcp_manager() -> Arc<Mutex<McpManager>> {
    MCP_MANAGER
        .get_or_init(|| Arc::new(Mutex::new(McpManager::new())))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_tool(server: &str, name: &str) -> McpDiscoveredTool {
        McpDiscoveredTool {
            name: name.to_string(),
            description: format!("{name} on {server}"),
            server_name: server.to_string(),
        }
    }

    #[test]
    fn register_and_connect_nonexistent_binary_fails() {
        let mut mgr = McpManager::new();
        mgr.register_server(
            "bad",
            "__nonexistent_mcp_binary_99999__",
            vec![],
        );

        assert_eq!(mgr.get_status("bad"), Some(McpServerStatus::Disconnected));

        let err = mgr.connect("bad").unwrap_err();
        assert!(err.contains("failed to spawn"), "unexpected error: {err}");
        assert_eq!(mgr.get_status("bad"), Some(McpServerStatus::Failed));
        assert!(mgr.servers.get("bad").unwrap().error.is_some());
    }

    #[test]
    fn connect_unregistered_server_fails() {
        let mut mgr = McpManager::new();
        let err = mgr.connect("ghost").unwrap_err();
        assert!(err.contains("not registered"));
    }

    #[test]
    fn find_tool_on_connected_server() {
        let mut mgr = McpManager::new();
        mgr.register_server("fs", "mcp-fs", vec![]);

        // Manually inject a tool (simulating discovery).
        mgr.servers
            .get_mut("fs")
            .unwrap()
            .tools
            .push(make_tool("fs", "read_file"));
        mgr.servers.get_mut("fs").unwrap().status = McpServerStatus::Connected;

        let found = mgr.find_tool("read_file");
        assert!(found.is_some());
        assert_eq!(found.unwrap().server_name, "fs");

        // Disconnected server tools are invisible.
        mgr.disconnect("fs");
        assert!(mgr.find_tool("read_file").is_none());
    }

    #[test]
    fn call_tool_on_disconnected_fails() {
        let mut mgr = McpManager::new();
        mgr.register_server("fs", "mcp-fs", vec![]);
        mgr.servers
            .get_mut("fs")
            .unwrap()
            .tools
            .push(make_tool("fs", "read_file"));

        // Server is Disconnected — tool lookup won't find it.
        let err = mgr.call_tool("read_file", json!({})).unwrap_err();
        assert!(err.contains("not found"));
    }

    #[test]
    fn call_tool_without_client_fails() {
        let mut mgr = McpManager::new();
        mgr.register_server("fs", "mcp-fs", vec![]);
        mgr.servers
            .get_mut("fs")
            .unwrap()
            .tools
            .push(make_tool("fs", "read_file"));
        // Set Connected but no client — simulates a broken state.
        mgr.servers.get_mut("fs").unwrap().status = McpServerStatus::Connected;

        let err = mgr
            .call_tool("read_file", json!({"path": "/etc/hosts"}))
            .unwrap_err();
        assert!(err.contains("no active client"));
    }

    #[test]
    fn call_tool_unknown_tool_fails() {
        let mut mgr = McpManager::new();
        mgr.register_server("fs", "mcp-fs", vec![]);
        mgr.servers.get_mut("fs").unwrap().status = McpServerStatus::Connected;

        let err = mgr.call_tool("nonexistent_tool", json!({})).unwrap_err();
        assert!(err.contains("not found"));
    }

    #[test]
    fn disconnect_clears_client_and_tools() {
        let mut mgr = McpManager::new();
        mgr.register_server("fs", "mcp-fs", vec![]);
        mgr.servers
            .get_mut("fs")
            .unwrap()
            .tools
            .push(make_tool("fs", "read_file"));
        mgr.servers.get_mut("fs").unwrap().status = McpServerStatus::Connected;

        mgr.disconnect("fs");
        assert_eq!(mgr.get_status("fs"), Some(McpServerStatus::Disconnected));
        assert!(mgr.servers.get("fs").unwrap().tools.is_empty());
        assert!(mgr.servers.get("fs").unwrap().client.is_none());
        assert!(mgr.servers.get("fs").unwrap().error.is_none());
    }

    #[test]
    fn list_servers() {
        let mut mgr = McpManager::new();
        mgr.register_server("a", "cmd-a", vec![]);
        mgr.register_server("b", "cmd-b", vec!["--flag".into()]);

        let servers = mgr.list_servers();
        assert_eq!(servers.len(), 2);
    }

    #[test]
    fn global_singleton() {
        let m1 = global_mcp_manager();
        let m2 = global_mcp_manager();
        // Same Arc — same allocation.
        assert!(Arc::ptr_eq(&m1, &m2));
    }

    #[test]
    fn health_check_no_client_marks_unhealthy() {
        let mut mgr = McpManager::new();
        mgr.register_server("fs", "mcp-fs", vec![]);
        mgr.servers.get_mut("fs").unwrap().status = McpServerStatus::Connected;
        // No client — health check should fail.

        for i in 0..3 {
            let result = mgr.health_check("fs").unwrap();
            assert!(!result);
            let h = mgr.get_health("fs").unwrap();
            assert_eq!(h.consecutive_failures, i + 1);
        }
        // After 3 failures → Unhealthy.
        assert_eq!(mgr.get_status("fs"), Some(McpServerStatus::Unhealthy));
        let h = mgr.get_health("fs").unwrap();
        assert_eq!(h.checks_total, 3);
        assert_eq!(h.checks_failed, 3);
        assert_eq!(h.checks_passed, 0);
        assert!(h.last_check_ms.is_some());
    }

    #[test]
    fn health_check_disconnected_server_errors() {
        let mut mgr = McpManager::new();
        mgr.register_server("fs", "mcp-fs", vec![]);
        let err = mgr.health_check("fs").unwrap_err();
        assert!(err.contains("Disconnected"));
    }

    #[test]
    fn health_check_unregistered_server_errors() {
        let mut mgr = McpManager::new();
        let err = mgr.health_check("ghost").unwrap_err();
        assert!(err.contains("not registered"));
    }

    #[test]
    fn health_check_all_filters_connected() {
        let mut mgr = McpManager::new();
        mgr.register_server("a", "cmd-a", vec![]);
        mgr.register_server("b", "cmd-b", vec![]);
        // Only 'a' is connected.
        mgr.servers.get_mut("a").unwrap().status = McpServerStatus::Connected;
        // 'b' stays Disconnected — should not be checked.

        let results = mgr.health_check_all();
        assert_eq!(results.len(), 1);
        assert!(results.contains_key("a"));
    }

    #[test]
    fn reconnect_failed_server() {
        let mut mgr = McpManager::new();
        mgr.register_server("bad", "__nonexistent_99__", vec![]);
        mgr.servers.get_mut("bad").unwrap().status = McpServerStatus::Failed;

        // Reconnect will disconnect then try connect — which will fail (binary doesn't exist).
        let err = mgr.reconnect("bad").unwrap_err();
        assert!(err.contains("failed to spawn"));
    }

    #[test]
    fn reconnect_connected_server_errors() {
        let mut mgr = McpManager::new();
        mgr.register_server("fs", "mcp-fs", vec![]);
        mgr.servers.get_mut("fs").unwrap().status = McpServerStatus::Connected;

        let err = mgr.reconnect("fs").unwrap_err();
        assert!(err.contains("already connected"));
    }

    #[test]
    fn reconnect_disconnected_server_tries_connect() {
        let mut mgr = McpManager::new();
        mgr.register_server("fs", "__nonexistent_99__", vec![]);
        // Disconnected → reconnect should try connect.
        let err = mgr.reconnect("fs").unwrap_err();
        assert!(err.contains("failed to spawn"));
        assert_eq!(mgr.get_status("fs"), Some(McpServerStatus::Failed));
    }

    #[test]
    fn health_stats_default() {
        let stats = McpHealthStats::default();
        assert_eq!(stats.checks_total, 0);
        assert_eq!(stats.consecutive_failures, 0);
        assert!(stats.last_check_ms.is_none());
    }

    #[test]
    fn get_health_returns_stats() {
        let mut mgr = McpManager::new();
        mgr.register_server("fs", "mcp-fs", vec![]);
        let h = mgr.get_health("fs").unwrap();
        assert_eq!(h.checks_total, 0);
        assert!(mgr.get_health("ghost").is_none());
    }
}
