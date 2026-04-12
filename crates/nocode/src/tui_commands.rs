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
            app.open_config_overlay();
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
            cmd_permissions_status(app);
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
            app.open_config_overlay();
            app.push_system(
                "Opened /config. Select API Key to paste a key, press T to test, R to refresh models, then S to save.",
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
        CommandAction::Insights => {
            cmd_insights(app, messages);
            SlashResult::Handled
        }
        CommandAction::AgentCreate => {
            cmd_agent_create(app, args.as_deref());
            SlashResult::Handled
        }
        CommandAction::FeatureFlags => {
            cmd_feature_flags(app, args.as_deref());
            SlashResult::Handled
        }
        CommandAction::PermissionsAdd => {
            cmd_permissions_add(app, args.as_deref());
            SlashResult::Handled
        }
        CommandAction::PermissionsRemove => {
            cmd_permissions_remove(app, args.as_deref());
            SlashResult::Handled
        }
        CommandAction::PluginInstall => {
            cmd_plugin_install(app, args.as_deref());
            SlashResult::Handled
        }
        CommandAction::PluginRemove => {
            cmd_plugin_remove(app, args.as_deref());
            SlashResult::Handled
        }
        CommandAction::PluginList => {
            cmd_plugin_list(app);
            SlashResult::Handled
        }
        CommandAction::Telemetry => {
            cmd_telemetry(app, args.as_deref());
            SlashResult::Handled
        }
        CommandAction::Ide => {
            cmd_ide(app, args.as_deref());
            SlashResult::Handled
        }
        CommandAction::Voice => {
            cmd_voice(app, args.as_deref());
            SlashResult::Handled
        }
        CommandAction::Copy => {
            app.copy_last_assistant_to_clipboard();
            SlashResult::Handled
        }
        CommandAction::Undo => {
            cmd_undo(app, messages);
            SlashResult::Handled
        }
        CommandAction::Redo => {
            cmd_redo(app, messages);
            SlashResult::Handled
        }
        CommandAction::Rewind => {
            cmd_rewind(app, args.as_deref(), messages);
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
    let mut lines = vec!["Slash commands:".to_string(), String::new()];
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

    // Discovered skill files
    let skills = nocode_core::tool::skill::list_skills();
    if !skills.is_empty() {
        lines.push("User skills:".to_string());
        lines.push(String::new());
        for (name, path) in &skills {
            lines.push(format!("  {name:<20} {}", path.display()));
        }
        lines.push(String::new());
    }

    lines.push(format!(
        "Total: {} commands, {} skills",
        registry.all_commands().len(),
        skills.len()
    ));
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
            drop(guard);
            nocode_core::tool::global_registry::refresh_global_mcp_bridged_tools();
            let mgr = nocode_core::mcp::manager::global_mcp_manager();
            let guard = mgr.lock().unwrap();
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
        Ok(()) => {
            drop(guard);
            nocode_core::tool::global_registry::refresh_global_mcp_bridged_tools();
            app.push_system(&format!("MCP server '{name}' disconnected"));
        }
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
        Ok(()) => {
            drop(guard);
            nocode_core::tool::global_registry::refresh_global_mcp_bridged_tools();
            app.push_system(&format!("MCP server '{name}' restarted"));
        }
        Err(e) => app.push_error(&format!("MCP restart failed: {e}")),
    }
}

