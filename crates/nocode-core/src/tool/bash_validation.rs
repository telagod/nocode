//! Bash command validation — 6-submodule safety checks before execution.
//!
//! Submodules:
//! 1. read_only — classify commands as read-only (auto-approvable)
//! 2. destructive — detect destructive commands/patterns
//! 3. mode — permission-mode-aware validation (ReadOnly/WorkspaceWrite/DangerFullAccess)
//! 4. sed — sed/awk command analysis (detect in-place writes)
//! 5. path — path escape detection (symlinks, traversal, sensitive paths)
//! 6. semantics — semantic analysis (pipe chains, subshells, backgrounding)

use crate::tool::permission::PermissionMode;

// =========================================================================
// 1. read_only — classify safe-to-auto-approve commands
// =========================================================================

const READ_ONLY_COMMANDS: &[&str] = &[
    "ls",
    "cat",
    "head",
    "tail",
    "wc",
    "find",
    "grep",
    "rg",
    "ag",
    "fd",
    "tree",
    "file",
    "stat",
    "du",
    "df",
    "which",
    "whereis",
    "type",
    "echo",
    "printf",
    "date",
    "whoami",
    "hostname",
    "uname",
    "env",
    "printenv",
    "pwd",
    "id",
    "git",
    "cargo",
    "rustc",
    "node",
    "python",
    "python3",
    "go",
    "java",
    "javac",
    "npm",
    "yarn",
    "pnpm",
    "pip",
    "pip3",
    "diff",
    "md5sum",
    "sha256sum",
    "readlink",
    "realpath",
    "basename",
    "dirname",
    "sort",
    "uniq",
    "tr",
    "cut",
    "awk",
    "jq",
    "less",
    "more",
    "strings",
    "hexdump",
    "xxd",
    "test",
    "true",
    "false",
];

/// Check if a command is read-only (safe to auto-approve).
pub fn is_read_only_command(command: &str) -> bool {
    let cmd = command.trim();
    let first_word = first_command_word(cmd);
    READ_ONLY_COMMANDS.contains(&first_word)
}

// =========================================================================
// 2. destructive — detect dangerous commands and patterns
// =========================================================================

const DESTRUCTIVE_COMMANDS: &[&str] = &[
    "rm",
    "rmdir",
    "mkfs",
    "dd",
    "shred",
    "kill",
    "killall",
    "pkill",
    "shutdown",
    "reboot",
    "halt",
    "poweroff",
    "init",
    "systemctl",
    "fdisk",
    "parted",
    "wipefs",
    "lvremove",
    "vgremove",
    "pvremove",
];

const DESTRUCTIVE_PATTERNS: &[&str] = &[
    "rm -rf /",
    "rm -rf /*",
    "> /dev/sda",
    "mkfs.",
    ":(){ :|:& };:",
    "dd if=/dev/zero",
    "chmod -R 777 /",
    "chown -R",
    "truncate -s 0",
    "> /dev/null 2>&1 &",
];

/// Check if a command is destructive (requires explicit approval).
pub fn is_destructive_command(command: &str) -> bool {
    let cmd = command.trim();
    let first_word = first_command_word(cmd);
    DESTRUCTIVE_COMMANDS.contains(&first_word)
        || DESTRUCTIVE_PATTERNS.iter().any(|p| cmd.contains(p))
}

// =========================================================================
// 3. mode — permission-mode-aware validation
// =========================================================================

/// Commands that perform write/modify operations (blocked in ReadOnly mode).
const WRITE_COMMANDS: &[&str] = &[
    "rm", "rmdir", "mv", "cp", "mkdir", "touch", "chmod", "chown", "chgrp",
    "ln", "install", "mktemp", "truncate", "shred", "dd", "tee",
    "sed", "patch", "nano", "vi", "vim", "emacs",
    "git", "cargo", "npm", "yarn", "pnpm", "pip", "pip3",
    "make", "cmake", "gcc", "g++", "rustc", "go", "javac",
    "docker", "podman", "kubectl",
    "systemctl", "service", "mount", "umount",
    "useradd", "userdel", "usermod", "groupadd", "groupdel",
    "iptables", "ufw", "firewall-cmd",
    "kill", "killall", "pkill",
    "shutdown", "reboot", "halt", "poweroff", "init",
    "mkfs", "fdisk", "parted", "wipefs",
    "curl", "wget",  // can write with -o
];

/// Check if a command performs write operations (not safe for ReadOnly mode).
pub fn is_write_command(command: &str) -> bool {
    let cmd = command.trim();
    let first_word = first_command_word(cmd);

    // Explicit write commands
    if WRITE_COMMANDS.contains(&first_word) {
        return true;
    }

    // Output redirection (> or >>)
    if cmd.contains(" > ") || cmd.contains(" >> ") {
        return true;
    }

    // sed in-place
    if is_sed_inplace(cmd) {
        return true;
    }

    // awk writing
    if is_awk_write(cmd) {
        return true;
    }

    // Pipe to write commands
    for wc in WRITE_COMMANDS {
        if cmd.contains(&format!("| {wc} ")) || cmd.contains(&format!("| {wc}")) {
            return true;
        }
    }

    false
}

