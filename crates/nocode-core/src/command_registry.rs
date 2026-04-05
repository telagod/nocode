/// Slash-command manifest: types, registry, and default command set.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandSource {
    Builtin,
    InternalOnly,
    FeatureGated,
    Plugin,
}

impl CommandSource {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Builtin => "Built-in",
            Self::InternalOnly => "Internal",
            Self::FeatureGated => "Feature-gated",
            Self::Plugin => "Plugin",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SlashCommandSpec {
    pub name: String,
    pub aliases: Vec<String>,
    pub summary: String,
    pub argument_hint: Option<String>,
    pub source: CommandSource,
    pub resume_supported: bool,
}

#[derive(Debug, Clone, Default)]
pub struct CommandRegistry {
    commands: Vec<SlashCommandSpec>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, spec: SlashCommandSpec) {
        self.commands.push(spec);
    }

    /// Find a command by its canonical name or any alias.
    pub fn find(&self, name: &str) -> Option<&SlashCommandSpec> {
        let normalized = name.strip_prefix('/').unwrap_or(name);
        self.commands.iter().find(|spec| {
            spec.name == normalized
                || spec.aliases.iter().any(|a| a == normalized)
        })
    }

    pub fn list(&self) -> &[SlashCommandSpec] {
        &self.commands
    }

    pub fn list_by_source(&self, source: CommandSource) -> Vec<&SlashCommandSpec> {
        self.commands.iter().filter(|s| s.source == source).collect()
    }

    pub fn render_help(&self) -> String {
        let mut sections: Vec<(CommandSource, Vec<String>)> = Vec::new();
        for &src in &[
            CommandSource::Builtin,
            CommandSource::FeatureGated,
            CommandSource::Plugin,
            CommandSource::InternalOnly,
        ] {
            let cmds = self.list_by_source(src);
            if cmds.is_empty() {
                continue;
            }
            let lines: Vec<String> = cmds
                .iter()
                .map(|c| {
                    let prefix = format!("/{}", c.name);
                    match &c.argument_hint {
                        Some(hint) => format!("  {prefix} {hint}  — {}", c.summary),
                        None => format!("  {prefix}  — {}", c.summary),
                    }
                })
                .collect();
            sections.push((src, lines));
        }
        let mut out = String::from("commands:\n");
        for (i, (src, lines)) in sections.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&format!("[{}]\n", src.label()));
            for line in lines {
                out.push_str(line);
                out.push('\n');
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Builder helper
// ---------------------------------------------------------------------------

fn builtin(name: &str, summary: &str) -> SlashCommandSpec {
    SlashCommandSpec {
        name: name.to_string(),
        aliases: Vec::new(),
        summary: summary.to_string(),
        argument_hint: None,
        source: CommandSource::Builtin,
        resume_supported: false,
    }
}

fn builtin_arg(name: &str, summary: &str, hint: &str) -> SlashCommandSpec {
    SlashCommandSpec {
        name: name.to_string(),
        aliases: Vec::new(),
        summary: summary.to_string(),
        argument_hint: Some(hint.to_string()),
        source: CommandSource::Builtin,
        resume_supported: false,
    }
}

fn builtin_alias(name: &str, aliases: &[&str], summary: &str) -> SlashCommandSpec {
    SlashCommandSpec {
        name: name.to_string(),
        aliases: aliases.iter().map(|a| (*a).to_string()).collect(),
        summary: summary.to_string(),
        argument_hint: None,
        source: CommandSource::Builtin,
        resume_supported: false,
    }
}

/// Returns a [`CommandRegistry`] pre-populated with every slash command
/// currently hard-coded in `repl.rs`.
pub fn default_command_registry() -> CommandRegistry {
    let mut r = CommandRegistry::new();

    // -- general --
    r.register(builtin("help", "Show available commands"));
    r.register(builtin("status", "Show session status"));
    r.register(builtin("runtime", "Show runtime info"));
    r.register(builtin("history", "Show command history"));
    r.register(builtin("inputs", "Show raw input history"));
    r.register(builtin("quit", "Exit nocode"));

    // -- navigation / TUI --
    r.register(builtin_arg("focus", "Focus TUI pane", "<transcript|tasks|detail>"));
    r.register(builtin("tasks-next", "Next task page"));
    r.register(builtin("tasks-prev", "Previous task page"));
    r.register(builtin_alias("j", &["down"], "Pane cursor down"));
    r.register(builtin_alias("k", &["up"], "Pane cursor up"));
    r.register(builtin("enter", "Activate selected pane item"));

    // -- tasks --
    r.register(builtin_arg("tasks", "List tasks", "[filter]"));
    r.register(builtin("task-queue", "Show task queue"));
    r.register(builtin_arg("task-shell", "Run shell task", "<command>"));
    r.register(builtin_arg(
        "task-agent",
        "Run agent task",
        "<agent-id> <prompt>",
    ));
    r.register(builtin_arg(
        "task-dream",
        "Run dream task",
        "[sessions] [description]",
    ));
    r.register(builtin_arg(
        "task-show",
        "Show task detail",
        "<task-id|first|last|latest|prev|next>",
    ));
    r.register(builtin("task-open", "Open task in editor"));
    r.register(builtin("task-run-next", "Run next queued task"));
    r.register(builtin("task-run-all", "Run all queued tasks"));
    r.register(builtin_arg("task-stop", "Stop a running task", "<task-id>"));

    // -- drafting / editing --
    r.register(builtin_arg("draft", "Set draft text", "<text>"));
    r.register(builtin_alias("edit", &[], "Set draft text (alias of draft)"));
    r.register(builtin_arg("append", "Append to draft", "<text>"));
    r.register(builtin("send", "Submit current draft"));

    // -- history navigation --
    r.register(builtin("history-prev", "Previous history entry"));
    r.register(builtin("history-next", "Next history entry"));

    // -- queue --
    r.register(builtin_arg("queue", "Queue a prompt", "<prompt>"));
    r.register(builtin_arg("queue-slash", "Queue a slash command", "</command>"));
    r.register(builtin("queue-show", "Show queued commands"));

    // -- git --
    r.register(builtin_arg("commit", "Create git commit", "<message>"));
    r.register(builtin_arg("diff", "Show git diff", "[args]"));
    r.register(builtin_arg("branch", "Show/create branch", "[name]"));

    // -- auth --
    r.register(builtin_arg("login", "Set API key", "[key]"));
    r.register(builtin("logout", "Clear credentials"));

    // -- diagnostics --
    r.register(builtin("doctor", "System diagnostics"));
    r.register(builtin("ide", "IDE integration info"));

    // -- plugins --
    r.register(builtin_arg("plugin", "Manage plugins", "[args]"));

    // -- teams --
    r.register(builtin_arg(
        "team-create",
        "Create team with parallel subtasks",
        "<task-description>",
    ));
    r.register(builtin("team-status", "Show team status"));

    r
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_by_name() {
        let reg = default_command_registry();
        let spec = reg.find("help").expect("help should exist");
        assert_eq!(spec.name, "help");
    }

    #[test]
    fn find_by_name_with_slash_prefix() {
        let reg = default_command_registry();
        let spec = reg.find("/quit").expect("/quit should resolve");
        assert_eq!(spec.name, "quit");
    }

    #[test]
    fn find_by_alias() {
        let reg = default_command_registry();
        let spec = reg.find("down").expect("alias 'down' should resolve to 'j'");
        assert_eq!(spec.name, "j");
        let spec2 = reg.find("up").expect("alias 'up' should resolve to 'k'");
        assert_eq!(spec2.name, "k");
    }

    #[test]
    fn list_by_source_filters() {
        let mut reg = CommandRegistry::new();
        reg.register(builtin("a", "cmd a"));
        reg.register(SlashCommandSpec {
            name: "b".to_string(),
            aliases: Vec::new(),
            summary: "cmd b".to_string(),
            argument_hint: None,
            source: CommandSource::Plugin,
            resume_supported: false,
        });
        reg.register(builtin("c", "cmd c"));

        let builtins = reg.list_by_source(CommandSource::Builtin);
        assert_eq!(builtins.len(), 2);
        let plugins = reg.list_by_source(CommandSource::Plugin);
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].name, "b");
    }

