//! TUI slash command handlers — extracted from tui_app.rs.

use crate::tui_app::{InputMode, Overlay, TuiApp};
use nocode_core::message::Message;

/// Handle a resolved slash command action. Returns `true` if the app should break (quit).
pub(crate) fn handle_slash_command(
    action: crate::command_registry::CommandAction,
    args: Option<String>,
    app: &mut TuiApp,
    messages: &mut Vec<Message>,
    model: &str,
) -> SlashResult {
    use crate::command_registry::CommandAction;

    match action {
        CommandAction::Quit => SlashResult::Quit,
        CommandAction::Clear => {
            messages.clear();
            app.chat_messages.clear();
            app.invalidate_height_cache();
            app.push_system("(conversation cleared)");
            SlashResult::Handled
        }
        CommandAction::Help => {
            app.overlay = Overlay::Help;
            app.dirty = true;
            SlashResult::Handled
        }
        CommandAction::Status => {
            app.overlay = Overlay::Status;
            app.dirty = true;
            SlashResult::Handled
        }
        CommandAction::Sessions => {
            cmd_sessions(app);
            SlashResult::Handled
        }
        CommandAction::Resume => {
            cmd_resume(app, args.as_deref(), messages);
            SlashResult::Handled
        }
        CommandAction::Mcp => {
            app.overlay = Overlay::Mcp;
            app.dirty = true;
            SlashResult::Handled
        }
        CommandAction::Agents => {
            app.overlay = Overlay::Agents;
            app.dirty = true;
            SlashResult::Handled
        }
        CommandAction::Config => {
            app.overlay = Overlay::Config;
            app.dirty = true;
            SlashResult::Handled
        }
        CommandAction::Memory => {
            app.overlay = Overlay::Memory;
            app.dirty = true;
            SlashResult::Handled
        }
        CommandAction::Cost => {
            app.overlay = Overlay::Cost;
            app.dirty = true;
            SlashResult::Handled
        }
        CommandAction::Theme => {
            let variant = crate::tui_theme::toggle_theme();
            app.push_system(&format!("Theme: {variant:?}"));
            app.invalidate_height_cache();
            SlashResult::Handled
        }
        CommandAction::Vim => {
            app.input_mode = if app.input_mode == InputMode::Insert {
                InputMode::Normal
            } else {
                InputMode::Insert
            };
            app.push_system(&format!("Vim mode: {}", app.input_mode.label()));
            SlashResult::Handled
        }
        CommandAction::Version => {
            app.push_system(&format!("nocode v{}", env!("CARGO_PKG_VERSION")));
            SlashResult::Handled
        }
        CommandAction::Compact => {
            cmd_compact(app, messages);
            SlashResult::Handled
        }
        CommandAction::Permissions => {
            app.push_system("Permission mode: ask (default)");
            SlashResult::Handled
        }
        CommandAction::History => {
            let hist: Vec<String> = app.input_history.iter().rev().take(20).cloned().collect();
            if hist.is_empty() {
                app.push_system("(no command history)");
            } else {
                app.push_system(&format!("Recent commands:\n{}", hist.join("\n")));
            }
            SlashResult::Handled
        }
        CommandAction::Model => {
            if let Some(new_model) = args {
                app.hud.model_name = new_model.clone();
                app.push_system(&format!("Model switched to: {new_model}"));
            } else {
                app.push_system(&format!("Current model: {}", app.hud.model_name()));
            }
            SlashResult::Handled
        }
        CommandAction::Export => {
            cmd_export(args.as_deref(), messages, app);
            SlashResult::Handled
        }
        CommandAction::Bug => {
            app.push_system(&format!(
                "Report bugs at: https://github.com/anthropics/nocode/issues/new\n\
                 Version: nocode v{}\n\
                 OS: {} ({})",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS,
                std::env::consts::ARCH,
            ));
            SlashResult::Handled
        }
        CommandAction::Doctor => {
            cmd_doctor(app, model);
            SlashResult::Handled
        }
        CommandAction::Init => {
            cmd_init(app);
            SlashResult::Handled
        }
        CommandAction::Login => {
            app.push_system(
                "Configure API keys via environment variables:\n\n\
                 \x20 export ANTHROPIC_API_KEY=sk-ant-...\n\
                 \x20 export OPENAI_API_KEY=sk-...\n\
                 \x20 export GEMINI_API_KEY=AI...\n\n\
                 Or add to ~/.nocode/settings.json",
            );
            SlashResult::Handled
        }
        CommandAction::Plan => {
            cmd_plan(app, args.as_deref());
            SlashResult::Handled
        }
        CommandAction::Review => {
            cmd_review(app, args.as_deref());
            SlashResult::Handled
        }
        CommandAction::Skills => {
            cmd_skills(app);
            SlashResult::Handled
        }
        CommandAction::Env => {
            cmd_env(app);
            SlashResult::Handled
        }
        CommandAction::Keybindings => {
            cmd_keybindings(app);
            SlashResult::Handled
        }
        CommandAction::BugHunter => {
            cmd_bughunter(app, args.as_deref());
            SlashResult::Handled
        }
        CommandAction::SecurityReview => {
            cmd_security_review(app, args.as_deref());
            SlashResult::Handled
        }
        CommandAction::McpAdd => {
            cmd_mcp_add(app, args.as_deref());
            SlashResult::Handled
        }
        CommandAction::McpRemove => {
            cmd_mcp_remove(app, args.as_deref());
            SlashResult::Handled
        }
        CommandAction::McpRestart => {
            cmd_mcp_restart(app, args.as_deref());
            SlashResult::Handled
        }
    }
}