/// Validate a command against the current permission mode.
pub fn validate_for_mode(command: &str, mode: PermissionMode) -> Result<(), String> {
    let cmd = command.trim();
    match mode {
        PermissionMode::Deny => Err("All commands blocked in Deny mode".to_string()),
        PermissionMode::ReadOnly => {
            validate_bash_command(cmd)?;
            if is_write_command(cmd) {
                Err(format!(
                    "Command blocked in ReadOnly mode (write operation): {}",
                    first_command_word(cmd)
                ))
            } else {
                Ok(())
            }
        }
        PermissionMode::Ask => {
            // In Ask mode, read-only commands pass; others need approval
            // (approval handled upstream by PermissionPrompter)
            validate_bash_command(cmd)
        }
        PermissionMode::Auto => validate_bash_command(cmd),
    }
}

// =========================================================================
// 4. sed — sed/awk in-place write detection
// =========================================================================

/// Check if a sed/awk command performs in-place file modification.
pub fn is_sed_inplace(command: &str) -> bool {
    let cmd = command.trim();
    // sed -i / sed --in-place
    if (cmd.starts_with("sed ") || cmd.contains("| sed ") || cmd.contains("; sed "))
        && (cmd.contains(" -i") || cmd.contains("--in-place"))
    {
        return true;
    }
    // perl -pi -e
    if cmd.contains("perl") && cmd.contains("-pi") {
        return true;
    }
    false
}

/// Check if an awk command writes to files (redirection within awk).
pub fn is_awk_write(command: &str) -> bool {
    let cmd = command.trim();
    if (cmd.starts_with("awk ") || cmd.contains("| awk ")) && cmd.contains(" > ") {
        return true;
    }
    // awk -i inplace
    if cmd.contains("awk") && cmd.contains("-i inplace") {
        return true;
    }
    false
}

// =========================================================================
// 5. path — path escape and sensitive path detection
// =========================================================================

const SENSITIVE_PATHS: &[&str] = &[
    "/etc/shadow",
    "/etc/passwd",
    "/etc/sudoers",
    "/etc/ssh",
    "/root/.ssh",
    "/proc/",
    "/sys/",
    "/dev/",
    "/boot/",
    "/var/log/auth",
    "/var/log/secure",
];

/// Check if a command references sensitive system paths.
pub fn references_sensitive_path(command: &str) -> bool {
    let cmd = command.trim();
    SENSITIVE_PATHS.iter().any(|p| cmd.contains(p))
}

/// Check if a command contains path traversal attempts.
pub fn contains_path_escape(command: &str) -> bool {
    let cmd = command.trim();
    // Deep traversal
    cmd.contains("/../../../")
        // Symlink creation pointing outside
        || (cmd.contains("ln -s") && cmd.contains(".."))
        // Null byte injection
        || cmd.contains("\\x00")
        || cmd.contains("\\0")
}

// =========================================================================
// 6. semantics — pipe chains, subshells, backgrounding
// =========================================================================

/// Check if a command uses backgrounding (&).
pub fn uses_backgrounding(command: &str) -> bool {
    let cmd = command.trim();
    // Trailing & (not &&)
    cmd.ends_with(" &") || cmd.ends_with("\t&") || (cmd.contains(" & ") && !cmd.contains(" && "))
}

/// Check if a command uses subshell execution.
pub fn uses_subshell(command: &str) -> bool {
    let cmd = command.trim();
    cmd.contains("$(") || cmd.contains("`") || cmd.starts_with('(')
}

/// Check if a command uses output redirection that could overwrite files.
pub fn uses_destructive_redirect(command: &str) -> bool {
    let cmd = command.trim();
    // > file (overwrite), but not >> (append) or 2> (stderr)
    // Simple heuristic: look for " > " not preceded by "2" or followed by ">"
    for (i, _) in cmd.match_indices(" > ") {
        let before = if i > 0 { &cmd[i - 1..i] } else { "" };
        let after_pos = i + 3;
        let after = if after_pos < cmd.len() {
            &cmd[after_pos..after_pos + 1]
        } else {
            ""
        };
        if before != "2" && after != ">" && after != "&" {
            return true;
        }
    }
    false
}

/// Count pipe stages in a command.
pub fn pipe_depth(command: &str) -> usize {
    let cmd = command.trim();
    // Count | but not ||
    let mut depth = 0;
    let bytes = cmd.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'|' {
            let next = bytes.get(i + 1).copied().unwrap_or(0);
            let prev = if i > 0 { bytes[i - 1] } else { 0 };
            if next != b'|' && prev != b'|' {
                depth += 1;
            }
        }
    }
    depth
}

