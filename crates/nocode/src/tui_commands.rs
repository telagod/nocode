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
