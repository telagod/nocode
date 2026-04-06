use crate::tool_registry::PermissionMode;

/// Result of a permission check at execution time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionCheckResult {
    Allowed,
    Denied {
        tool_name: String,
        required: PermissionMode,
        active: PermissionMode,
        reason: String,
    },
}

/// Check if a tool call is permitted under the active permission mode.
pub fn check_tool_permission(
    tool_name: &str,
    active_mode: PermissionMode,
) -> PermissionCheckResult {
    let required = required_permission_for(tool_name);
    if required <= active_mode {
        PermissionCheckResult::Allowed
    } else {
        PermissionCheckResult::Denied {
            tool_name: tool_name.to_string(),
            required,
            active: active_mode,
            reason: format!(
                "tool '{tool_name}' requires {required:?} but active mode is {active_mode:?}"
            ),
        }
    }
}

/// Map tool names to their required permission level.
fn required_permission_for(tool_name: &str) -> PermissionMode {
    match tool_name {
        "Read" | "Glob" | "Grep" | "WebFetch" | "WebSearch" | "TaskGet" | "TaskList"
        | "TaskOutput" | "CronList" | "ToolSearch" | "Lsp" | "MemoryList" | "MemorySearch" => {
            PermissionMode::ReadOnly
        }
        "Edit" | "Write" | "Bash" | "Agent" | "TaskUpdate" | "TaskStop" | "TeamCreate"
        | "TeamDelete" | "CronCreate" | "CronDelete" | "MemorySave" | "MemoryDelete" => {
            PermissionMode::WorkspaceWrite
        }
        _ if tool_name.starts_with("mcp:") => PermissionMode::WorkspaceWrite,
        _ => PermissionMode::WorkspaceWrite,
    }
}

/// Check if a file write is within workspace boundaries.
pub fn check_workspace_write(file_path: &str, cwd: &str) -> PermissionCheckResult {
    let canonical_cwd = std::fs::canonicalize(cwd).ok();
    let canonical_path = std::path::Path::new(file_path);

    // If path is absolute and doesn't start with cwd, deny
    if canonical_path.is_absolute()
        && let Some(ref cwd_path) = canonical_cwd
        && !canonical_path.starts_with(cwd_path)
    {
        return PermissionCheckResult::Denied {
            tool_name: "Write".to_string(),
            required: PermissionMode::DangerFullAccess,
            active: PermissionMode::WorkspaceWrite,
            reason: format!("path '{file_path}' is outside workspace '{cwd}'"),
        };
    }
    PermissionCheckResult::Allowed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_registry::PermissionMode;

    #[test]
    fn read_tool_allowed_in_readonly() {
        let result = check_tool_permission("Read", PermissionMode::ReadOnly);
        assert_eq!(result, PermissionCheckResult::Allowed);
    }

    #[test]
    fn edit_tool_denied_in_readonly() {
        let result = check_tool_permission("Edit", PermissionMode::ReadOnly);
        assert!(matches!(result, PermissionCheckResult::Denied { .. }));
        if let PermissionCheckResult::Denied {
            tool_name,
            required,
            active,
            ..
        } = result
        {
            assert_eq!(tool_name, "Edit");
            assert_eq!(required, PermissionMode::WorkspaceWrite);
            assert_eq!(active, PermissionMode::ReadOnly);
        }
    }

    #[test]
    fn bash_tool_allowed_in_workspace_write() {
        let result = check_tool_permission("Bash", PermissionMode::WorkspaceWrite);
        assert_eq!(result, PermissionCheckResult::Allowed);
    }

    #[test]
    fn all_tools_allowed_in_danger_mode() {
        let tools = [
            "Read",
            "Edit",
            "Write",
            "Bash",
            "Glob",
            "Grep",
            "Agent",
            "WebFetch",
            "WebSearch",
            "TaskGet",
            "TaskList",
            "TaskUpdate",
            "TaskStop",
            "TaskOutput",
            "TeamCreate",
            "TeamDelete",
            "CronCreate",
            "CronDelete",
            "CronList",
            "ToolSearch",
            "Lsp",
            "MemorySave",
            "MemoryList",
            "MemorySearch",
            "MemoryDelete",
            "mcp:server:tool",
        ];
        for tool in tools {
            let result = check_tool_permission(tool, PermissionMode::DangerFullAccess);
            assert_eq!(result, PermissionCheckResult::Allowed, "failed for {tool}");
        }
    }

    #[test]
    fn workspace_write_check_allows_relative_paths() {
        let result = check_workspace_write("src/main.rs", "/tmp");
        assert_eq!(result, PermissionCheckResult::Allowed);
    }

    #[test]
    fn workspace_write_check_denies_outside_paths() {
        let result = check_workspace_write("/etc/passwd", "/tmp");
        assert!(matches!(result, PermissionCheckResult::Denied { .. }));
    }

    #[test]
    fn unknown_tool_defaults_to_workspace() {
        // Unknown tool requires WorkspaceWrite, so ReadOnly denies it.
        let result = check_tool_permission("SomeFutureTool", PermissionMode::ReadOnly);
        assert!(matches!(result, PermissionCheckResult::Denied { .. }));

        // But WorkspaceWrite allows it.
        let result = check_tool_permission("SomeFutureTool", PermissionMode::WorkspaceWrite);
        assert_eq!(result, PermissionCheckResult::Allowed);
    }
}