    #[test]
    fn render_help_groups_by_source() {
        let mut reg = CommandRegistry::new();
        reg.register(builtin("x", "do x"));
        reg.register(SlashCommandSpec {
            name: "y".to_string(),
            aliases: Vec::new(),
            summary: "do y".to_string(),
            argument_hint: None,
            source: CommandSource::Plugin,
            resume_supported: false,
        });
        let help = reg.render_help();
        assert!(help.contains("[Built-in]"));
        assert!(help.contains("[Plugin]"));
        assert!(help.contains("/x"));
        assert!(help.contains("/y"));
        // Built-in should appear before Plugin
        let bi_pos = help.find("[Built-in]").unwrap();
        let pl_pos = help.find("[Plugin]").unwrap();
        assert!(bi_pos < pl_pos);
    }

    #[test]
    fn default_registry_has_core_commands() {
        let reg = default_command_registry();
        let names: Vec<&str> = reg.list().iter().map(|s| s.name.as_str()).collect();
        for expected in &[
            "help", "status", "runtime", "history", "quit", "commit",
            "diff", "branch", "task-shell", "task-agent", "task-dream",
            "tasks", "team-create", "team-status", "login", "logout",
            "doctor", "draft", "edit", "append", "send", "queue",
        ] {
            assert!(names.contains(expected), "missing command: {expected}");
        }
    }

    #[test]
    fn unknown_command_returns_none() {
        let reg = default_command_registry();
        assert!(reg.find("nonexistent-xyz").is_none());
    }
}
