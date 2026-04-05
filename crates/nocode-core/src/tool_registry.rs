/// Three-tier permission mode controlling which tools are available.
///
/// Ordering: `ReadOnly < WorkspaceWrite < DangerFullAccess`.
/// A tool whose `required_permission` exceeds the active mode is rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionMode {
    /// Only read-oriented tools (Read, Glob, Grep, WebFetch, WebSearch).
    ReadOnly = 0,
    /// Allows workspace-scoped writes (Edit, Write, Bash, Agent, Task).
    WorkspaceWrite = 1,
    /// No restrictions whatsoever.
    DangerFullAccess = 2,
}

impl Default for PermissionMode {
    fn default() -> Self {
        Self::WorkspaceWrite
    }
}

impl PartialOrd for PermissionMode {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PermissionMode {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (*self as u8).cmp(&(*other as u8))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolRegistryModule;

impl ToolRegistryModule {
    pub const LABEL: &'static str = "tool-registry";
    pub const TS_SOURCE: &'static str = "src/tools.ts";
    pub const RESPONSIBILITY: &'static str =
        "Declares the available tools and applies environment or permission-based filtering.";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    ReadOnly,
    Edit,
    Execution,
    Orchestration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDefinition {
    pub name: String,
    pub kind: ToolKind,
    pub enabled_in_simple_mode: bool,
    pub required_permission: PermissionMode,
}

impl ToolDefinition {
    pub fn new(name: impl Into<String>, kind: ToolKind, enabled_in_simple_mode: bool) -> Self {
        Self {
            name: name.into(),
            kind,
            enabled_in_simple_mode,
            required_permission: PermissionMode::default(),
        }
    }

    pub fn with_permission(mut self, permission: PermissionMode) -> Self {
        self.required_permission = permission;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToolPermissionContext {
    pub blanket_denies: Vec<String>,
    pub rules: Vec<PermissionRule>,
    pub mode: PermissionMode,
}

/// A rule that conditionally denies tool execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRule {
    pub tool_name: String,
    pub condition: PermissionCondition,
    pub reason: String,
}

/// Condition under which a permission rule triggers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionCondition {
    /// Always deny this tool.
    AlwaysDeny,
    /// Deny if a specific argument matches a substring.
    ArgumentContains { arg_name: String, pattern: String },
    /// Deny if the command matches a pattern (for Bash).
    CommandContains(String),
}

impl ToolPermissionContext {
    pub fn deny(mut self, tool_name: impl Into<String>) -> Self {
        self.blanket_denies.push(tool_name.into());
        self
    }

