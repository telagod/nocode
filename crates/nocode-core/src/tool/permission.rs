/// Permission mode for tool execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionMode {
    /// Auto-approve all tool calls.
    Auto,
    /// Ask user for approval on each tool call.
    Ask,
    /// Deny all tool calls.
    Deny,
}

impl Default for PermissionMode {
    fn default() -> Self {
        Self::Ask
    }
}
