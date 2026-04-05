pub mod cron_tools;
mod executor;
pub mod lsp_tools;
pub mod mcp_bridge;
mod model;
pub mod task_tools;
pub mod team_tools;
pub mod tool_search;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolExecutionModule;

impl ToolExecutionModule {
    pub const LABEL: &'static str = "tool-execution";
    pub const TS_SOURCE: &'static str = "src/services/tools/toolExecution.ts";
    pub const RESPONSIBILITY: &'static str =
        "Executes tool calls, permission checks, hook dispatch, and tool-result mapping.";
}

pub use executor::{
    DefaultToolExecutor, LiveToolHost, ToolCommandOutput, ToolExecutionContext, ToolExecutor,
    ToolHost,
};
pub use model::{
    ToolCallArgument, ToolCallInput, ToolCallOutput, ToolCallResult, ToolExecutionRequest,
    ToolExecutionTrace, ToolPermissionDecision, ToolProgressUpdate,
};
pub use tool_search::{DeferredTool, DeferredToolRegistry, ToolSearchResult};
