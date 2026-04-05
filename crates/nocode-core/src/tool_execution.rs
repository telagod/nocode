mod executor;
mod model;

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