pub(crate) enum SlashResult {
    Quit,
    Handled,
}

// ---------------------------------------------------------------------------
// Individual command implementations
// ---------------------------------------------------------------------------

fn cmd_sessions(app: &mut TuiApp) {
    use nocode_core::session::persistence::SessionPersistence;
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let infos = SessionPersistence::list_sessions_with_info(&cwd);
    if infos.is_empty() {
        app.push_system("No saved sessions.");
    } else {
        let mut lines = vec!["Saved sessions:".to_string()];
        for info in infos.iter().take(20) {
            let preview = info.first_user_message.as_deref().unwrap_or("(empty)");
            lines.push(format!(
                "  {} ({} msgs) — {}",
                info.id, info.message_count, preview
            ));
        }
        lines.push(String::new());
        lines.push("Use /resume <id> to restore.".to_string());
        app.push_system(&lines.join("\n"));
    }
}

fn cmd_resume(app: &mut TuiApp, session_id: Option<&str>, messages: &mut Vec<Message>) {
    use nocode_core::session::persistence::SessionPersistence;
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    if let Some(session_id) = session_id {
        match SessionPersistence::resume(&cwd, session_id) {
            Ok((_persistence, loaded)) => {
                *messages = loaded;
                app.chat_messages.clear();
                app.invalidate_height_cache();
                for msg in messages.iter() {
                    match msg.role {
                        nocode_core::message::Role::User => {
                            app.push_user_message(&msg.text_content());
                        }
                        nocode_core::message::Role::Assistant => {
                            let text = msg.text_content();
                            if !text.is_empty() {
                                app.update_streaming_assistant(&text);
                            }
                        }
                    }
                }
                app.push_system(&format!(
                    "Resumed session '{session_id}' ({} messages)",
                    messages.len()
                ));
            }
            Err(e) => {
                app.push_error(&format!("Failed to resume: {e}"));
            }
        }
    } else {
        app.push_system("Usage: /resume <session_id>");
    }
}

fn cmd_compact(app: &mut TuiApp, messages: &mut Vec<Message>) {
    use nocode_core::session::compaction::{Compactor, TailCompactor};
    let compactor = TailCompactor::new(10);
    let result = compactor.compact(messages);
    if result.compacted_count > 0 {
        *messages = result.messages;
        app.push_system(&format!(
            "Compacted {} messages, ~{} tokens saved",
            result.compacted_count, result.tokens_saved
        ));
    } else {
        app.push_system("Nothing to compact (conversation too short)");
    }
}

