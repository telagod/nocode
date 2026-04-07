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
pub mod grep;
pub mod memory_tools;
pub mod permission;
pub mod read;
pub mod task_tools;
pub mod team_tools;
pub mod tool_validation;
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
}

impl ToolOutput {
    pub fn success(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
        }
    }

    pub fn error(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: true,
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
        }
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

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|t| t.definition()).collect()
    }

    pub fn names(&self) -> Vec<&str> {
        self.tools.keys().map(String::as_str).collect()
    }

    /// Create a registry with all built-in tools.
    pub fn with_defaults(cwd: impl Into<String>) -> Self {
        let cwd = cwd.into();
        let mut registry = Self::new();
        // Core tools
        registry.register(Box::new(bash::BashTool::new(&cwd)));
        registry.register(Box::new(read::ReadTool));
        registry.register(Box::new(write::WriteTool));
        registry.register(Box::new(edit::EditTool));
        registry.register(Box::new(glob::GlobTool));
        registry.register(Box::new(grep::GrepTool));
        registry.register(Box::new(web::WebFetchTool));
        registry.register(Box::new(web::WebSearchTool));
        registry.register(Box::new(agent::AgentTool));
        // Task tools
        registry.register(Box::new(task_tools::TaskGetTool));
        registry.register(Box::new(task_tools::TaskListTool));
        registry.register(Box::new(task_tools::TaskUpdateTool));
        registry.register(Box::new(task_tools::TaskStopTool));
        registry.register(Box::new(task_tools::TaskOutputTool));
        // Team tools
        registry.register(Box::new(team_tools::TeamCreateTool));
        registry.register(Box::new(team_tools::TeamDeleteTool));
        // Cron tools
        registry.register(Box::new(cron_tools::CronCreateTool));
        registry.register(Box::new(cron_tools::CronDeleteTool));
        registry.register(Box::new(cron_tools::CronListTool));
        // Discovery tools
        registry.register(Box::new(discovery_tools::ToolSearchTool));
        registry.register(Box::new(discovery_tools::LspTool));
        // Memory tools
        registry.register(Box::new(memory_tools::MemorySaveTool));
        registry.register(Box::new(memory_tools::MemoryListTool));
        registry.register(Box::new(memory_tools::MemorySearchTool));
        registry.register(Box::new(memory_tools::MemoryDeleteTool));
        registry
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