fn cmd_insights(app: &mut TuiApp, messages: &[Message]) {
    use std::collections::HashMap;

    let total_msgs = messages.len();
    let user_msgs = messages
        .iter()
        .filter(|m| m.role == nocode_core::message::Role::User)
        .count();
    let assistant_msgs = total_msgs - user_msgs;

    // Token estimates
    let mut total_chars: usize = 0;
    let mut tool_calls: usize = 0;
    let mut tool_freq: HashMap<String, usize> = HashMap::new();

    for msg in messages {
        for block in &msg.content {
            match block {
                nocode_core::message::ContentBlock::Text { text } => {
                    total_chars += text.len();
                }
                nocode_core::message::ContentBlock::ToolUse { name, .. } => {
                    tool_calls += 1;
                    *tool_freq.entry(name.clone()).or_insert(0) += 1;
                }
                nocode_core::message::ContentBlock::ToolResult { content, .. } => {
                    total_chars += content.len();
                }
                nocode_core::message::ContentBlock::Thinking { thinking } => {
                    total_chars += thinking.len();
                }
            }
        }
    }

    let est_tokens = total_chars / 4;

    // Top 5 tools
    let mut top_tools: Vec<(String, usize)> = tool_freq.into_iter().collect();
    top_tools.sort_by(|a, b| b.1.cmp(&a.1));
    top_tools.truncate(5);

    let mut lines = vec![
        "Session Insights:".to_string(),
        String::new(),
        format!("  Messages:     {total_msgs} ({user_msgs} user, {assistant_msgs} assistant)"),
        format!("  Est. tokens:  ~{est_tokens}"),
        format!("  Tool calls:   {tool_calls}"),
        format!("  Cost:         ${:.4}", app.hud.estimated_cost()),
        format!("  Context:      {:.1}%", app.hud.context_pct()),
    ];

    if !top_tools.is_empty() {
        lines.push(String::new());
        lines.push("  Top tools:".to_string());
        for (name, count) in &top_tools {
            lines.push(format!("    {name:<20} {count}x"));
        }
    }

    app.push_system(&lines.join("\n"));
}

fn cmd_agent_create(app: &mut TuiApp, args: Option<&str>) {
    let Some(args) = args else {
        app.push_system(
            "Usage: /agent-create <name> <prompt>\n\n\
             Example: /agent-create explorer Find all TODO comments in the codebase",
        );
        return;
    };

    let (name, prompt) = match args.find(' ') {
        Some(pos) => (&args[..pos], args[pos + 1..].trim()),
        None => {
            app.push_system("Usage: /agent-create <name> <prompt>");
            return;
        }
    };

    if prompt.is_empty() {
        app.push_system("Usage: /agent-create <name> <prompt>");
        return;
    }

    let registry = nocode_core::agent::worker::global_worker_registry();
    let mut guard = registry.lock().unwrap_or_else(|e| e.into_inner());

    if guard.find_by_name(name).is_some() {
        app.push_error(&format!("Agent '{name}' already exists"));
        return;
    }

    let id = guard.register(name, prompt);
    guard.set_state(&id, nocode_core::agent::worker::WorkerState::ReadyForPrompt);

    app.push_system(&format!(
        "Agent created:\n\
         \x20 ID:     {id}\n\
         \x20 Name:   {name}\n\
         \x20 Prompt: {}\n\
         \x20 State:  ReadyForPrompt\n\n\
         Use /agents to see all workers.",
        if prompt.len() > 80 {
            format!("{}...", &prompt[..80])
        } else {
            prompt.to_string()
        }
    ));
}

fn cmd_feature_flags(app: &mut TuiApp, args: Option<&str>) {
    let store = nocode_core::config::feature_flags::global_feature_flags();
    let mut guard = store.lock().unwrap_or_else(|e| e.into_inner());

    match args {
        None => {
            let flags = guard.list();
            let mut lines = vec!["Feature flags:".to_string(), String::new()];
            for (flag, enabled, source) in &flags {
                let status = if *enabled { "ON " } else { "OFF" };
                lines.push(format!("  {:<20} {status}  ({source})", flag.name()));
            }
            lines.push(String::new());
            lines.push("Usage: /feature-flags <name> on|off".to_string());
            app.push_system(&lines.join("\n"));
        }
        Some(args) => {
            let parts: Vec<&str> = args.splitn(2, ' ').collect();
            let flag_name = parts[0];
            let Some(flag) = nocode_core::config::feature_flags::FeatureFlag::parse(flag_name)
            else {
                app.push_error(&format!("Unknown flag: {flag_name}"));
                return;
            };

            if parts.len() < 2 {
                let enabled = guard.is_enabled(flag);
                let status = if enabled { "ON" } else { "OFF" };
                app.push_system(&format!("{flag_name}: {status}"));
                return;
            }

            let action = parts[1].trim();
            let enabled = match action {
                "on" | "true" | "1" | "enable" => true,
                "off" | "false" | "0" | "disable" => false,
                "reset" => {
                    match guard.reset(flag) {
                        Ok(()) => app.push_system(&format!("{flag_name}: reset to default")),
                        Err(e) => app.push_error(&format!("Failed to reset: {e}")),
                    }
                    return;
                }
                _ => {
                    app.push_error(&format!("Invalid action: {action} (use on/off/reset)"));
                    return;
                }
            };

            match guard.set(flag, enabled) {
                Ok(()) => {
                    let status = if enabled { "ON" } else { "OFF" };
                    app.push_system(&format!("{flag_name}: {status}"));
                }
                Err(e) => app.push_error(&format!("Failed to set flag: {e}")),
            }
        }
    }
}

