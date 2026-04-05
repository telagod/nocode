use crate::tool_registry::PermissionMode;

/// Result of running all bash validation checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BashValidationResult {
    pub allowed: bool,
    pub warnings: Vec<String>,
    pub denial_reason: Option<String>,
}

/// Main entry point: run all 6 validators against a command.
pub fn validate_bash_command(command: &str, mode: PermissionMode, cwd: &str) -> BashValidationResult {
    let mut warnings = Vec::new();

    // 1. Read-only validation
    if mode == PermissionMode::ReadOnly
        && let Some(reason) = read_only_validation(command)
    {
        return BashValidationResult {
            allowed: false,
            warnings,
            denial_reason: Some(reason),
        };
    }

    // 2. Destructive command warning (always checked, blocks)
    if let Some(reason) = destructive_command_warning(command) {
        return BashValidationResult {
            allowed: false,
            warnings,
            denial_reason: Some(reason),
        };
    }

    // 3. Mode validation (workspace write restrictions)
    if let Some(reason) = mode_validation(command, mode) {
        return BashValidationResult {
            allowed: false,
            warnings,
            denial_reason: Some(reason),
        };
    }

    // 4. Sed validation
    if mode == PermissionMode::ReadOnly
        && let Some(reason) = sed_validation(command)
    {
        return BashValidationResult {
            allowed: false,
            warnings,
            denial_reason: Some(reason),
        };
    }
    // 5. Path validation
    if let Some(reason) = path_validation(command, cwd) {
        warnings.push(reason);
    }

    // 6. Command semantics
    if let Some(reason) = command_semantics(command) {
        return BashValidationResult {
            allowed: false,
            warnings,
            denial_reason: Some(reason),
        };
    }

    BashValidationResult {
        allowed: true,
        warnings,
        denial_reason: None,
    }
}

/// (a) In ReadOnly mode, block all write commands.
pub fn read_only_validation(command: &str) -> Option<String> {
    let normalized = command.to_ascii_lowercase();
    let trimmed = normalized.trim();

    // Write indicators
    static WRITE_INDICATORS: &[&str] = &[
        "tee ", "tee\t", "dd ", "mv ", "cp ", "rm ", "mkdir ", "rmdir ",
        "chmod ", "chown ", "install ", "patch ",
    ];

    // Redirect operators
    if trimmed.contains(">>") || contains_output_redirect(trimmed) {
        return Some("write operation blocked in read-only mode".to_string());
    }

    for indicator in WRITE_INDICATORS {
        if trimmed.starts_with(indicator.trim()) || trimmed.contains(&format!("| {}", indicator.trim())) {
            return Some(format!(
                "write command '{}' blocked in read-only mode",
                indicator.trim()
            ));
        }
    }

    // sed -i (in-place edit)
    if trimmed.contains("sed") && trimmed.contains("-i") {
        return Some("sed in-place edit blocked in read-only mode".to_string());
    }

    None
}

/// Check if a string contains an output redirect (> but not >>).
fn contains_output_redirect(s: &str) -> bool {
    let bytes = s.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'>' {
            // Not part of >>
            if i + 1 < bytes.len() && bytes[i + 1] == b'>' {
                continue;
            }
            // Not preceded by another > (second char of >>)
            if i > 0 && bytes[i - 1] == b'>' {
                continue;
            }
            // Not part of heredoc <<
            if i > 0 && bytes[i - 1] == b'<' {
                continue;
            }
            return true;
        }
    }
    false
}

/// (b) Detect highly destructive commands.
pub fn destructive_command_warning(command: &str) -> Option<String> {
    let normalized = command.to_ascii_lowercase();
    let trimmed = normalized.trim();

    static DESTRUCTIVE_PATTERNS: &[(&str, &str)] = &[
        ("rm -rf /", "recursive delete of root filesystem"),
        ("rm -rf /*", "recursive delete of root filesystem"),
        ("rm -rf ~", "recursive delete of home directory"),
        ("mkfs", "filesystem format command"),
        ("dd if=", "raw disk write command"),
        (":(){:|:&};:", "fork bomb"),
        ("chmod -r 777 /", "recursive permission change on root"),
        ("> /dev/sda", "raw device write"),
        ("mv / ", "move root filesystem"),
        ("mv /\t", "move root filesystem"),
    ];

    for (pattern, reason) in DESTRUCTIVE_PATTERNS {
        if trimmed.contains(pattern) {
            return Some((*reason).to_string());
        }
    }

    // Database destructive commands
    static DB_DESTRUCTIVE: &[(&str, &str)] = &[
        ("drop database", "database drop without backup"),
        ("drop table", "table drop without backup"),
        ("truncate table", "table truncate without backup"),
    ];

    for (pattern, reason) in DB_DESTRUCTIVE {
        if trimmed.contains(pattern) && !trimmed.contains("backup") && !trimmed.contains("--dry") {
            return Some((*reason).to_string());
        }
    }

    // DELETE FROM without WHERE
    if trimmed.contains("delete from") && !trimmed.contains("where")
        && !trimmed.contains("backup") && !trimmed.contains("--dry")
    {
        return Some("bulk delete without WHERE clause".to_string());
    }

    None
}

