//! Bash command validation — safety checks before execution.

/// Validate a bash command for safety. Returns Err(reason) if blocked.
pub fn validate_bash_command(command: &str) -> Result<(), String> {
    let cmd = command.trim();

    // Block empty commands
    if cmd.is_empty() {
        return Err("Empty command".to_string());
    }

    // Block destructive patterns
    for pattern in DESTRUCTIVE_PATTERNS {
        if cmd.contains(pattern) {
            return Err(format!("Blocked destructive pattern: {pattern}"));
        }
    }

    // Block path traversal outside workspace
    if contains_path_escape(cmd) {
        return Err("Command contains suspicious path traversal".to_string());
    }

    Ok(())
}

/// Check if a command is read-only (safe to auto-approve).
pub fn is_read_only_command(command: &str) -> bool {
    let cmd = command.trim();
    let first_word = cmd.split_whitespace().next().unwrap_or("");
    READ_ONLY_COMMANDS.contains(&first_word)
}

/// Check if a command is destructive (requires explicit approval).
pub fn is_destructive_command(command: &str) -> bool {
    let cmd = command.trim();
    let first_word = cmd.split_whitespace().next().unwrap_or("");
    DESTRUCTIVE_COMMANDS.contains(&first_word)
        || DESTRUCTIVE_PATTERNS.iter().any(|p| cmd.contains(p))
}

fn contains_path_escape(cmd: &str) -> bool {
    // Check for attempts to escape via symlinks or ../
    cmd.contains("/../../../") || cmd.contains("/etc/shadow") || cmd.contains("/etc/passwd")
}

const READ_ONLY_COMMANDS: &[&str] = &[
    "ls", "cat", "head", "tail", "wc", "find", "grep", "rg", "ag", "fd", "tree", "file", "stat",
    "du", "df", "which", "whereis", "type", "echo", "printf", "date", "whoami", "hostname",
    "uname", "env", "printenv", "pwd", "id", "git", "cargo", "rustc", "node", "python", "python3",
    "go", "java", "javac", "npm", "yarn", "pnpm", "pip", "pip3",
];

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
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_safe_commands() {
        assert!(validate_bash_command("ls -la").is_ok());
        assert!(validate_bash_command("cargo build").is_ok());
        assert!(validate_bash_command("git status").is_ok());
    }

    #[test]
    fn blocks_destructive_patterns() {
        assert!(validate_bash_command("rm -rf /").is_err());
        assert!(validate_bash_command("rm -rf /*").is_err());
        assert!(validate_bash_command("dd if=/dev/zero of=/dev/sda").is_err());
    }

    #[test]
    fn blocks_empty_commands() {
        assert!(validate_bash_command("").is_err());
        assert!(validate_bash_command("   ").is_err());
    }

    #[test]
    fn read_only_detection() {
        assert!(is_read_only_command("ls -la"));
        assert!(is_read_only_command("git log"));
        assert!(is_read_only_command("cargo test"));
        assert!(!is_read_only_command("rm file.txt"));
        assert!(!is_read_only_command("mv a b"));
    }

    #[test]
    fn destructive_detection() {
        assert!(is_destructive_command("rm -rf /tmp/test"));
        assert!(is_destructive_command("kill -9 1234"));
        assert!(!is_destructive_command("ls -la"));
        assert!(!is_destructive_command("echo hello"));
    }
}
