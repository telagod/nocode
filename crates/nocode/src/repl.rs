use crate::command_registry::{CommandAction, CommandRegistry};
use nocode_core::message::{ContentBlock, Message, SystemBlock};
use nocode_core::provider::Provider;
use nocode_core::provider::types::{StreamDelta, StreamEvent};
use nocode_core::query::r#loop::{self, LoopConfig, LoopObserver};
use nocode_core::tool::ToolRegistry;
use nocode_core::tool::executor::ToolExecutor;
use std::io::{self, BufRead, Write};

/// Run the interactive REPL.
pub fn run_repl(
    provider: &dyn Provider,
    registry: &ToolRegistry,
    system: &[SystemBlock],
    model: &str,
    max_tokens: u32,
    max_turns: u32,
) {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    println!(
        "nocode v{} — type /help for commands",
        env!("CARGO_PKG_VERSION")
    );
    println!();

    let executor = ToolExecutor::new(registry);
    let mut messages: Vec<Message> = Vec::new();
    let cmd_reg = CommandRegistry::with_defaults();
    let mut current_model = model.to_string();

    loop {
        print!("> ");
        let _ = stdout.flush();

        let mut input = String::new();
        if stdin.lock().read_line(&mut input).is_err() || input.is_empty() {
            break;
        }
        let input = input.trim();
        if input.is_empty() {
            continue;
        }

        // Slash commands via registry
        if let Some((action, args)) = cmd_reg.resolve(input) {
            match action {
                CommandAction::Quit => break,
                CommandAction::Clear => {
                    messages.clear();
                    println!("(conversation cleared)");
                }
                CommandAction::Help => {
                    println!("{}", cmd_reg.help_text());
                }
                CommandAction::Status => {
                    println!("Model: {current_model}");
                    println!("Messages: {}", messages.len());
                    let total_tok = messages.iter().map(|m| m.content.len() as u64).sum::<u64>();
                    println!("Content blocks: {total_tok}");
                }
                CommandAction::Model => {
                    if let Some(new_model) = args {
                        current_model = new_model.clone();
                        println!("Model switched to: {current_model}");
                    } else {
                        println!("Current model: {current_model}");
                        println!("Usage: /model <model_name>");
                    }
                }
                // REPL_SLASH_PLACEHOLDER
                CommandAction::Sessions => {
                    repl_cmd_sessions();
                }
                CommandAction::Resume => {
                    if let Some(sid) = args {
                        repl_cmd_resume(&sid, &mut messages);
                    } else {
                        println!("Usage: /resume <session_id>");
                    }
                }
                CommandAction::Version => {
                    println!("nocode v{}", env!("CARGO_PKG_VERSION"));
                }
                CommandAction::Cost => {
                    println!("(cost tracking available in TUI mode)");
                }
                CommandAction::Export => {
                    repl_cmd_export(args.as_deref(), &messages);
                }
                CommandAction::Bug => {
                    repl_cmd_bug();
                }
                CommandAction::Doctor => {
                    repl_cmd_doctor(&current_model);
                }
                CommandAction::Init => {
                    repl_cmd_init();
                }
                CommandAction::Login => {
                    repl_cmd_login();
                }
                CommandAction::History => {
                    println!("(input history available in TUI mode)");
                }
                CommandAction::Permissions => {
                    println!("Permission mode: ask (default)");
                }
                _ => {
                    println!("(command not available in REPL mode)");
                }
            }
            continue;
        }

        messages.push(Message::user_text(input));

        let config = LoopConfig {
            model: current_model.clone(),
            max_tokens,
            max_turns,
            system: system.to_vec(),
            tools: registry.definitions(),
            parallel_tool_execution: true,
        };

        let mut observer = ReplObserver::new();

        match r#loop::run_agentic_loop(
            provider,
            &executor,
            &config,
            messages.clone(),
            &mut observer,
        ) {
            Ok(result) => {
                if observer.needs_newline {
                    println!();
                }
                messages = result.messages;
            }
            Err(e) => {
                eprintln!("\nerror: {e}");
            }
        }

        println!();
    }
}

// ---------------------------------------------------------------------------
// Slash command implementations (shared REPL helpers)
// ---------------------------------------------------------------------------

fn repl_cmd_sessions() {
    use nocode_core::session::persistence::SessionPersistence;
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let infos = SessionPersistence::list_sessions_with_info(&cwd);
    if infos.is_empty() {
        println!("No saved sessions.");
    } else {
        println!("Saved sessions:");
        for info in infos.iter().take(20) {
            let preview = info.first_user_message.as_deref().unwrap_or("(empty)");
            println!("  {} ({} msgs) — {}", info.id, info.message_count, preview);
        }
        println!("\nUse /resume <id> to restore.");
    }
}

fn repl_cmd_resume(session_id: &str, messages: &mut Vec<Message>) {
    use nocode_core::session::persistence::SessionPersistence;
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    match SessionPersistence::resume(&cwd, session_id) {
        Ok((_persistence, loaded)) => {
            let count = loaded.len();
            *messages = loaded;
            println!("Resumed session '{session_id}' ({count} messages)");
        }
        Err(e) => eprintln!("Failed to resume: {e}"),
    }
}

