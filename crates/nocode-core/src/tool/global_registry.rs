//! Global tool registry — unified registry for base, MCP, and plugin tools.
//!
//! Singleton that merges built-in tools, MCP-bridged tools, and plugin tools
//! into a single namespace. Tools are prefixed by source:
//! - Base tools: no prefix (e.g. "Bash", "Read")
//! - MCP tools: "mcp:{server}:{tool}" (e.g. "mcp:github:search_repos")
//! - Plugin tools: "plugin:{name}:{tool}"

use crate::config::settings::Settings;
use crate::mcp::manager::global_mcp_manager;
use crate::provider::types::ToolDefinition;
use crate::tool::{Tool, ToolOutput, ToolRegistry};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// Handler for bridged (non-native) tools.
pub type BridgedHandler = Box<dyn Fn(&serde_json::Value) -> ToolOutput + Send + Sync>;

/// A bridged tool entry (MCP or plugin).
struct BridgedTool {
    definition: ToolDefinition,
    handler: BridgedHandler,
}

/// Unified tool registry — wraps base ToolRegistry + bridged tools.
pub struct GlobalToolRegistry {
    base: ToolRegistry,
    bridged: HashMap<String, BridgedTool>,
}

impl GlobalToolRegistry {
    /// Create from a base ToolRegistry.
    pub fn new(base: ToolRegistry) -> Self {
        Self {
            base,
            bridged: HashMap::new(),
        }
    }

    /// Register a bridged tool (MCP or plugin).
    pub fn register_bridged(
        &mut self,
        name: impl Into<String>,
        definition: ToolDefinition,
        handler: BridgedHandler,
    ) {
        let name = name.into();
        self.bridged.insert(
            name,
            BridgedTool {
                definition,
                handler,
            },
        );
    }

    /// Remove a bridged tool.
    pub fn remove_bridged(&mut self, name: &str) -> bool {
        self.bridged.remove(name).is_some()
    }

    /// Remove all bridged tools with the given prefix.
    pub fn remove_bridged_with_prefix(&mut self, prefix: &str) -> usize {
        let before = self.bridged.len();
        self.bridged.retain(|name, _| !name.starts_with(prefix));
        before - self.bridged.len()
    }

    /// Look up a tool by name — checks base first, then bridged.
    pub fn get_native(&self, name: &str) -> Option<&dyn Tool> {
        self.base.get(name)
    }

    /// Execute a tool by name — base or bridged.
    pub fn execute(&self, name: &str, input: &serde_json::Value) -> Option<ToolOutput> {
        if let Some(tool) = self.base.get(name) {
            return Some(tool.execute(input));
        }
        if let Some(bridged) = self.bridged.get(name) {
            return Some((bridged.handler)(input));
        }
        None
    }

    /// Get all tool definitions (base + bridged).
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        let mut defs = self.base.definitions();
        for bt in self.bridged.values() {
            defs.push(bt.definition.clone());
        }
        defs
    }

    /// Get all tool names.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.base.names().into_iter().map(String::from).collect();
        names.extend(self.bridged.keys().cloned());
        names
    }

    /// Check if a tool exists.
    pub fn contains(&self, name: &str) -> bool {
        self.base.get(name).is_some() || self.bridged.contains_key(name)
    }

    /// Number of registered tools.
    pub fn len(&self) -> usize {
        self.base.names().len() + self.bridged.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get a reference to the base registry.
    pub fn base(&self) -> &ToolRegistry {
        &self.base
    }

    /// Get a mutable reference to the base registry.
    pub fn base_mut(&mut self) -> &mut ToolRegistry {
        &mut self.base
    }
}

impl Default for GlobalToolRegistry {
    fn default() -> Self {
        Self::new(ToolRegistry::new())
    }
}

/// Global singleton.
static GLOBAL_TOOL_REGISTRY: OnceLock<Arc<Mutex<GlobalToolRegistry>>> = OnceLock::new();

pub fn global_tool_registry() -> &'static Arc<Mutex<GlobalToolRegistry>> {
    GLOBAL_TOOL_REGISTRY.get_or_init(|| Arc::new(Mutex::new(GlobalToolRegistry::default())))
}