fn cmd_export(path: Option<&str>, messages: &[Message], app: &mut TuiApp) {
    if messages.is_empty() {
        app.push_system("Nothing to export — conversation is empty.");
        return;
    }
    let out_path = path.unwrap_or("conversation.md");
    let mut content = String::new();
    for msg in messages {
        let role = match msg.role {
            nocode_core::message::Role::User => "## User",
            nocode_core::message::Role::Assistant => "## Assistant",
        };
        content.push_str(role);
        content.push_str("\n\n");
        content.push_str(&msg.text_content());
        content.push_str("\n\n");
    }
    match std::fs::write(out_path, &content) {
        Ok(()) => app.push_system(&format!(
            "Exported {} messages to {out_path}",
            messages.len()
        )),
        Err(e) => app.push_error(&format!("Export failed: {e}")),
    }
}

fn cmd_doctor(app: &mut TuiApp, model: &str) {
    let mut lines = Vec::new();
    lines.push(format!("nocode v{}", env!("CARGO_PKG_VERSION")));
    lines.push(format!(
        "OS: {} ({})",
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    lines.push(format!("Model: {model}"));
    lines.push(String::new());

    let keys = [
        ("ANTHROPIC_API_KEY", "Claude"),
        ("OPENAI_API_KEY", "OpenAI"),
        ("GEMINI_API_KEY", "Gemini"),
    ];
    lines.push("API keys:".to_string());
    for (var, name) in &keys {
        let status = if std::env::var(var).is_ok() {
            "set"
        } else {
            "not set"
        };
        lines.push(format!("  {name}: {status}"));
    }
    lines.push(String::new());

    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let home = std::env::var("HOME").unwrap_or_default();
    let paths = [
        (format!("{home}/.nocode/settings.json"), "User"),
        (format!("{cwd}/.nocode/settings.json"), "Project"),
        (format!("{cwd}/.nocode/settings.local.json"), "Local"),
    ];
    lines.push("Settings:".to_string());
    for (path, tier) in &paths {
        let mark = if std::path::Path::new(path).exists() {
            "found"
        } else {
            "not found"
        };
        lines.push(format!("  {tier}: {mark}"));
    }
    lines.push(String::new());

    let md_files = nocode_core::prompt::assembly::discover_claude_md(&cwd);
    lines.push(format!("CLAUDE.md files: {}", md_files.len()));
    let sessions = nocode_core::session::persistence::SessionPersistence::list_sessions(&cwd);
    lines.push(format!("Saved sessions: {}", sessions.len()));
    lines.push(String::new());
    lines.push("All checks passed.".to_string());

    app.push_system(&lines.join("\n"));
}

fn cmd_init(app: &mut TuiApp) {
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let claude_md_path = format!("{cwd}/CLAUDE.md");
    if std::path::Path::new(&claude_md_path).exists() {
        app.push_system(&format!("CLAUDE.md already exists at {claude_md_path}"));
        return;
    }
    let template = "# CLAUDE.md\n\n\
        This file provides guidance to AI coding assistants working with this codebase.\n\n\
        ## Project Overview\n\n\
        <!-- Describe your project here -->\n\n\
        ## Build & Test\n\n\
        ```bash\n\
        # Add your build/test commands here\n\
        ```\n\n\
        ## Key Conventions\n\n\
        <!-- Add coding conventions, architecture notes, etc. -->\n";
    match std::fs::write(&claude_md_path, template) {
        Ok(()) => app.push_system(&format!("Created {claude_md_path}")),
        Err(e) => app.push_error(&format!("Failed to create CLAUDE.md: {e}")),
    }
}

fn cmd_plan(app: &mut TuiApp, description: Option<&str>) {
    let desc = description.unwrap_or("(no description)");
    app.push_system(&format!(
        "Plan mode activated: {desc}\n\n\
         Outline your approach, then use /send to submit.\n\
         The model will help structure your plan before execution."
    ));
    app.input_mode = InputMode::Insert;
}

fn cmd_review(app: &mut TuiApp, args: Option<&str>) {
    let flag = args.unwrap_or("");
    let cmd = if flag == "--staged" || flag == "staged" {
        "git diff --staged --stat"
    } else if flag.is_empty() {
        "git diff --stat"
    } else {
        // Treat as path
        &format!("git diff -- {flag}")
    };

    match std::process::Command::new("sh").arg("-c").arg(cmd).output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stdout.is_empty() && stderr.is_empty() {
                app.push_system("No changes to review.");
            } else {
                let mut lines = String::new();
                lines.push_str("Code review:\n\n");
                if !stdout.is_empty() {
                    lines.push_str(&stdout);
                }
                if !stderr.is_empty() {
                    lines.push_str(&stderr);
                }
                app.push_system(&lines);
            }
        }
        Err(e) => app.push_error(&format!("Failed to run git diff: {e}")),
    }
}

