use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::Value;

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
#[derive(Debug)]
pub struct McpServerEntry {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub status: McpServerStatus,
    pub tools: Vec<McpDiscoveredTool>,
    pub error: Option<String>,
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
            },
        );
    }

    /// Connect to a registered server and discover its tools.
    ///
    /// Currently simulates connection (sets status to `Connected`, returns empty
    /// tool list). Real `McpClient::spawn` integration is deferred because
    /// `Child` is not `Send` and cannot live inside a `Mutex` safely.
    pub fn connect(&mut self, name: &str) -> Result<Vec<McpDiscoveredTool>, String> {
        let entry = self
            .servers
            .get_mut(name)
            .ok_or_else(|| format!("MCP server not registered: {name}"))?;

        entry.status = McpServerStatus::Connecting;
        // TODO: real implementation — McpClient::spawn + initialize + list_tools
        entry.status = McpServerStatus::Connected;
        entry.error = None;
        Ok(entry.tools.clone())
    }

    /// Disconnect a server, clearing its tools and resetting status.
    pub fn disconnect(&mut self, name: &str) {
        if let Some(entry) = self.servers.get_mut(name) {
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

    /// Call a tool on its owning server.
    ///
    /// Returns a simulated success response for connected servers.
    /// Returns an error if the server is not connected or the tool is unknown.
    pub fn call_tool(&self, tool_name: &str, arguments: Value) -> Result<String, String> {
        let tool = self
            .find_tool(tool_name)
            .ok_or_else(|| format!("MCP tool not found: {tool_name}"))?;

        let server_name = &tool.server_name;
        let entry = self
            .servers
            .get(server_name)
            .ok_or_else(|| format!("MCP server '{server_name}' not registered"))?;

        match entry.status {
            McpServerStatus::Connected => {
                // TODO: real McpClient::call_tool integration
                Ok(format!(
                    "{{\"result\": \"simulated success\", \"tool\": \"{tool_name}\", \"server\": \"{server_name}\", \"arguments\": {arguments}}}"
                ))
            }
            McpServerStatus::Disconnected => {
                Err(format!("MCP server '{server_name}' is disconnected"))
            }
            McpServerStatus::Connecting => {
                Err(format!("MCP server '{server_name}' is still connecting"))
            }
            McpServerStatus::Failed => {
                let err_msg = entry.error.as_deref().unwrap_or("unknown error");
                Err(format!(
                    "MCP server '{server_name}' is in failed state: {err_msg}"
                ))
            }
        }
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
    fn register_and_connect_server() {
        let mut mgr = McpManager::new();
        mgr.register_server("fs", "mcp-fs", vec!["--root".into(), "/tmp".into()]);

        assert_eq!(mgr.get_status("fs"), Some(McpServerStatus::Disconnected));

        let tools = mgr.connect("fs").unwrap();
        assert!(tools.is_empty()); // simulated — no real tools yet
        assert_eq!(mgr.get_status("fs"), Some(McpServerStatus::Connected));
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

        // Server is Disconnected — find_tool won't find it.
        let err = mgr.call_tool("read_file", json!({})).unwrap_err();
        assert!(err.contains("not found"));
    }

    #[test]
    fn call_tool_on_connected_succeeds() {
        let mut mgr = McpManager::new();
        mgr.register_server("fs", "mcp-fs", vec![]);
        mgr.servers
            .get_mut("fs")
            .unwrap()
            .tools
            .push(make_tool("fs", "read_file"));
        mgr.servers.get_mut("fs").unwrap().status = McpServerStatus::Connected;

        let result = mgr.call_tool("read_file", json!({"path": "/etc/hosts"}));
        assert!(result.is_ok());
        let body = result.unwrap();
        assert!(body.contains("simulated success"));
        assert!(body.contains("read_file"));
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
    fn disconnect_clears_tools_and_status() {
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
    }

    #[test]
    fn global_singleton() {
        let m1 = global_mcp_manager();
        let m2 = global_mcp_manager();
        // Same Arc — same allocation.
        assert!(Arc::ptr_eq(&m1, &m2));
    }
}