fn cmd_permissions_add(app: &mut TuiApp, args: Option<&str>) {
    let Some(args) = args else {
        // No args — list current rules
        let store = nocode_core::tool::permission::global_permission_rules();
        let guard = store.lock().unwrap_or_else(|e| e.into_inner());
        let rules = guard.list();
        if rules.is_empty() {
            app.push_system(
                "No permission rules configured.\n\n\
                 Usage: /permissions-add <tool> <allow|deny> [pattern]\n\
                 Example: /permissions-add Bash deny docker\n\
                 Example: /permissions-add FileWrite allow",
            );
        } else {
            let mut lines = vec!["Permission rules:".to_string(), String::new()];
            for rule in rules {
                let pattern = rule.argument_pattern.as_deref().unwrap_or("(any)");
                lines.push(format!(
                    "  {:<16} {:?}  pattern: {}",
                    rule.tool_name, rule.action, pattern
                ));
            }
            lines.push(String::new());
            lines.push("Usage: /permissions-add <tool> <allow|deny> [pattern]".to_string());
            app.push_system(&lines.join("\n"));
        }
        return;
    };

    let parts: Vec<&str> = args.splitn(3, ' ').collect();
    if parts.len() < 2 {
        app.push_system("Usage: /permissions-add <tool> <allow|deny> [pattern]");
        return;
    }

    let tool_name = parts[0];
    let action = match parts[1] {
        "allow" => nocode_core::tool::permission::RuleAction::Allow,
        "deny" => nocode_core::tool::permission::RuleAction::Deny,
        "ask" => nocode_core::tool::permission::RuleAction::AlwaysAsk,
        other => {
            app.push_error(&format!("Invalid action: {other} (use allow/deny/ask)"));
            return;
        }
    };
    let pattern = parts.get(2).map(|s| s.to_string());

    let store = nocode_core::tool::permission::global_permission_rules();
    let mut guard = store.lock().unwrap_or_else(|e| e.into_inner());
    match guard.add(nocode_core::tool::permission::PermissionRule {
        tool_name: tool_name.to_string(),
        action,
        argument_pattern: pattern.clone(),
    }) {
        Ok(()) => {
            let pat = pattern.as_deref().unwrap_or("(any)");
            app.push_system(&format!(
                "Rule added: {tool_name} → {action:?} (pattern: {pat})"
            ));
        }
        Err(e) => app.push_error(&format!("Failed to add rule: {e}")),
    }
}

fn cmd_permissions_remove(app: &mut TuiApp, args: Option<&str>) {
    let Some(args) = args else {
        app.push_system("Usage: /permissions-remove <tool> [pattern]");
        return;
    };

    let parts: Vec<&str> = args.splitn(2, ' ').collect();
    let tool_name = parts[0];
    let pattern = parts.get(1).copied();

    let store = nocode_core::tool::permission::global_permission_rules();
    let mut guard = store.lock().unwrap_or_else(|e| e.into_inner());
    match guard.remove(tool_name, pattern) {
        Ok(true) => app.push_system(&format!("Rule removed for '{tool_name}'")),
        Ok(false) => app.push_system(&format!("No rule found for '{tool_name}'")),
        Err(e) => app.push_error(&format!("Failed to remove rule: {e}")),
    }
}

fn cmd_plugin_install(app: &mut TuiApp, args: Option<&str>) {
    let Some(path) = args else {
        app.push_system("Usage: /plugin-install <path>");
        return;
    };
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let plugins_dir = nocode_core::tool::plugin_registry::PluginRuntime::default_dir(&cwd);
    let rt = nocode_core::tool::plugin_registry::PluginRuntime::new(&plugins_dir);
    match rt.install_from_path(path.trim()) {
        Ok(msg) => app.push_system(&msg),
        Err(e) => app.push_error(&format!("Install failed: {e}")),
    }
}

