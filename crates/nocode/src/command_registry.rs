//! Command registry — centralized slash command management.
//!
//! Each command has a name, aliases, summary, optional argument hint, and an action type.
//! The TUI and REPL dispatch through this registry.

use std::collections::HashMap;

/// What a slash command does when invoked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandAction {
    Quit,
    Clear,
    Help,
    Status,
    Model,
    Sessions,
    Resume,
    Mcp,
    Agents,
    Compact,
    Config,
    Memory,
    Theme,
    Vim,
    Export,
    History,
    Version,
    Bug,
    Doctor,
    Permissions,
    Cost,
    Init,
    Login,
    Plan,
    Review,
    Skills,
    Env,
    Keybindings,
    BugHunter,
    SecurityReview,
    McpAdd,
    McpRemove,
    McpRestart,
    Insights,
}

/// A registered slash command.
#[derive(Debug, Clone)]
pub struct CommandEntry {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub summary: &'static str,
    pub argument_hint: Option<&'static str>,
    pub action: CommandAction,
}

/// Registry of all available slash commands.
pub struct CommandRegistry {
    commands: Vec<CommandEntry>,
    lookup: HashMap<String, usize>,
}

impl CommandRegistry {
    /// Create a registry with all built-in commands.
    pub fn with_defaults() -> Self {
        let commands = vec![
            CommandEntry {
                name: "quit",
                aliases: &["exit", "q"],
                summary: "Exit nocode",
                argument_hint: None,
                action: CommandAction::Quit,
            },
            CommandEntry {
                name: "clear",
                aliases: &[],
                summary: "Clear conversation history",
                argument_hint: None,
                action: CommandAction::Clear,
            },
            CommandEntry {
                name: "help",
                aliases: &["?", "h"],
                summary: "Show available commands",
                argument_hint: None,
                action: CommandAction::Help,
            },
            CommandEntry {
                name: "status",
                aliases: &["s"],
                summary: "Show session status and diagnostics",
                argument_hint: None,
                action: CommandAction::Status,
            },
            CommandEntry {
                name: "model",
                aliases: &[],
                summary: "Show or switch the active model",
                argument_hint: Some("<model_name>"),
                action: CommandAction::Model,
            },
            CommandEntry {
                name: "sessions",
                aliases: &[],
                summary: "List saved sessions",
                argument_hint: None,
                action: CommandAction::Sessions,
            },
            CommandEntry {
                name: "resume",
                aliases: &[],
                summary: "Resume a saved session",
                argument_hint: Some("<session_id>"),
                action: CommandAction::Resume,
            },
            CommandEntry {
                name: "mcp",
                aliases: &[],
                summary: "Show connected MCP servers and tools",
                argument_hint: None,
                action: CommandAction::Mcp,
            },
            CommandEntry {
                name: "agents",
                aliases: &["workers"],
                summary: "Show background agent workers",
                argument_hint: None,
                action: CommandAction::Agents,
            },
            CommandEntry {
                name: "compact",
                aliases: &[],
                summary: "Compact conversation context",
                argument_hint: None,
                action: CommandAction::Compact,
            },
            CommandEntry {
                name: "config",
                aliases: &["settings"],
                summary: "Show current configuration",
                argument_hint: None,
                action: CommandAction::Config,
            },
            CommandEntry {
                name: "memory",
                aliases: &["mem"],
                summary: "List or search memories",
                argument_hint: Some("[search_query]"),
                action: CommandAction::Memory,
            },
            CommandEntry {
                name: "theme",
                aliases: &[],
                summary: "Toggle dark/light theme",
                argument_hint: None,
                action: CommandAction::Theme,
            },
            CommandEntry {
                name: "vim",
                aliases: &[],
                summary: "Toggle vim input mode",
                argument_hint: None,
                action: CommandAction::Vim,
            },
            CommandEntry {
                name: "export",
                aliases: &[],
                summary: "Export conversation to file",
                argument_hint: Some("[path]"),
                action: CommandAction::Export,
            },
            CommandEntry {
                name: "history",
                aliases: &[],
                summary: "Show command history",
                argument_hint: None,
                action: CommandAction::History,
            },
            CommandEntry {
                name: "version",
                aliases: &["v"],
                summary: "Show nocode version",
                argument_hint: None,
                action: CommandAction::Version,
            },
            CommandEntry {
                name: "bug",
                aliases: &[],
                summary: "Report a bug",
                argument_hint: None,
                action: CommandAction::Bug,
            },
            CommandEntry {
                name: "doctor",
                aliases: &[],
                summary: "Run diagnostics and health checks",
                argument_hint: None,
                action: CommandAction::Doctor,
            },
            CommandEntry {
                name: "permissions",
                aliases: &["perms"],
                summary: "Show or change permission mode",
                argument_hint: Some("[mode]"),
                action: CommandAction::Permissions,
            },
            CommandEntry {
                name: "cost",
                aliases: &[],
                summary: "Show token usage and estimated cost",
                argument_hint: None,
                action: CommandAction::Cost,
            },
            CommandEntry {
                name: "init",
                aliases: &[],
                summary: "Initialize CLAUDE.md in current project",
                argument_hint: None,
                action: CommandAction::Init,
            },
            CommandEntry {
                name: "login",
                aliases: &[],
                summary: "Configure API key",
                argument_hint: None,
                action: CommandAction::Login,
            },
            CommandEntry {
                name: "plan",
                aliases: &["ultraplan"],
                summary: "Enter plan mode for structured task planning",
                argument_hint: Some("[description]"),
                action: CommandAction::Plan,
            },
            CommandEntry {
                name: "review",
                aliases: &["ultrareview"],
                summary: "Review code changes (staged or working tree)",
                argument_hint: Some("[path|--staged]"),
                action: CommandAction::Review,
            },
            CommandEntry {
                name: "skills",
                aliases: &[],
                summary: "List available skills and slash commands",
                argument_hint: None,
                action: CommandAction::Skills,
            },
            CommandEntry {
                name: "env",
                aliases: &[],
                summary: "Show relevant environment variables",
                argument_hint: None,
                action: CommandAction::Env,
            },
            CommandEntry {
                name: "keybindings",
                aliases: &["keys"],
                summary: "Show keyboard shortcuts",
                argument_hint: None,
                action: CommandAction::Keybindings,
            },
            CommandEntry {
                name: "bughunter",
                aliases: &[],
                summary: "Scan project for common bugs and issues",
                argument_hint: Some("[path]"),
                action: CommandAction::BugHunter,
            },
            CommandEntry {
                name: "security-review",
                aliases: &["secreview"],
                summary: "Security review of project code",
                argument_hint: Some("[path]"),
                action: CommandAction::SecurityReview,
            },
            CommandEntry {
                name: "mcp-add",
                aliases: &[],
                summary: "Add and connect an MCP server",
                argument_hint: Some("<name> <command> [args...]"),
                action: CommandAction::McpAdd,
            },
            CommandEntry {
                name: "mcp-remove",
                aliases: &["mcp-rm"],
                summary: "Disconnect and remove an MCP server",
                argument_hint: Some("<name>"),
                action: CommandAction::McpRemove,
            },
            CommandEntry {
                name: "mcp-restart",
                aliases: &[],
                summary: "Restart an MCP server connection",
                argument_hint: Some("<name>"),
                action: CommandAction::McpRestart,
            },
            CommandEntry {
                name: "insights",
                aliases: &[],
                summary: "Show session insights and statistics",
                argument_hint: None,
                action: CommandAction::Insights,
            },
        ];

        let mut lookup = HashMap::new();
        for (i, cmd) in commands.iter().enumerate() {
            lookup.insert(format!("/{}", cmd.name), i);
            for alias in cmd.aliases {
                lookup.insert(format!("/{alias}"), i);
            }
        }

        Self { commands, lookup }
    }

