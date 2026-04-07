//! MCP Manager — lifecycle management, health checks, auto-connect.

use crate::mcp::client::{McpClient, McpTool};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// The 11 phases of an MCP server lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpPhase {
    Registered,
    Spawning,
    Handshake,
    ToolDiscovery,
    Connected,
    HealthCheck,
    Degraded,
    Reconnecting,
    Shutdown,
}

impl McpPhase {
    fn can_transition_to(self, target: McpPhase) -> bool {
        use McpPhase::*;
        matches!(
            (self, target),
            (Registered, Spawning)
                | (Spawning, Handshake)
                | (Spawning, Shutdown)
                | (Handshake, ToolDiscovery)
                | (Handshake, Shutdown)
                | (ToolDiscovery, Connected)
                | (ToolDiscovery, Shutdown)
                | (Connected, HealthCheck)
                | (Connected, Degraded)
                | (Connected, Shutdown)
                | (HealthCheck, Connected)
                | (HealthCheck, Degraded)
                | (HealthCheck, Shutdown)
                | (Degraded, Reconnecting)
                | (Degraded, Shutdown)
                | (Reconnecting, Spawning)
                | (Reconnecting, Shutdown)
                | (Shutdown, Registered)
        )
    }
}

/// Entry tracking a registered MCP server.
pub struct McpServerEntry {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub phase: McpPhase,
    pub client: Option<McpClient>,
    pub tools: Vec<McpTool>,
    pub health_failures: u32,
}

impl McpServerEntry {
    fn new(name: &str, command: &str, args: Vec<String>) -> Self {
        Self {
            name: name.to_string(),
            command: command.to_string(),
            args,
            phase: McpPhase::Registered,
            client: None,
            tools: Vec::new(),
            health_failures: 0,
        }
    }

    fn transition(&mut self, target: McpPhase) -> Result<(), String> {
        if !self.phase.can_transition_to(target) {
            return Err(format!(
                "Invalid MCP transition: {:?} → {:?} for '{}'",
                self.phase, target, self.name
            ));
        }
        self.phase = target;
        Ok(())
    }
}

/// Manages all MCP server connections.
pub struct McpManager {
    servers: HashMap<String, McpServerEntry>,
}

impl McpManager {
    pub fn new() -> Self {
        Self {
            servers: HashMap::new(),
        }
    }

    /// Register a server (does not connect yet).
    pub fn register_server(&mut self, name: &str, command: &str, args: Vec<String>) {
        self.servers
            .insert(name.to_string(), McpServerEntry::new(name, command, args));
    }

    /// Connect to a registered server — spawn, handshake, discover tools.
    pub fn connect(&mut self, name: &str) -> Result<(), String> {
        let entry = self
            .servers
            .get_mut(name)
            .ok_or_else(|| format!("MCP server '{name}' not registered"))?;

        entry.transition(McpPhase::Spawning)?;

        let args_refs: Vec<&str> = entry.args.iter().map(String::as_str).collect();
        let mut client = match McpClient::spawn(&entry.command, &args_refs) {
            Ok(c) => c,
            Err(e) => {
                entry.phase = McpPhase::Shutdown;
                return Err(format!("Failed to spawn '{name}': {e}"));
            }
        };

        entry.transition(McpPhase::Handshake)?;
        entry.transition(McpPhase::ToolDiscovery)?;

        match client.list_tools() {
            Ok(tools) => {
                entry.tools = tools;
                entry.client = Some(client);
                entry.transition(McpPhase::Connected)?;
                Ok(())
            }
            Err(e) => {
                entry.phase = McpPhase::Shutdown;
                Err(format!("Tool discovery failed for '{name}': {e}"))
            }
        }
    }

    /// Disconnect a server.
    pub fn disconnect(&mut self, name: &str) -> Result<(), String> {
        let entry = self
            .servers
            .get_mut(name)
            .ok_or_else(|| format!("MCP server '{name}' not registered"))?;
        entry.client = None;
        entry.tools.clear();
        entry.phase = McpPhase::Shutdown;
        Ok(())
    }

    /// Health check a connected server.
    pub fn health_check(&mut self, name: &str) -> Result<bool, String> {
        let entry = self
            .servers
            .get_mut(name)
            .ok_or_else(|| format!("MCP server '{name}' not registered"))?;

        if entry.phase != McpPhase::Connected {
            return Ok(false);
        }

        entry.transition(McpPhase::HealthCheck)?;

        // Try listing tools as a health probe
        let healthy = entry
            .client
            .as_mut()
            .is_some_and(|c| c.list_tools().is_ok());

        if healthy {
            entry.health_failures = 0;
            entry.transition(McpPhase::Connected)?;
        } else {
            entry.health_failures += 1;
            if entry.health_failures >= 3 {
                entry.phase = McpPhase::Degraded;
            } else {
                entry.transition(McpPhase::Connected)?;
            }
        }

        Ok(healthy)
    }

