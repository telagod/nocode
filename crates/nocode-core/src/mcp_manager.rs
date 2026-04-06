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
}