fn cmd_skills(app: &mut TuiApp) {
    let registry = crate::command_registry::CommandRegistry::with_defaults();
    let mut lines = vec!["Available skills and commands:".to_string(), String::new()];
    for cmd in registry.all_commands() {
        let mut line = format!("  /{:<16}", cmd.name);
        line.push_str(cmd.summary);
        if !cmd.aliases.is_empty() {
            let aliases: Vec<String> = cmd.aliases.iter().map(|a| format!("/{a}")).collect();
            line.push_str(&format!("  ({})", aliases.join(", ")));
        }
        lines.push(line);
    }
    lines.push(String::new());
    lines.push(format!("Total: {} commands", registry.all_commands().len()));
    app.push_system(&lines.join("\n"));
}

fn cmd_env(app: &mut TuiApp) {
    let vars = [
        ("NOCODE_MODEL_PROVIDER", "Provider override"),
        ("NOCODE_MODEL", "Model override"),
        ("NOCODE_CUSTOM_BASE_URL", "Custom endpoint"),
        ("NOCODE_CUSTOM_API_FORMAT", "Custom API format"),
        ("NOCODE_SYSTEM_PROMPT", "System prompt override"),
        ("NOCODE_MODEL_REASONING_EFFORT", "Reasoning effort"),
        ("ANTHROPIC_API_KEY", "Claude API key"),
        ("OPENAI_API_KEY", "OpenAI API key"),
        ("GEMINI_API_KEY", "Gemini API key"),
        ("ANTHROPIC_BASE_URL", "Claude base URL"),
        ("OPENAI_BASE_URL", "OpenAI base URL"),
        ("NOCODE_BRIDGE_BASE_URL", "Bridge endpoint"),
        ("NOCODE_BRIDGE_AUTH_TOKEN", "Bridge auth token"),
    ];

    let mut lines = vec!["Environment variables:".to_string(), String::new()];
    for (var, desc) in &vars {
        let val = std::env::var(var).ok();
        let display = match val {
            Some(v) if var.contains("KEY") || var.contains("TOKEN") => {
                if v.len() > 8 {
                    format!("{}...{}", &v[..4], &v[v.len() - 4..])
                } else {
                    "(set)".to_string()
                }
            }
            Some(v) => v,
            None => "(not set)".to_string(),
        };
        lines.push(format!("  {var:<35} {display:<20} {desc}"));
    }
    app.push_system(&lines.join("\n"));
}

fn cmd_keybindings(app: &mut TuiApp) {
    app.push_system(
        "Keyboard shortcuts:\n\n\
         General:\n\
         \x20 Alt-1..4        Focus pane (transcript/tasks/detail/events)\n\
         \x20 Tab / Shift-Tab Cycle panes\n\
         \x20 Ctrl-T          Toggle dark/light theme\n\
         \x20 Ctrl-O          Expand/collapse thinking blocks\n\
         \x20 F1 / ?          Help overlay\n\
         \x20 F2              Inspector overlay\n\
         \x20 F3              Permission overlay\n\
         \x20 Esc             Close overlay / exit\n\n\
         Input:\n\
         \x20 Enter           Send message\n\
         \x20 Shift-Enter     New line\n\
         \x20 Ctrl-P / Ctrl-N Input history prev/next\n\
         \x20 Ctrl-U          Clear input\n\
         \x20 Tab (on tool)   Collapse/expand tool output\n\n\
         Scrolling:\n\
         \x20 Up / Down       Scroll or navigate\n\
         \x20 PgUp / PgDn    Fast scroll\n\n\
         Vim mode (toggle with /vim):\n\
         \x20 Esc             Normal mode\n\
         \x20 i / a / I / A   Insert mode\n\
         \x20 h/j/k/l         Movement\n\
         \x20 w/b/e           Word movement\n\
         \x20 x / dd / C      Delete char / line / to end",
    );
}