    /// Look up a command by input string (e.g. "/quit", "/q").
    /// Returns the action and any trailing argument text.
    pub fn resolve(&self, input: &str) -> Option<(CommandAction, Option<String>)> {
        let trimmed = input.trim();
        if !trimmed.starts_with('/') {
            return None;
        }

        // Split into command and args: "/resume abc123" → ("/resume", "abc123")
        let (cmd_str, args) = match trimmed.find(' ') {
            Some(pos) => (&trimmed[..pos], Some(trimmed[pos + 1..].trim().to_string())),
            None => (trimmed, None),
        };

        let cmd_lower = cmd_str.to_lowercase();
        self.lookup.get(&cmd_lower).map(|&idx| {
            let action = self.commands[idx].action.clone();
            (action, args.filter(|a| !a.is_empty()))
        })
    }

    /// Get all commands for help display.
    pub fn all_commands(&self) -> &[CommandEntry] {
        &self.commands
    }

    /// Format help text for all commands.
    pub fn help_text(&self) -> String {
        let mut lines = Vec::new();
        lines.push("Available commands:".to_string());
        lines.push(String::new());
        for cmd in &self.commands {
            let mut line = format!("  /{:<14}", cmd.name);
            if let Some(hint) = cmd.argument_hint {
                line.push_str(&format!(" {hint:<16}"));
            } else {
                line.push_str(&" ".repeat(17));
            }
            line.push_str(cmd.summary);
            if !cmd.aliases.is_empty() {
                let aliases: Vec<String> = cmd.aliases.iter().map(|a| format!("/{a}")).collect();
                line.push_str(&format!("  ({})", aliases.join(", ")));
            }
            lines.push(line);
        }
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_primary_name() {
        let reg = CommandRegistry::with_defaults();
        let (action, args) = reg.resolve("/quit").unwrap();
        assert_eq!(action, CommandAction::Quit);
        assert!(args.is_none());
    }

    #[test]
    fn resolve_alias() {
        let reg = CommandRegistry::with_defaults();
        let (action, _) = reg.resolve("/q").unwrap();
        assert_eq!(action, CommandAction::Quit);
        let (action, _) = reg.resolve("/exit").unwrap();
        assert_eq!(action, CommandAction::Quit);
    }

    #[test]
    fn resolve_with_args() {
        let reg = CommandRegistry::with_defaults();
        let (action, args) = reg.resolve("/resume abc123").unwrap();
        assert_eq!(action, CommandAction::Resume);
        assert_eq!(args.as_deref(), Some("abc123"));
    }

    #[test]
    fn resolve_case_insensitive() {
        let reg = CommandRegistry::with_defaults();
        let (action, _) = reg.resolve("/QUIT").unwrap();
        assert_eq!(action, CommandAction::Quit);
    }

    #[test]
    fn resolve_unknown_returns_none() {
        let reg = CommandRegistry::with_defaults();
        assert!(reg.resolve("/nonexistent").is_none());
    }

    #[test]
    fn resolve_non_slash_returns_none() {
        let reg = CommandRegistry::with_defaults();
        assert!(reg.resolve("quit").is_none());
    }

    #[test]
    fn help_text_contains_all_commands() {
        let reg = CommandRegistry::with_defaults();
        let help = reg.help_text();
        assert!(help.contains("/quit"));
        assert!(help.contains("/help"));
        assert!(help.contains("/model"));
        assert!(help.contains("/sessions"));
        assert!(help.contains("/mcp"));
        assert!(help.contains("/agents"));
    }

    #[test]
    fn all_commands_count() {
        let reg = CommandRegistry::with_defaults();
        assert!(reg.all_commands().len() >= 20);
    }

    #[test]
    fn resolve_empty_args_is_none() {
        let reg = CommandRegistry::with_defaults();
        let (_, args) = reg.resolve("/resume   ").unwrap();
        assert!(args.is_none());
    }
}