    pub fn with_mode(mut self, mode: PermissionMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn with_rule(mut self, rule: PermissionRule) -> Self {
        self.rules.push(rule);
        self
    }

    pub fn denies(&self, tool_name: &str) -> bool {
        self.blanket_denies.iter().any(|rule| rule == tool_name)
    }

    /// Evaluate rules against a tool call. Returns `Some(reason)` if denied.
    pub fn evaluate(&self, tool_name: &str, arguments: &[(String, String)]) -> Option<String> {
        if self.denies(tool_name) {
            return Some(format!("tool '{tool_name}' is blanket-denied"));
        }
        for rule in &self.rules {
            if rule.tool_name != "*" && rule.tool_name != tool_name {
                continue;
            }
            let matches = match &rule.condition {
                PermissionCondition::AlwaysDeny => true,
                PermissionCondition::ArgumentContains { arg_name, pattern } => arguments
                    .iter()
                    .any(|(k, v)| k == arg_name && v.contains(pattern.as_str())),
                PermissionCondition::CommandContains(pattern) => arguments
                    .iter()
                    .any(|(k, v)| k == "command" && v.contains(pattern.as_str())),
            };
            if matches {
                return Some(rule.reason.clone());
            }
        }
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToolRuntimeMode {
    #[default]
    Standard,
    Simple,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSelectionIssue {
    pub tool_name: String,
    pub reason: String,
}

impl ToolSelectionIssue {
    fn new(tool_name: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            tool_name: tool_name.into(),
            reason: reason.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToolRegistrySelection {
    pub available_tools: Vec<ToolDefinition>,
    pub unavailable_tools: Vec<ToolSelectionIssue>,
}

impl ToolRegistrySelection {
    pub fn has_tool(&self, tool_name: &str) -> bool {
        self.available_tools
            .iter()
            .any(|tool| tool.name == tool_name)
    }

    pub fn issue_for(&self, tool_name: &str) -> Option<&ToolSelectionIssue> {
        self.unavailable_tools
            .iter()
            .find(|issue| issue.tool_name == tool_name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolRegistry {
    pub base_tools: Vec<ToolDefinition>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self {
            base_tools: vec![
                ToolDefinition::new("Agent", ToolKind::Orchestration, false)
                    .with_permission(PermissionMode::WorkspaceWrite),
                ToolDefinition::new("Task", ToolKind::Orchestration, false)
                    .with_permission(PermissionMode::WorkspaceWrite),
                ToolDefinition::new("Bash", ToolKind::Execution, true)
                    .with_permission(PermissionMode::WorkspaceWrite),
                ToolDefinition::new("Glob", ToolKind::ReadOnly, false)
                    .with_permission(PermissionMode::ReadOnly),
                ToolDefinition::new("Grep", ToolKind::ReadOnly, false)
                    .with_permission(PermissionMode::ReadOnly),
                ToolDefinition::new("Read", ToolKind::ReadOnly, true)
                    .with_permission(PermissionMode::ReadOnly),
                ToolDefinition::new("Edit", ToolKind::Edit, true)
                    .with_permission(PermissionMode::WorkspaceWrite),
                ToolDefinition::new("Write", ToolKind::Edit, false)
                    .with_permission(PermissionMode::WorkspaceWrite),
                ToolDefinition::new("WebFetch", ToolKind::ReadOnly, false)
                    .with_permission(PermissionMode::ReadOnly),
                ToolDefinition::new("CronCreate", ToolKind::Orchestration, false)
                    .with_permission(PermissionMode::WorkspaceWrite),
                ToolDefinition::new("CronDelete", ToolKind::Orchestration, false)
                    .with_permission(PermissionMode::WorkspaceWrite),
                ToolDefinition::new("CronList", ToolKind::Orchestration, false)
                    .with_permission(PermissionMode::ReadOnly),
                ToolDefinition::new("TaskGet", ToolKind::ReadOnly, false)
                    .with_permission(PermissionMode::ReadOnly),
                ToolDefinition::new("TaskList", ToolKind::ReadOnly, false)
                    .with_permission(PermissionMode::ReadOnly),
                ToolDefinition::new("TaskUpdate", ToolKind::Orchestration, false)
                    .with_permission(PermissionMode::WorkspaceWrite),
                ToolDefinition::new("TaskStop", ToolKind::Orchestration, false)
                    .with_permission(PermissionMode::WorkspaceWrite),
                ToolDefinition::new("TaskOutput", ToolKind::ReadOnly, false)
                    .with_permission(PermissionMode::ReadOnly),
                ToolDefinition::new("TeamCreate", ToolKind::Orchestration, false)
                    .with_permission(PermissionMode::WorkspaceWrite),
                ToolDefinition::new("TeamDelete", ToolKind::Orchestration, false)
                    .with_permission(PermissionMode::WorkspaceWrite),
                ToolDefinition::new("ToolSearch", ToolKind::ReadOnly, false)
                    .with_permission(PermissionMode::ReadOnly),
                ToolDefinition::new("Lsp", ToolKind::ReadOnly, false)
                    .with_permission(PermissionMode::ReadOnly),
                ToolDefinition::new("MemorySave", ToolKind::Edit, false)
                    .with_permission(PermissionMode::WorkspaceWrite),
                ToolDefinition::new("MemoryList", ToolKind::ReadOnly, false)
                    .with_permission(PermissionMode::ReadOnly),
                ToolDefinition::new("MemorySearch", ToolKind::ReadOnly, false)
                    .with_permission(PermissionMode::ReadOnly),
                ToolDefinition::new("MemoryDelete", ToolKind::Edit, false)
                    .with_permission(PermissionMode::WorkspaceWrite),
            ],
        }
    }
}

impl ToolRegistry {
    pub fn select_tools(
        &self,
        requested_tools: &[String],
        runtime_mode: ToolRuntimeMode,
        permission_context: &ToolPermissionContext,
    ) -> ToolRegistrySelection {
        let mut selection = ToolRegistrySelection::default();
        let requested = if requested_tools.is_empty() {
            self.base_tools
                .iter()
                .map(|tool| tool.name.clone())
                .collect::<Vec<_>>()
        } else {
            requested_tools.to_vec()
        };

        for tool_name in requested {
            let Some(tool) = self.base_tools.iter().find(|tool| tool.name == tool_name) else {
                selection.unavailable_tools.push(ToolSelectionIssue::new(
                    tool_name,
                    "unknown tool in registry",
                ));
                continue;
            };

            if runtime_mode == ToolRuntimeMode::Simple && !tool.enabled_in_simple_mode {
                selection.unavailable_tools.push(ToolSelectionIssue::new(
                    tool.name.clone(),
                    "tool hidden in simple mode",
                ));
                continue;
            }

            if permission_context.denies(&tool.name) {
                selection.unavailable_tools.push(ToolSelectionIssue::new(
                    tool.name.clone(),
                    "tool denied by permission context",
                ));
                continue;
            }

            if tool.required_permission > permission_context.mode {
                selection.unavailable_tools.push(ToolSelectionIssue::new(
                    tool.name.clone(),
                    format!(
                        "tool requires {:?} but mode is {:?}",
                        tool.required_permission, permission_context.mode
                    ),
                ));
                continue;
            }

            if !selection
                .available_tools
                .iter()
                .any(|existing| existing.name == tool.name)
            {
                selection.available_tools.push(tool.clone());
            }
        }

        selection
    }
}

#[cfg(test)]
mod tests {
    use super::{PermissionMode, ToolKind, ToolPermissionContext, ToolRegistry, ToolRuntimeMode};

    #[test]
    fn standard_mode_keeps_requested_tools() {
        let registry = ToolRegistry::default();
        let selection = registry.select_tools(
            &[
                String::from("Read"),
                String::from("Edit"),
                String::from("Bash"),
            ],
            ToolRuntimeMode::Standard,
            &ToolPermissionContext::default(),
        );

        assert_eq!(selection.available_tools.len(), 3);
        assert_eq!(selection.available_tools[0].kind, ToolKind::ReadOnly);
        assert!(selection.unavailable_tools.is_empty());
    }

    #[test]
    fn simple_mode_hides_non_simple_tools() {
        let registry = ToolRegistry::default();
        let selection = registry.select_tools(
            &[
                String::from("Read"),
                String::from("Glob"),
                String::from("Bash"),
            ],
            ToolRuntimeMode::Simple,
            &ToolPermissionContext::default(),
        );

        assert_eq!(
            selection
                .available_tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Read", "Bash"]
        );
        assert_eq!(selection.unavailable_tools.len(), 1);
        assert_eq!(selection.unavailable_tools[0].tool_name, "Glob");
    }

    #[test]
    fn permission_context_can_blanket_deny_tool() {
        let registry = ToolRegistry::default();
        let selection = registry.select_tools(
            &[String::from("Read"), String::from("Edit")],
            ToolRuntimeMode::Standard,
            &ToolPermissionContext::default().deny("Read"),
        );

        assert_eq!(
            selection
                .available_tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Edit"]
        );
        assert_eq!(
            selection
                .issue_for("Read")
                .map(|issue| issue.reason.as_str()),
            Some("tool denied by permission context")
        );
    }

    #[test]
    fn unknown_requested_tool_is_reported() {
        let registry = ToolRegistry::default();
        let selection = registry.select_tools(
            &[String::from("Read"), String::from("UnknownTool")],
            ToolRuntimeMode::Standard,
            &ToolPermissionContext::default(),
        );

        assert!(selection.has_tool("Read"));
        assert_eq!(
            selection
                .issue_for("UnknownTool")
                .map(|issue| issue.reason.as_str()),
            Some("unknown tool in registry")
        );
    }

    #[test]
    fn read_only_mode_blocks_edit_tools() {
        let registry = ToolRegistry::default();
        let ctx = ToolPermissionContext::default().with_mode(PermissionMode::ReadOnly);
        let selection = registry.select_tools(
            &[
                String::from("Read"),
                String::from("Glob"),
                String::from("Edit"),
                String::from("Write"),
                String::from("Bash"),
            ],
            ToolRuntimeMode::Standard,
            &ctx,
        );

        let available: Vec<&str> = selection
            .available_tools
            .iter()
            .map(|t| t.name.as_str())
            .collect();
        assert!(available.contains(&"Read"));
        assert!(available.contains(&"Glob"));
        assert!(!available.contains(&"Edit"));
        assert!(!available.contains(&"Write"));
        assert!(!available.contains(&"Bash"));
        assert!(selection.issue_for("Edit").is_some());
        assert!(selection.issue_for("Write").is_some());
        assert!(selection.issue_for("Bash").is_some());
    }

    #[test]
    fn workspace_write_mode_allows_edit_tools() {
        let registry = ToolRegistry::default();
        let ctx = ToolPermissionContext::default().with_mode(PermissionMode::WorkspaceWrite);
        let selection = registry.select_tools(
            &[
                String::from("Read"),
                String::from("Edit"),
                String::from("Write"),
                String::from("Bash"),
                String::from("Agent"),
            ],
            ToolRuntimeMode::Standard,
            &ctx,
        );

        let available: Vec<&str> = selection
            .available_tools
            .iter()
            .map(|t| t.name.as_str())
            .collect();
        assert!(available.contains(&"Read"));
        assert!(available.contains(&"Edit"));
        assert!(available.contains(&"Write"));
        assert!(available.contains(&"Bash"));
        assert!(available.contains(&"Agent"));
        assert!(selection.unavailable_tools.is_empty());
    }

    #[test]
    fn danger_mode_allows_all() {
        let registry = ToolRegistry::default();
        let ctx = ToolPermissionContext::default().with_mode(PermissionMode::DangerFullAccess);
        let selection = registry.select_tools(
            &[],
            ToolRuntimeMode::Standard,
            &ctx,
        );

        // All base tools should be available.
        assert_eq!(selection.available_tools.len(), registry.base_tools.len());
        assert!(selection.unavailable_tools.is_empty());
    }
}
