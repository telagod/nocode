pub mod agent;
pub mod bash;
pub mod bash_validation;
pub mod cron_tools;
pub mod definitions;
pub mod discovery_tools;
pub mod edit;
pub mod executor;
pub mod file_safety;
pub mod glob;
pub mod global_registry;
pub mod grep;
pub mod hook_runner;
pub mod interactive_tools;
pub mod lsp_registry;
pub mod mcp_tools;
pub mod memory_tools;
pub mod permission;
pub mod plugin_registry;
pub mod read;
pub mod send_message;
pub mod session_tools;
pub mod skill;
pub mod task_tools;
pub mod team_tools;
pub mod todo_write;
pub mod tool_validation;
pub mod trust;
pub mod web;
pub mod write;

use crate::provider::types::ToolDefinition;
use serde_json::Value;
use std::collections::HashMap;

/// Result of executing a tool.
#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
    pub structured_content: Option<Value>,
}

impl ToolOutput {
    pub fn success(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
            structured_content: None,
        }
    }

    pub fn error(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: true,
            structured_content: None,
        }
    }

    pub fn success_with_structured(content: impl Into<String>, structured_content: Value) -> Self {
        Self {
            content: content.into(),
            is_error: false,
            structured_content: Some(structured_content),
        }
    }

    pub fn error_with_structured(content: impl Into<String>, structured_content: Value) -> Self {
        Self {
            content: content.into(),
            is_error: true,
            structured_content: Some(structured_content),
        }
    }
}

/// Trait that all tools must implement.
pub trait Tool: Send + Sync {
    /// Tool name as sent to the API.
    fn name(&self) -> &str;

    /// Tool description for the model.
    fn description(&self) -> &str;

    /// JSON Schema for the tool's input parameters.
    fn input_schema(&self) -> Value;

    /// Execute the tool with the given input.
    fn execute(&self, input: &Value) -> ToolOutput;

    /// Build the `ToolDefinition` for API registration.
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: self.input_schema(),
            cache_control: None,
        }
    }

    /// Support downcasting for tools that need runtime type injection.
    fn as_any(&self) -> &dyn std::any::Any {
        // Default: return a dangling reference — tools must override if they
        // need to support downcasting.
        &()
    }
}

/// Registry of available tools.
pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(AsRef::as_ref)
    }

    /// Get a tool as a concrete type via downcasting.
    /// Returns None if the tool doesn't exist or isn't the requested type.
    pub fn get_as<T: 'static>(&self, name: &str) -> Option<&T> {
        self.tools
            .get(name)
            .and_then(|tool| tool.as_any().downcast_ref::<T>())
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|t| t.definition()).collect()
    }

    pub fn names(&self) -> Vec<&str> {
        self.tools.keys().map(String::as_str).collect()
    }

    /// Create a registry with the core built-in tools (Pi-inspired minimal set).
    pub fn with_defaults(cwd: impl Into<String>) -> Self {
        let cwd = cwd.into();
        let mut registry = Self::new();
        // Core: read, write, search, execute
        registry.register(Box::new(agent::AgentTool));
        registry.register(Box::new(bash::BashTool::new(&cwd)));
        registry.register(Box::new(edit::EditTool));
        registry.register(Box::new(read::ReadTool));
        registry.register(Box::new(write::WriteTool));
        registry.register(Box::new(glob::GlobTool));
        registry.register(Box::new(grep::GrepTool));
        // Extended: web access
        registry.register(Box::new(web::WebFetchTool));
        registry.register(Box::new(web::WebSearchTool));
        registry
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