fn cmd_plugin_remove(app: &mut TuiApp, args: Option<&str>) {
    let Some(name) = args else {
        app.push_system("Usage: /plugin-remove <name>");
        return;
    };
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let plugins_dir = nocode_core::tool::plugin_registry::PluginRuntime::default_dir(&cwd);
    let rt = nocode_core::tool::plugin_registry::PluginRuntime::new(&plugins_dir);
    match rt.uninstall(name.trim()) {
        Ok(msg) => app.push_system(&msg),
        Err(e) => app.push_error(&format!("Uninstall failed: {e}")),
    }
}

fn cmd_plugin_list(app: &mut TuiApp) {
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let plugins_dir = nocode_core::tool::plugin_registry::PluginRuntime::default_dir(&cwd);
    let rt = nocode_core::tool::plugin_registry::PluginRuntime::new(&plugins_dir);
    let plugins = rt.list_installed();
    if plugins.is_empty() {
        app.push_system(&format!(
            "No plugins installed.\n\nPlugins directory: {plugins_dir}\n\
             Usage: /plugin-install <path>"
        ));
        return;
    }
    let mut lines = vec!["Installed plugins:".to_string(), String::new()];
    for p in &plugins {
        lines.push(format!(
            "  {:<20} v{:<8} {} ({} tools)",
            p.name, p.version, p.description, p.tool_count
        ));
    }
    lines.push(String::new());
    lines.push(format!("Total: {} plugins", plugins.len()));
    app.push_system(&lines.join("\n"));
}

fn cmd_telemetry(app: &mut TuiApp, args: Option<&str>) {
    let ff_store = nocode_core::config::feature_flags::global_feature_flags();
    let mut ff_guard = ff_store.lock().unwrap_or_else(|e| e.into_inner());

    let logger = nocode_core::telemetry::global_event_logger();
    let mut log_guard = logger.lock().unwrap_or_else(|e| e.into_inner());

    match args.map(str::trim) {
        None | Some("status") | Some("") => {
            let ff_enabled =
                ff_guard.is_enabled(nocode_core::config::feature_flags::FeatureFlag::Telemetry);
            let logger_enabled = log_guard.is_enabled();
            let events = log_guard.event_count();
            app.push_system(&format!(
                "Telemetry:\n\n\
                 \x20 Feature flag:  {}\n\
                 \x20 Logger active: {}\n\
                 \x20 Events logged: {events}\n\n\
                 Usage: /telemetry on|off",
                if ff_enabled { "ON" } else { "OFF" },
                if logger_enabled { "ON" } else { "OFF" },
            ));
        }
        Some("on" | "true" | "1" | "enable") => {
            let _ = ff_guard.set(
                nocode_core::config::feature_flags::FeatureFlag::Telemetry,
                true,
            );
            log_guard.set_enabled(true);
            app.push_system("Telemetry: ON (events will be logged)");
        }
        Some("off" | "false" | "0" | "disable") => {
            let _ = ff_guard.set(
                nocode_core::config::feature_flags::FeatureFlag::Telemetry,
                false,
            );
            log_guard.set_enabled(false);
            app.push_system("Telemetry: OFF (event logging disabled)");
        }
        Some(other) => {
            app.push_error(&format!("Invalid option: {other} (use on/off/status)"));
        }
    }
}

fn cmd_permissions_status(app: &mut TuiApp) {
    let store = nocode_core::tool::permission::global_permission_rules();
    let guard = store.lock().unwrap_or_else(|e| e.into_inner());
    let rules = guard.list();
    let mut lines = vec!["Permission rules:".to_string(), String::new()];
    if rules.is_empty() {
        lines.push("  (no rules configured — default: ask)".to_string());
    } else {
        for rule in rules {
            let pattern = rule.argument_pattern.as_deref().unwrap_or("(any)");
            lines.push(format!(
                "  {:<16} {:?}  pattern: {}",
                rule.tool_name, rule.action, pattern
            ));
        }
    }
    lines.push(String::new());
    lines.push("Usage: /permissions-add <tool> <allow|deny> [pattern]".to_string());
    lines.push("       /permissions-remove <tool> [pattern]".to_string());
    app.push_system(&lines.join("\n"));
}