/// (c) Mode-based validation: WorkspaceWrite blocks system path writes.
pub fn mode_validation(command: &str, mode: PermissionMode) -> Option<String> {
    if mode == PermissionMode::DangerFullAccess {
        return None;
    }

    let normalized = command.to_ascii_lowercase();
    let trimmed = normalized.trim();

    static SYSTEM_PATHS: &[&str] = &["/etc/", "/boot/", "/usr/", "/var/", "/opt/"];
    static WRITE_PREFIXES: &[&str] = &[
        "rm ", "mv ", "cp ", "mkdir ", "rmdir ", "chmod ", "chown ",
        "install ", "tee ", "dd ",
    ];

    let is_write_cmd = WRITE_PREFIXES.iter().any(|p| trimmed.starts_with(p));
    // Also check for redirect to system path
    let has_redirect_to_system = SYSTEM_PATHS.iter().any(|sp| {
        trimmed.contains(&format!("> {sp}")) || trimmed.contains(&format!(">> {sp}"))
    });

    if is_write_cmd || has_redirect_to_system {
        for sys_path in SYSTEM_PATHS {
            if trimmed.contains(sys_path) {
                return Some(format!(
                    "write to system path '{}' blocked in {:?} mode",
                    sys_path.trim_end_matches('/'),
                    mode
                ));
            }
        }
    }

    None
}

/// (d) Sed-specific validation.
pub fn sed_validation(command: &str) -> Option<String> {
    let normalized = command.to_ascii_lowercase();
    let trimmed = normalized.trim();

    if !trimmed.contains("sed") {
        return None;
    }

    // sed -i (in-place) detection
    if trimmed.contains("sed -i") || trimmed.contains("sed -i'") || trimmed.contains("sed -i\"") {
        // Check if targeting system files
        static SYSTEM_PATHS: &[&str] = &["/etc/", "/boot/", "/usr/", "/var/", "/opt/", "/sys/", "/proc/"];
        for sp in SYSTEM_PATHS {
            if trimmed.contains(sp) {
                return Some(format!("sed in-place edit of system file in {sp} blocked"));
            }
        }
        return Some("sed in-place edit detected".to_string());
    }

    None
}

/// (e) Path validation: detect commands targeting paths outside cwd.
pub fn path_validation(command: &str, cwd: &str) -> Option<String> {
    let tokens: Vec<&str> = command.split_whitespace().collect();

    static SYSTEM_PATHS: &[&str] = &["/etc", "/boot", "/usr", "/sys", "/proc"];

    for token in &tokens {
        if !token.starts_with('/') {
            continue;
        }
        // Check system paths — always warn
        for sp in SYSTEM_PATHS {
            if token.starts_with(sp) {
                return Some(format!("command references system path: {token}"));
            }
        }
        // Check if path is outside cwd
        if !token.starts_with(cwd) {
            return Some(format!(
                "path '{token}' is outside working directory '{cwd}'"
            ));
        }
    }

    None
}

