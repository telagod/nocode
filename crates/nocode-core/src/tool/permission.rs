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

/// Decision returned by a permission prompter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionDecision {
    /// Allow this single call.
    Allow,
    /// Deny this single call.
    Deny,
    /// Always allow this tool (for the rest of the session).
    AlwaysAllow,
}

/// Trait for interactive permission prompting.
/// Implementations block until the user responds.
pub trait PermissionPrompter: Send + Sync {
    fn prompt(&self, tool_name: &str, arguments_summary: &str) -> PermissionDecision;
}

/// Auto-approve prompter (for non-interactive / Auto mode).
pub struct AutoApprovePrompter;

impl PermissionPrompter for AutoApprovePrompter {
    fn prompt(&self, _tool_name: &str, _arguments_summary: &str) -> PermissionDecision {
        PermissionDecision::Allow
    }
}

/// Auto-deny prompter (for Deny mode).
pub struct AutoDenyPrompter;

impl PermissionPrompter for AutoDenyPrompter {
    fn prompt(&self, _tool_name: &str, _arguments_summary: &str) -> PermissionDecision {
        PermissionDecision::Deny
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_approve_always_allows() {
        let p = AutoApprovePrompter;
        assert_eq!(p.prompt("Bash", "cmd=ls"), PermissionDecision::Allow);
    }

    #[test]
    fn auto_deny_always_denies() {
        let p = AutoDenyPrompter;
        assert_eq!(p.prompt("Bash", "cmd=ls"), PermissionDecision::Deny);
    }

    #[test]
    fn default_mode_is_ask() {
        assert_eq!(PermissionMode::default(), PermissionMode::Ask);
    }
}