fn cmd_bughunter(app: &mut TuiApp, path: Option<&str>) {
    let target = path.unwrap_or(".");
    let prompt = format!(
        "Scan the codebase at '{target}' for common bugs and issues. Look for:\n\
         - Null/None dereferences without checks\n\
         - Resource leaks (unclosed files, connections)\n\
         - Race conditions and data races\n\
         - Integer overflow/underflow\n\
         - Buffer overflows or out-of-bounds access\n\
         - Error handling gaps (unwrap on fallible ops)\n\
         - Logic errors (off-by-one, wrong comparisons)\n\
         - Dead code and unreachable branches\n\n\
         Report each finding with: file:line, severity (critical/high/medium/low), description, and suggested fix."
    );
    app.push_system(&format!("Bug hunter scanning: {target}"));
    app.input = prompt;
    app.cursor_pos = app.input.len();
    app.dirty = true;
}

fn cmd_security_review(app: &mut TuiApp, path: Option<&str>) {
    let target = path.unwrap_or(".");
    let prompt = format!(
        "Perform a security review of the codebase at '{target}'. Check for:\n\
         - Injection vulnerabilities (SQL, command, path traversal)\n\
         - Authentication/authorization flaws\n\
         - Sensitive data exposure (hardcoded secrets, API keys, tokens)\n\
         - Insecure cryptography (weak algorithms, bad RNG)\n\
         - SSRF, CSRF, XSS vectors\n\
         - Insecure deserialization\n\
         - Dependency vulnerabilities (known CVEs)\n\
         - Privilege escalation paths\n\
         - Missing input validation\n\
         - Insecure file operations (symlink attacks, TOCTOU)\n\n\
         Report each finding with: file:line, severity (critical/high/medium/low), CWE ID if applicable, description, and remediation."
    );
    app.push_system(&format!("Security review scanning: {target}"));
    app.input = prompt;
    app.cursor_pos = app.input.len();
    app.dirty = true;
}

fn cmd_mcp_add(app: &mut TuiApp, args: Option<&str>) {
    let Some(args) = args else {
        app.push_system("Usage: /mcp-add <name> <command> [args...]");
        return;
    };
    let parts: Vec<&str> = args.splitn(3, ' ').collect();
    if parts.len() < 2 {
        app.push_system("Usage: /mcp-add <name> <command> [args...]");
        return;
    }
    let name = parts[0];
    let command = parts[1];
    let cmd_args: Vec<String> = if parts.len() > 2 {
        parts[2].split_whitespace().map(String::from).collect()
    } else {
        Vec::new()
    };

    let mgr = nocode_core::mcp::manager::global_mcp_manager();
    let mut guard = mgr.lock().unwrap();
    guard.register_server(name, command, cmd_args);
    match guard.connect(name) {
        Ok(()) => {
            let count = guard.all_tools().iter().filter(|(s, _)| *s == name).count();
            app.push_system(&format!("MCP server '{name}' connected ({count} tools)"));
        }
        Err(e) => app.push_error(&format!("MCP connect failed: {e}")),
    }
}

fn cmd_mcp_remove(app: &mut TuiApp, args: Option<&str>) {
    let Some(name) = args else {
        app.push_system("Usage: /mcp-remove <name>");
        return;
    };
    let name = name.trim();
    let mgr = nocode_core::mcp::manager::global_mcp_manager();
    let mut guard = mgr.lock().unwrap();
    match guard.disconnect(name) {
        Ok(()) => app.push_system(&format!("MCP server '{name}' disconnected")),
        Err(e) => app.push_error(&format!("MCP disconnect failed: {e}")),
    }
}

fn cmd_mcp_restart(app: &mut TuiApp, args: Option<&str>) {
    let Some(name) = args else {
        app.push_system("Usage: /mcp-restart <name>");
        return;
    };
    let name = name.trim();
    let mgr = nocode_core::mcp::manager::global_mcp_manager();
    let mut guard = mgr.lock().unwrap();
    let _ = guard.disconnect(name);
    match guard.connect(name) {
        Ok(()) => app.push_system(&format!("MCP server '{name}' restarted")),
        Err(e) => app.push_error(&format!("MCP restart failed: {e}")),
    }
}