/// (f) Command semantics: block system-level dangerous operations.
pub fn command_semantics(command: &str) -> Option<String> {
    let normalized = command.to_ascii_lowercase();
    let trimmed = normalized.trim();

    // Shutdown/reboot/halt/poweroff
    static SHUTDOWN_CMDS: &[&str] = &[
        "shutdown", "reboot", "halt", "poweroff", "init 0", "init 6",
    ];
    for cmd in SHUTDOWN_CMDS {
        if trimmed.starts_with(cmd) {
            return Some(format!("system command '{cmd}' blocked"));
        }
    }

    // kill -9 1 (killing init/systemd)
    if trimmed.contains("kill") && trimmed.contains("-9") && trimmed.contains(" 1") {
        // More precise: check for "kill -9 1" pattern
        let re_tokens: Vec<&str> = trimmed.split_whitespace().collect();
        if let Some(pos) = re_tokens.iter().position(|t| *t == "kill") {
            let rest = &re_tokens[pos..];
            if rest.contains(&"-9") && rest.last() == Some(&"1") {
                return Some("killing init process (PID 1) blocked".to_string());
            }
        }
    }

    // iptables (firewall modification)
    if trimmed.starts_with("iptables") || trimmed.starts_with("ip6tables") {
        return Some("firewall modification via iptables blocked".to_string());
    }

    // systemctl stop/disable
    if trimmed.starts_with("systemctl stop") || trimmed.starts_with("systemctl disable") {
        return Some("systemctl stop/disable blocked".to_string());
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_registry::PermissionMode;

    #[test]
    fn read_only_blocks_write_commands() {
        let result = validate_bash_command("mv file.txt /tmp/", PermissionMode::ReadOnly, "/home/user/project");
        assert!(!result.allowed);
        assert!(result.denial_reason.is_some());

        let result = validate_bash_command("rm file.txt", PermissionMode::ReadOnly, "/home/user/project");
        assert!(!result.allowed);

        let result = validate_bash_command("echo hello > out.txt", PermissionMode::ReadOnly, "/home/user/project");
        assert!(!result.allowed);

        let result = validate_bash_command("cp a.txt b.txt", PermissionMode::ReadOnly, "/home/user/project");
        assert!(!result.allowed);
    }

    #[test]
    fn read_only_allows_read_commands() {
        let cwd = "/home/user/project";
        let result = validate_bash_command("cat README.md", PermissionMode::ReadOnly, cwd);
        assert!(result.allowed);

        let result = validate_bash_command("ls -la", PermissionMode::ReadOnly, cwd);
        assert!(result.allowed);

        let result = validate_bash_command("grep -rn pattern .", PermissionMode::ReadOnly, cwd);
        assert!(result.allowed);

        let result = validate_bash_command("head -20 file.txt", PermissionMode::ReadOnly, cwd);
        assert!(result.allowed);

        let result = validate_bash_command("wc -l file.txt", PermissionMode::ReadOnly, cwd);
        assert!(result.allowed);
    }

    #[test]
    fn destructive_blocks_rm_rf() {
        let result = validate_bash_command("rm -rf /", PermissionMode::WorkspaceWrite, "/home/user");
        assert!(!result.allowed);
        assert!(result.denial_reason.unwrap().contains("recursive delete"));

        let result = validate_bash_command("rm -rf /*", PermissionMode::WorkspaceWrite, "/home/user");
        assert!(!result.allowed);
    }

    #[test]
    fn destructive_blocks_fork_bomb() {
        let result = validate_bash_command(":(){:|:&};:", PermissionMode::WorkspaceWrite, "/home/user");
        assert!(!result.allowed);
        assert!(result.denial_reason.unwrap().contains("fork bomb"));
    }

    #[test]
    fn destructive_blocks_db_drop() {
        let result = validate_bash_command("psql -c 'DROP DATABASE prod'", PermissionMode::WorkspaceWrite, "/home/user");
        assert!(!result.allowed);
        assert!(result.denial_reason.unwrap().contains("database drop"));

        let result = validate_bash_command("mysql -e 'TRUNCATE TABLE users'", PermissionMode::WorkspaceWrite, "/home/user");
        assert!(!result.allowed);

        let result = validate_bash_command("psql -c 'DELETE FROM users'", PermissionMode::WorkspaceWrite, "/home/user");
        assert!(!result.allowed);
        assert!(result.denial_reason.unwrap().contains("WHERE"));
    }

    #[test]
    fn mode_blocks_system_paths_in_workspace_mode() {
        let result = validate_bash_command("rm /etc/passwd", PermissionMode::WorkspaceWrite, "/home/user");
        assert!(!result.allowed);
        assert!(result.denial_reason.unwrap().contains("/etc"));

        let result = validate_bash_command("cp file /boot/grub.cfg", PermissionMode::WorkspaceWrite, "/home/user");
        assert!(!result.allowed);

        let result = validate_bash_command("mv file /usr/local/bin/x", PermissionMode::WorkspaceWrite, "/home/user");
        assert!(!result.allowed);
    }

    #[test]
    fn mode_allows_system_paths_in_danger_mode() {
        // DangerFullAccess skips mode_validation, but destructive still blocks rm -rf /
        let result = validate_bash_command("cp file /etc/config", PermissionMode::DangerFullAccess, "/home/user");
        // mode_validation won't block, but path_validation will warn
        assert!(result.allowed);
    }

    #[test]
    fn sed_i_blocked_in_readonly() {
        let result = validate_bash_command("sed -i 's/foo/bar/' file.txt", PermissionMode::ReadOnly, "/home/user");
        assert!(!result.allowed);
        assert!(result.denial_reason.unwrap().contains("sed"));
    }

    #[test]
    fn path_outside_cwd_warned() {
        let result = validate_bash_command("cat /tmp/secret.txt", PermissionMode::WorkspaceWrite, "/home/user/project");
        assert!(result.allowed); // warning only, not denial
        assert!(!result.warnings.is_empty());
        assert!(result.warnings[0].contains("outside working directory"));
    }

    #[test]
    fn semantics_blocks_shutdown() {
        let result = validate_bash_command("shutdown -h now", PermissionMode::WorkspaceWrite, "/home/user");
        assert!(!result.allowed);
        assert!(result.denial_reason.unwrap().contains("shutdown"));

        let result = validate_bash_command("reboot", PermissionMode::WorkspaceWrite, "/home/user");
        assert!(!result.allowed);

        let result = validate_bash_command("poweroff", PermissionMode::WorkspaceWrite, "/home/user");
        assert!(!result.allowed);

        let result = validate_bash_command("init 0", PermissionMode::WorkspaceWrite, "/home/user");
        assert!(!result.allowed);
    }

    #[test]
    fn semantics_blocks_kill_init() {
        let result = validate_bash_command("kill -9 1", PermissionMode::WorkspaceWrite, "/home/user");
        assert!(!result.allowed);
        assert!(result.denial_reason.unwrap().contains("PID 1"));
    }

    #[test]
    fn safe_command_passes_all() {
        let cwd = "/home/user/project";
        let result = validate_bash_command("cargo test", PermissionMode::WorkspaceWrite, cwd);
        assert!(result.allowed);
        assert!(result.warnings.is_empty());
        assert!(result.denial_reason.is_none());

        let result = validate_bash_command("git status", PermissionMode::WorkspaceWrite, cwd);
        assert!(result.allowed);

        let result = validate_bash_command("ls -la", PermissionMode::WorkspaceWrite, cwd);
        assert!(result.allowed);
    }

    #[test]
    fn combined_validation_collects_warnings() {
        // A command that is allowed but has a path warning
        let result = validate_bash_command("cat /opt/data/file.txt", PermissionMode::WorkspaceWrite, "/home/user/project");
        // path_validation warns about system path /opt — but mode_validation won't block cat
        // Actually cat is not a write prefix, so mode_validation passes. path_validation warns.
        assert!(result.allowed);
        assert!(!result.warnings.is_empty());
    }

    #[test]
    fn workspace_write_allows_local_writes() {
        let cwd = "/home/user/project";
        let result = validate_bash_command("rm src/temp.txt", PermissionMode::WorkspaceWrite, cwd);
        assert!(result.allowed);

        let result = validate_bash_command("cp a.txt b.txt", PermissionMode::WorkspaceWrite, cwd);
        assert!(result.allowed);
    }

    #[test]
    fn danger_mode_allows_everything_except_destructive() {
        let cwd = "/home/user/project";
        // Allowed: system path write in danger mode
        let result = validate_bash_command("cp file /etc/config", PermissionMode::DangerFullAccess, cwd);
        assert!(result.allowed);

        // Still blocked: rm -rf /
        let result = validate_bash_command("rm -rf /", PermissionMode::DangerFullAccess, cwd);
        assert!(!result.allowed);

        // Still blocked: fork bomb
        let result = validate_bash_command(":(){:|:&};:", PermissionMode::DangerFullAccess, cwd);
        assert!(!result.allowed);
    }

    #[test]
    fn semantics_blocks_iptables() {
        let result = validate_bash_command("iptables -F", PermissionMode::WorkspaceWrite, "/home/user");
        assert!(!result.allowed);
        assert!(result.denial_reason.unwrap().contains("iptables"));
    }

    #[test]
    fn semantics_blocks_systemctl_stop() {
        let result = validate_bash_command("systemctl stop nginx", PermissionMode::WorkspaceWrite, "/home/user");
        assert!(!result.allowed);
        assert!(result.denial_reason.unwrap().contains("systemctl"));

        let result = validate_bash_command("systemctl disable sshd", PermissionMode::WorkspaceWrite, "/home/user");
        assert!(!result.allowed);
    }
}