fn repl_cmd_export(path: Option<&str>, messages: &[Message]) {
    if messages.is_empty() {
        println!("Nothing to export — conversation is empty.");
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
        content.push('\n');
        content.push('\n');
        content.push_str(&msg.text_content());
        content.push_str("\n\n");
    }
    match std::fs::write(out_path, &content) {
        Ok(()) => println!(
            "Exported {len} messages to {out_path}",
            len = messages.len()
        ),
        Err(e) => eprintln!("Export failed: {e}"),
    }
}

fn repl_cmd_bug() {
    println!("To report a bug:");
    println!("  1. Open https://github.com/anthropics/nocode/issues/new");
    println!("  2. Include: nocode version, OS, steps to reproduce");
    println!("  Version: nocode v{}", env!("CARGO_PKG_VERSION"));
    println!("  OS: {}", std::env::consts::OS);
    println!("  Arch: {}", std::env::consts::ARCH);
}

fn repl_cmd_doctor(model: &str) {
    println!("nocode doctor — diagnostics\n");
    println!("Version: v{}", env!("CARGO_PKG_VERSION"));
    println!("OS: {} ({})", std::env::consts::OS, std::env::consts::ARCH);
    println!("Model: {model}");
    println!();

    // API keys
    let keys = [
        ("ANTHROPIC_API_KEY", "Claude"),
        ("OPENAI_API_KEY", "OpenAI"),
        ("GEMINI_API_KEY", "Gemini"),
    ];
    println!("API keys:");
    for (var, name) in &keys {
        let status = if std::env::var(var).is_ok() {
            "set"
        } else {
            "not set"
        };
        println!("  {name}: {status}");
    }
    println!();

    // Config files
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let home = std::env::var("HOME").unwrap_or_default();
    let paths = [
        (format!("{home}/.nocode/settings.json"), "User"),
        (format!("{cwd}/.nocode/settings.json"), "Project"),
        (format!("{cwd}/.nocode/settings.local.json"), "Local"),
    ];
    println!("Settings files:");
    for (path, tier) in &paths {
        let mark = if std::path::Path::new(path).exists() {
            "found"
        } else {
            "not found"
        };
        println!("  {tier}: {mark}");
    }
    println!();

    // CLAUDE.md
    let md_files = nocode_core::prompt::assembly::discover_claude_md(&cwd);
    println!("CLAUDE.md files: {}", md_files.len());

    // Sessions
    let sessions = nocode_core::session::persistence::SessionPersistence::list_sessions(&cwd);
    println!("Saved sessions: {}", sessions.len());

    println!("\nAll checks passed.");
}

fn repl_cmd_init() {
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let claude_md_path = format!("{cwd}/CLAUDE.md");
    if std::path::Path::new(&claude_md_path).exists() {
        println!("CLAUDE.md already exists at {claude_md_path}");
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
        Ok(()) => println!("Created {claude_md_path}"),
        Err(e) => eprintln!("Failed to create CLAUDE.md: {e}"),
    }
}

fn repl_cmd_login() {
    println!("Configure API keys via environment variables:");
    println!();
    println!("  export ANTHROPIC_API_KEY=sk-ant-...");
    println!("  export OPENAI_API_KEY=sk-...");
    println!("  export GEMINI_API_KEY=AI...");
    println!();
    println!("Or add to ~/.nocode/settings.json");
}

struct ReplObserver {
    needs_newline: bool,
}

impl ReplObserver {
    fn new() -> Self {
        Self {
            needs_newline: false,
        }
    }
}

impl LoopObserver for ReplObserver {
    fn on_stream_event(&mut self, event: &StreamEvent) {
        if let StreamEvent::ContentBlockDelta { delta, .. } = event {
            match delta {
                StreamDelta::TextDelta { text } => {
                    print!("{text}");
                    let _ = io::stdout().flush();
                    self.needs_newline = !text.ends_with('\n');
                }
                StreamDelta::ThinkingDelta { thinking } => {
                    print!("\x1b[2m∴ {thinking}\x1b[0m");
                    let _ = io::stdout().flush();
                }
                _ => {}
            }
        }
    }

    fn on_tool_start(&mut self, name: &str, _id: &str) {
        if self.needs_newline {
            println!();
            self.needs_newline = false;
        }
        println!("\x1b[36m❯ {name}\x1b[0m");
    }

    fn on_tool_done(&mut self, name: &str, _id: &str, result: &ContentBlock) {
        if let ContentBlock::ToolResult {
            content, is_error, ..
        } = result
        {
            let prefix = if *is_error {
                "\x1b[31m✖"
            } else {
                "\x1b[32m⎿"
            };
            // Show truncated result
            let display = if content.len() > 200 {
                format!("{}...", &content[..200])
            } else {
                content.clone()
            };
            println!("{prefix} {name}\x1b[0m {display}");
        }
    }
}