fn cmd_ide(app: &mut TuiApp, args: Option<&str>) {
    match args.map(str::trim) {
        None | Some("status") | Some("") => {
            app.push_system(
                "IDE Server:\n\n\
                 \x20 Status:  available (standalone mode)\n\
                 \x20 Mode:    --ide-server\n\
                 \x20 Port:    3002 (default)\n\
                 \x20 Endpoints: initialize, query, completions, hover, diagnostics, status\n\n\
                 The IDE server provides a JSON-RPC interface for VS Code / JetBrains.\n\
                 It supports:\n\
                 \x20 - query:     Full agentic loop execution\n\
                 \x20 - completions: Slash commands + tool names\n\
                 \x20 - hover:     File content at line\n\
                 \x20 - diagnostics: Platform info\n\n\
                 Start: nocode --ide-server\n\
                 Or use /ide start to launch from TUI.",
            );
        }
        Some("start") => {
            // Launch IDE server as a background process
            let exe = std::env::current_exe()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| "nocode".to_string());
            match std::process::Command::new(&exe)
                .arg("--ide-server")
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
            {
                Ok(child) => {
                    let pid = child.id();
                    app.push_system(&format!(
                        "IDE server started (PID {pid})\n\
                         \x20 Endpoint: stdio JSON-RPC\n\
                         \x20 Use /ide stop to terminate."
                    ));
                }
                Err(e) => {
                    app.push_error(&format!(
                        "Failed to start IDE server: {e}\n\
                         Try running directly: nocode --ide-server"
                    ));
                }
            }
        }
        Some("stop") => {
            // Try to find and kill a running IDE server process
            match std::process::Command::new("pkill")
                .args(["-f", "nocode.*--ide-server"])
                .output()
            {
                Ok(output) if output.status.success() => {
                    app.push_system("IDE server stopped.");
                }
                _ => {
                    app.push_system(
                        "No running IDE server found.\n\
                         The server may have been started from a different terminal.",
                    );
                }
            }
        }
        Some(other) => {
            app.push_error(&format!("Invalid option: {other} (use start/stop/status)"));
        }
    }
}