    /// Reconnect a degraded server.
    pub fn reconnect(&mut self, name: &str) -> Result<(), String> {
        let entry = self
            .servers
            .get_mut(name)
            .ok_or_else(|| format!("MCP server '{name}' not registered"))?;

        if entry.phase != McpPhase::Degraded {
            return Err(format!("Server '{name}' is not degraded"));
        }

        entry.transition(McpPhase::Reconnecting)?;
        entry.client = None;
        entry.tools.clear();
        entry.health_failures = 0;

        // Re-spawn
        entry.transition(McpPhase::Spawning)?;
        let args_refs: Vec<&str> = entry.args.iter().map(String::as_str).collect();
        match McpClient::spawn(&entry.command, &args_refs) {
            Ok(mut client) => {
                entry.transition(McpPhase::Handshake)?;
                entry.transition(McpPhase::ToolDiscovery)?;
                match client.list_tools() {
                    Ok(tools) => {
                        entry.tools = tools;
                        entry.client = Some(client);
                        entry.transition(McpPhase::Connected)?;
                        Ok(())
                    }
                    Err(e) => {
                        entry.phase = McpPhase::Shutdown;
                        Err(format!("Reconnect tool discovery failed: {e}"))
                    }
                }
            }
            Err(e) => {
                entry.phase = McpPhase::Shutdown;
                Err(format!("Reconnect spawn failed: {e}"))
            }
        }
    }

    /// List all registered servers with their status.
    pub fn list_servers(&self) -> Vec<(&str, McpPhase, usize)> {
        self.servers
            .values()
            .map(|e| (e.name.as_str(), e.phase, e.tools.len()))
            .collect()
    }

    /// Get all discovered tools across all connected servers.
    pub fn all_tools(&self) -> Vec<(&str, &McpTool)> {
        self.servers
            .values()
            .filter(|e| e.phase == McpPhase::Connected)
            .flat_map(|e| e.tools.iter().map(move |t| (e.name.as_str(), t)))
            .collect()
    }

    /// Connect all registered servers.
    pub fn connect_all(&mut self) -> Vec<(String, Result<(), String>)> {
        let names: Vec<String> = self.servers.keys().cloned().collect();
        names
            .into_iter()
            .map(|name| {
                let result = self.connect(&name);
                (name, result)
            })
            .collect()
    }
}

impl Default for McpManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Global singleton MCP manager.
static GLOBAL_MCP_MANAGER: OnceLock<Arc<Mutex<McpManager>>> = OnceLock::new();

pub fn global_mcp_manager() -> &'static Arc<Mutex<McpManager>> {
    GLOBAL_MCP_MANAGER.get_or_init(|| Arc::new(Mutex::new(McpManager::new())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_list() {
        let mut mgr = McpManager::new();
        mgr.register_server("test", "echo", vec!["hello".to_string()]);
        let servers = mgr.list_servers();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].0, "test");
        assert_eq!(servers[0].1, McpPhase::Registered);
    }

    #[test]
    fn connect_nonexistent_fails() {
        let mut mgr = McpManager::new();
        assert!(mgr.connect("ghost").is_err());
    }

    #[test]
    fn connect_bad_command_fails() {
        let mut mgr = McpManager::new();
        mgr.register_server("bad", "__nonexistent_cmd__", vec![]);
        assert!(mgr.connect("bad").is_err());
    }

    #[test]
    fn phase_transitions() {
        assert!(McpPhase::Registered.can_transition_to(McpPhase::Spawning));
        assert!(!McpPhase::Registered.can_transition_to(McpPhase::Connected));
        assert!(McpPhase::Connected.can_transition_to(McpPhase::HealthCheck));
        assert!(McpPhase::Degraded.can_transition_to(McpPhase::Reconnecting));
    }

    #[test]
    fn disconnect_resets_state() {
        let mut mgr = McpManager::new();
        mgr.register_server("srv", "echo", vec![]);
        mgr.disconnect("srv").unwrap();
        let servers = mgr.list_servers();
        assert_eq!(servers[0].1, McpPhase::Shutdown);
    }
}