/// Initialize the global tool registry with a base registry.
pub fn init_global_tool_registry(base: ToolRegistry) {
    let global = global_tool_registry();
    let mut guard = global.lock().unwrap_or_else(|e| e.into_inner());
    *guard = GlobalToolRegistry::new(base);
}

fn input_to_string_map(input: &serde_json::Value) -> HashMap<String, String> {
    input
        .as_object()
        .map(|obj| {
            obj.iter()
                .filter_map(|(key, value)| match value {
                    serde_json::Value::String(text) => Some((key.clone(), text.clone())),
                    serde_json::Value::Bool(flag) => Some((key.clone(), flag.to_string())),
                    serde_json::Value::Number(num) => Some((key.clone(), num.to_string())),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn refresh_global_mcp_bridged_tools() {
    let global = global_tool_registry();
    let mut registry = global.lock().unwrap_or_else(|e| e.into_inner());
    registry.remove_bridged_with_prefix("mcp:");

    let manager = global_mcp_manager();
    let manager = manager.lock().unwrap_or_else(|e| e.into_inner());
    for (server_name, tool) in manager.all_tools() {
        let prefixed_name = format!("mcp:{server_name}:{}", tool.name);
        let original_name = tool.name.clone();
        let server_name = server_name.to_string();
        let definition = ToolDefinition {
            name: prefixed_name.clone(),
            description: tool.description.clone(),
            input_schema: tool.input_schema.clone(),
            cache_control: None,
        };
        registry.register_bridged(
            prefixed_name,
            definition,
            Box::new(move |input| {
                let args = input_to_string_map(input);
                let manager = global_mcp_manager();
                let mut manager = manager.lock().unwrap_or_else(|e| e.into_inner());
                match manager.call_tool(&server_name, &original_name, &args) {
                    Ok(result) if result.is_error => ToolOutput::error(result.content),
                    Ok(result) => ToolOutput::success(result.content),
                    Err(err) => ToolOutput::error(err),
                }
            }),
        );
    }
}

pub fn initialize_runtime_global_registry(cwd: &str, settings: &Settings) {
    init_global_tool_registry(ToolRegistry::with_defaults(cwd));

    let manager = global_mcp_manager();
    let mut manager = manager.lock().unwrap_or_else(|e| e.into_inner());
    manager.sync_from_settings(&settings.mcp_servers);
    let _ = manager.connect_all();
    drop(manager);

    refresh_global_mcp_bridged_tools();
}

pub fn tool_definitions_for_model(registry: &ToolRegistry) -> Vec<ToolDefinition> {
    let global = global_tool_registry();
    let guard = global.lock().unwrap_or_else(|e| e.into_inner());
    if !guard.is_empty() {
        return guard.definitions();
    }
    registry.definitions()
}

pub fn tool_names_for_display(registry: &ToolRegistry) -> Vec<String> {
    let global = global_tool_registry();
    let guard = global.lock().unwrap_or_else(|e| e.into_inner());
    if !guard.is_empty() {
        return guard.names();
    }
    registry.names().into_iter().map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::runtime::McpServerConfig;
    use crate::mcp::manager::global_mcp_manager;
    use serde_json::json;
    use std::sync::{Mutex, OnceLock};

    fn test_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn new_registry_has_no_bridged() {
        let reg = GlobalToolRegistry::new(ToolRegistry::new());
        assert!(reg.bridged.is_empty());
    }

    #[test]
    fn register_and_execute_bridged() {
        let _guard = test_lock();
        let mut reg = GlobalToolRegistry::new(ToolRegistry::new());
        let def = ToolDefinition {
            name: "mcp:test:echo".to_string(),
            description: "Echo tool".to_string(),
            input_schema: json!({"type": "object"}),
            cache_control: None,
        };
        reg.register_bridged(
            "mcp:test:echo",
            def,
            Box::new(|input| {
                let text = input["text"].as_str().unwrap_or("no text");
                ToolOutput::success(text)
            }),
        );

        assert!(reg.contains("mcp:test:echo"));
        let result = reg.execute("mcp:test:echo", &json!({"text": "hello"}));
        assert!(result.is_some());
        assert_eq!(result.unwrap().content, "hello");
    }

    #[test]
    fn definitions_includes_bridged() {
        let _guard = test_lock();
        let mut reg = GlobalToolRegistry::new(ToolRegistry::new());
        let def = ToolDefinition {
            name: "plugin:foo:bar".to_string(),
            description: "Foo bar".to_string(),
            input_schema: json!({"type": "object"}),
            cache_control: None,
        };
        reg.register_bridged(
            "plugin:foo:bar",
            def,
            Box::new(|_| ToolOutput::success("ok")),
        );

        let defs = reg.definitions();
        assert!(defs.iter().any(|d| d.name == "plugin:foo:bar"));
    }

    #[test]
    fn remove_bridged_works() {
        let _guard = test_lock();
        let mut reg = GlobalToolRegistry::new(ToolRegistry::new());
        let def = ToolDefinition {
            name: "temp".to_string(),
            description: "Temp".to_string(),
            input_schema: json!({"type": "object"}),
            cache_control: None,
        };
        reg.register_bridged("temp", def, Box::new(|_| ToolOutput::success("ok")));
        assert!(reg.contains("temp"));
        assert!(reg.remove_bridged("temp"));
        assert!(!reg.contains("temp"));
    }

    #[test]
    fn unknown_tool_returns_none() {
        let reg = GlobalToolRegistry::new(ToolRegistry::new());
        assert!(reg.execute("nonexistent", &json!({})).is_none());
    }

    #[test]
    fn base_tools_accessible() {
        let base = ToolRegistry::with_defaults("/tmp");
        let reg = GlobalToolRegistry::new(base);
        assert!(reg.contains("Bash"));
        assert!(reg.contains("FileRead"));
        assert!(reg.len() >= 21);
    }

    #[test]
    fn remove_bridged_with_prefix_works() {
        let _guard = test_lock();
        let mut reg = GlobalToolRegistry::new(ToolRegistry::new());
        let def = ToolDefinition {
            name: "mcp:test:echo".to_string(),
            description: "Echo".to_string(),
            input_schema: json!({"type": "object"}),
            cache_control: None,
        };
        reg.register_bridged(
            "mcp:test:echo",
            def.clone(),
            Box::new(|_| ToolOutput::success("ok")),
        );
        reg.register_bridged(
            "plugin:test:echo",
            def,
            Box::new(|_| ToolOutput::success("ok")),
        );

        assert_eq!(reg.remove_bridged_with_prefix("mcp:"), 1);
        assert!(!reg.contains("mcp:test:echo"));
        assert!(reg.contains("plugin:test:echo"));
    }

    #[test]
    fn tool_names_fallback_to_base_registry() {
        let _guard = test_lock();
        let reg = ToolRegistry::with_defaults("/tmp");
        let names = tool_names_for_display(&reg);
        assert!(names.iter().any(|name| name == "Bash"));
    }

    #[test]
    fn initialize_runtime_global_registry_sets_base_tools() {
        let _guard = test_lock();
        let settings = Settings::default();
        initialize_runtime_global_registry("/tmp", &settings);
        let reg = ToolRegistry::with_defaults("/tmp");
        let defs = tool_definitions_for_model(&reg);
        assert!(defs.iter().any(|def| def.name == "Bash"));
    }

    #[test]
    fn initialize_runtime_global_registry_syncs_mcp_settings() {
        let _guard = test_lock();
        let mut settings = Settings::default();
        settings.mcp_servers.insert(
            "bad".to_string(),
            McpServerConfig {
                command: "__nonexistent_cmd__".to_string(),
                args: Vec::new(),
                env: HashMap::new(),
            },
        );
        initialize_runtime_global_registry("/tmp", &settings);

        let manager = global_mcp_manager();
        let manager = manager.lock().unwrap_or_else(|e| e.into_inner());
        assert!(manager.get_entry("bad").is_some());
    }
}