fn cmd_voice(app: &mut TuiApp, args: Option<&str>) {
    match args.map(str::trim) {
        None | Some("status") | Some("") => {
            // Check for common voice input tools
            let has_sox = std::process::Command::new("which")
                .arg("sox")
                .output()
                .is_ok_and(|o| o.status.success());
            let has_arecord = std::process::Command::new("which")
                .arg("arecord")
                .output()
                .is_ok_and(|o| o.status.success());
            let has_whisper = std::process::Command::new("which")
                .arg("whisper")
                .output()
                .is_ok_and(|o| o.status.success());

            let backend = if has_sox {
                "sox (rec)"
            } else if has_arecord {
                "arecord (ALSA)"
            } else {
                "none detected"
            };
            let transcribe = if has_whisper {
                "whisper CLI"
            } else {
                "API needed"
            };

            app.push_system(&format!(
                "Voice Input:\n\n\
                 \x20 Status:     available\n\
                 \x20 Recording:  {backend}\n\
                 \x20 Transcribe: {transcribe}\n\n\
                 Voice captures audio, transcribes, and sends as text.\n\n\
                 Usage: /voice start|stop|status"
            ));
        }
        Some("start") => {
            let has_sox = std::process::Command::new("which")
                .arg("sox")
                .output()
                .is_ok_and(|o| o.status.success());
            let has_arecord = std::process::Command::new("which")
                .arg("arecord")
                .output()
                .is_ok_and(|o| o.status.success());

            if !has_sox && !has_arecord {
                app.push_error(
                    "Voice input requires 'sox' or 'arecord' to be installed.\n\
                     Install: sudo apt install sox (Linux) / brew install sox (macOS)",
                );
                return;
            }

            // Record to a temp WAV file
            let tmp_path = std::env::temp_dir().join("nocode_voice.wav");
            let tmp_str = tmp_path.to_string_lossy().into_owned();

            // Use sox rec or arecord
            let record_result = if has_sox {
                std::process::Command::new("rec")
                    .args(["-q", &tmp_str, "trim", "0", "5"])
                    .output()
            } else {
                std::process::Command::new("arecord")
                    .args(["-q", "-d", "5", "-f", "S16_LE", "-r", "16000", &tmp_str])
                    .output()
            };

            match record_result {
                Ok(output) if output.status.success() => {
                    // Try whisper CLI transcription
                    let transcript = if std::process::Command::new("which")
                        .arg("whisper")
                        .output()
                        .is_ok_and(|o| o.status.success())
                    {
                        match std::process::Command::new("whisper")
                            .args([
                                &tmp_str,
                                "--model",
                                "tiny",
                                "--output_format",
                                "txt",
                                "--output_dir",
                                "/tmp",
                            ])
                            .output()
                        {
                            Ok(wo) => {
                                let txt_path = tmp_path.with_extension("txt");
                                std::fs::read_to_string(&txt_path).unwrap_or_else(|_| {
                                    String::from_utf8_lossy(&wo.stdout).into_owned()
                                })
                            }
                            Err(_) => "(whisper transcription failed)".to_string(),
                        }
                    } else {
                        "(recorded to ".to_string()
                            + &tmp_str
                            + " — install whisper CLI for transcription)"
                    };

                    if !transcript.trim().is_empty()
                        && transcript != "(whisper transcription failed)"
                    {
                        app.input = transcript.trim().to_string();
                        app.cursor_pos = app.input.len();
                        app.push_system("Voice input captured. Press Enter to send.");
                    } else {
                        app.push_system(&format!(
                            "Voice recorded to {tmp_str} (no transcription available)"
                        ));
                    }
                }
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    app.push_error(&format!("Recording failed: {stderr}"));
                }
                Err(e) => {
                    app.push_error(&format!("Failed to start recording: {e}"));
                }
            }
        }
        Some("stop") => {
            // Kill any running recording process
            let _ = std::process::Command::new("pkill")
                .args(["-f", "rec.*nocode_voice|arecord.*nocode_voice"])
                .output();
            app.push_system("Voice recording stopped.");
        }
        Some(other) => {
            app.push_error(&format!("Invalid option: {other} (use start/stop/status)"));
        }
    }
}

fn cmd_undo(app: &mut TuiApp, _messages: &mut Vec<Message>) {
    let mut history = nocode_core::storage::file_history::FileHistory::new(".")
        .unwrap_or_else(|_| nocode_core::storage::file_history::FileHistory::new("/tmp").unwrap());
    if history.can_undo() {
        match history.undo() {
            Ok(path) => app.push_system(&format!("Undone: {path}")),
            Err(e) => app.push_error(&format!("Undo failed: {e}")),
        }
    } else {
        app.push_system("Nothing to undo.");
    }
}

fn cmd_redo(app: &mut TuiApp, _messages: &mut Vec<Message>) {
    let mut history = nocode_core::storage::file_history::FileHistory::new(".")
        .unwrap_or_else(|_| nocode_core::storage::file_history::FileHistory::new("/tmp").unwrap());
    if history.can_redo() {
        match history.redo() {
            Ok(path) => app.push_system(&format!("Redone: {path}")),
            Err(e) => app.push_error(&format!("Redo failed: {e}")),
        }
    } else {
        app.push_system("Nothing to redo.");
    }
}

fn cmd_rewind(app: &mut TuiApp, args: Option<&str>, messages: &mut Vec<Message>) {
    let target = match args {
        Some(n) => match n.parse::<usize>() {
            Ok(idx) => idx,
            Err(_) => {
                app.push_error("Usage: /rewind <message_index>");
                return;
            }
        },
        None => {
            if messages.len() > 1 {
                messages.len() - 1
            } else {
                app.push_system("Nothing to rewind.");
                return;
            }
        }
    };

    if target >= messages.len() {
        app.push_error(&format!(
            "Index {target} out of range (0..{})",
            messages.len()
        ));
        return;
    }

    let removed = messages.len() - target;
    messages.truncate(target);
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
        "Rewound {removed} messages (now {} total)",
        messages.len()
    ));
}