// =========================================================================
// Main validation entry point (combines all submodules)
// =========================================================================

/// Validate a bash command for safety. Returns Err(reason) if blocked.
pub fn validate_bash_command(command: &str) -> Result<(), String> {
    let cmd = command.trim();

    if cmd.is_empty() {
        return Err("Empty command".to_string());
    }

    // Destructive pattern check
    for pattern in DESTRUCTIVE_PATTERNS {
        if cmd.contains(pattern) {
            return Err(format!("Blocked destructive pattern: {pattern}"));
        }
    }

    // Path escape check
    if contains_path_escape(cmd) {
        return Err("Command contains suspicious path traversal".to_string());
    }

    // Sensitive path check (warn but don't block — executor handles permission)
    // Blocked only for the most dangerous combinations
    if references_sensitive_path(cmd) && is_destructive_command(cmd) {
        return Err("Destructive command targeting sensitive system path".to_string());
    }

    // Sed in-place on sensitive paths
    if is_sed_inplace(cmd) && references_sensitive_path(cmd) {
        return Err("In-place sed on sensitive system path".to_string());
    }

    Ok(())
}

// =========================================================================
// Helpers
// =========================================================================

fn first_command_word(cmd: &str) -> &str {
    // Skip env var assignments (FOO=bar cmd)
    let mut parts = cmd.split_whitespace();
    for part in parts.by_ref() {
        if !part.contains('=') || part.starts_with('-') {
            return part;
        }
    }
    ""
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- read_only ---
    #[test]
    fn read_only_detection() {
        assert!(is_read_only_command("ls -la"));
        assert!(is_read_only_command("git log"));
        assert!(is_read_only_command("cargo test"));
        assert!(is_read_only_command("jq '.foo' file.json"));
        assert!(!is_read_only_command("rm file.txt"));
        assert!(!is_read_only_command("mv a b"));
    }

    // --- destructive ---
    #[test]
    fn destructive_detection() {
        assert!(is_destructive_command("rm -rf /tmp/test"));
        assert!(is_destructive_command("kill -9 1234"));
        assert!(is_destructive_command("dd if=/dev/zero of=disk.img"));
        assert!(!is_destructive_command("ls -la"));
        assert!(!is_destructive_command("echo hello"));
    }

    #[test]
    fn blocks_destructive_patterns() {
        assert!(validate_bash_command("rm -rf /").is_err());
        assert!(validate_bash_command("rm -rf /*").is_err());
        assert!(validate_bash_command("dd if=/dev/zero of=/dev/sda").is_err());
        assert!(validate_bash_command(":(){ :|:& };:").is_err());
    }

    #[test]
    fn allows_safe_commands() {
        assert!(validate_bash_command("ls -la").is_ok());
        assert!(validate_bash_command("cargo build").is_ok());
        assert!(validate_bash_command("git status").is_ok());
    }

    #[test]
    fn blocks_empty_commands() {
        assert!(validate_bash_command("").is_err());
        assert!(validate_bash_command("   ").is_err());
    }

    // --- mode ---
    #[test]
    fn deny_mode_blocks_all() {
        assert!(validate_for_mode("ls", PermissionMode::Deny).is_err());
    }

    #[test]
    fn auto_mode_allows_safe() {
        assert!(validate_for_mode("echo hi", PermissionMode::Auto).is_ok());
    }

    // --- read_only mode ---
    #[test]
    fn readonly_allows_read_commands() {
        assert!(validate_for_mode("ls -la", PermissionMode::ReadOnly).is_ok());
        assert!(validate_for_mode("cat file.txt", PermissionMode::ReadOnly).is_ok());
        assert!(validate_for_mode("grep foo bar.txt", PermissionMode::ReadOnly).is_ok());
        assert!(validate_for_mode("find . -name '*.rs'", PermissionMode::ReadOnly).is_ok());
        assert!(validate_for_mode("head -20 file.txt", PermissionMode::ReadOnly).is_ok());
        assert!(validate_for_mode("wc -l file.txt", PermissionMode::ReadOnly).is_ok());
        assert!(validate_for_mode("echo hello", PermissionMode::ReadOnly).is_ok());
    }

    #[test]
    fn readonly_blocks_write_commands() {
        assert!(validate_for_mode("rm file.txt", PermissionMode::ReadOnly).is_err());
        assert!(validate_for_mode("mv a b", PermissionMode::ReadOnly).is_err());
        assert!(validate_for_mode("cp a b", PermissionMode::ReadOnly).is_err());
        assert!(validate_for_mode("mkdir /tmp/test", PermissionMode::ReadOnly).is_err());
        assert!(validate_for_mode("touch file.txt", PermissionMode::ReadOnly).is_err());
        assert!(validate_for_mode("chmod 755 file", PermissionMode::ReadOnly).is_err());
        assert!(validate_for_mode("chown root file", PermissionMode::ReadOnly).is_err());
    }

    #[test]
    fn readonly_blocks_redirect() {
        assert!(validate_for_mode("echo x > file.txt", PermissionMode::ReadOnly).is_err());
        assert!(validate_for_mode("echo x >> file.txt", PermissionMode::ReadOnly).is_err());
    }

    #[test]
    fn readonly_blocks_sed_inplace() {
        assert!(validate_for_mode("sed -i 's/a/b/' f.txt", PermissionMode::ReadOnly).is_err());
    }

    #[test]
    fn readonly_blocks_pipe_to_write() {
        assert!(validate_for_mode("cat f | tee out.txt", PermissionMode::ReadOnly).is_err());
    }

    // --- write command detection ---
    #[test]
    fn write_command_detection() {
        assert!(is_write_command("rm file.txt"));
        assert!(is_write_command("mv a b"));
        assert!(is_write_command("cp a b"));
        assert!(is_write_command("mkdir /tmp/test"));
        assert!(is_write_command("touch file.txt"));
        assert!(is_write_command("chmod 755 file"));
        assert!(is_write_command("echo x > file.txt"));
        assert!(is_write_command("sed -i 's/a/b/' f"));
        assert!(!is_write_command("ls -la"));
        assert!(!is_write_command("cat file.txt"));
        assert!(!is_write_command("grep foo bar"));
        assert!(!is_write_command("echo hello"));
    }

    // --- sed ---
    #[test]
    fn sed_inplace_detected() {
        assert!(is_sed_inplace("sed -i 's/foo/bar/' file.txt"));
        assert!(is_sed_inplace("sed --in-place 's/a/b/' f"));
        assert!(is_sed_inplace("cat f | sed -i 's/x/y/' g"));
        assert!(!is_sed_inplace("sed 's/foo/bar/' file.txt"));
        assert!(!is_sed_inplace("echo hello"));
    }

    #[test]
    fn awk_write_detected() {
        assert!(is_awk_write("awk '{print}' f > out.txt"));
        assert!(is_awk_write("awk -i inplace '{gsub(/a/,\"b\")}' f"));
        assert!(!is_awk_write("awk '{print $1}' file.txt"));
    }

    // --- path ---
    #[test]
    fn sensitive_path_detected() {
        assert!(references_sensitive_path("cat /etc/shadow"));
        assert!(references_sensitive_path("ls /proc/1/maps"));
        assert!(!references_sensitive_path("cat /tmp/test.txt"));
    }

    #[test]
    fn path_escape_detected() {
        assert!(contains_path_escape("cat /tmp/../../../../etc/passwd"));
        assert!(!contains_path_escape("cat /tmp/file.txt"));
    }

    #[test]
    fn destructive_on_sensitive_blocked() {
        assert!(validate_bash_command("rm /etc/shadow").is_err());
    }

    #[test]
    fn sed_inplace_on_sensitive_blocked() {
        assert!(validate_bash_command("sed -i 's/x/y/' /etc/passwd").is_err());
    }

    // --- semantics ---
    #[test]
    fn backgrounding_detected() {
        assert!(uses_backgrounding("sleep 100 &"));
        assert!(!uses_backgrounding("echo foo && echo bar"));
        assert!(!uses_backgrounding("echo hello"));
    }

    #[test]
    fn subshell_detected() {
        assert!(uses_subshell("echo $(whoami)"));
        assert!(uses_subshell("echo `date`"));
        assert!(uses_subshell("(cd /tmp && ls)"));
        assert!(!uses_subshell("echo hello"));
    }

    #[test]
    fn destructive_redirect_detected() {
        assert!(uses_destructive_redirect("echo x > /tmp/file"));
        assert!(!uses_destructive_redirect("echo x >> /tmp/file"));
        assert!(!uses_destructive_redirect("cmd 2> /dev/null"));
    }

    #[test]
    fn pipe_depth_counted() {
        assert_eq!(pipe_depth("ls"), 0);
        assert_eq!(pipe_depth("ls | grep foo"), 1);
        assert_eq!(pipe_depth("cat f | grep x | wc -l"), 2);
        assert_eq!(pipe_depth("echo a || echo b"), 0);
    }

    // --- first_command_word ---
    #[test]
    fn env_var_prefix_skipped() {
        assert_eq!(first_command_word("FOO=bar ls -la"), "ls");
        assert_eq!(first_command_word("A=1 B=2 echo hi"), "echo");
        assert_eq!(first_command_word("ls -la"), "ls");
    }
}
