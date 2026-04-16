use crate::command_registry::{CommandAction, CommandRegistry};
use nocode_core::message::{ContentBlock, Message, SystemBlock};
use nocode_core::provider::ProviderBox;
use nocode_core::provider::types::{StreamDelta, StreamEvent};
use nocode_core::query::r#loop::{self, LoopConfig, LoopObserver};
use nocode_core::tool::ToolRegistry;
use nocode_core::tool::executor::ToolExecutor;
use nocode_core::tool::global_registry::tool_definitions_for_model;
use nocode_core::tool::permission::{QuestionPrompter, UserAnswer};
use std::io::{self, BufRead, Write};

/// Run the interactive REPL.
pub fn run_repl(
    provider: ProviderBox,
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

    // Inject REPL question prompter into AskUserQuestion tool
    if let Some(ask_tool) = registry
        .get_as::<nocode_core::tool::interactive_tools::AskUserQuestionTool>("AskUserQuestion")
    {
        ask_tool.set_prompter(Box::new(ReplQuestionPrompter));
    }

    let cmd_reg = CommandRegistry::with_defaults();
    let mut current_model = model.to_string();

    // Session persistence
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let session_id = format!(
        "{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        std::process::id()
    );
    let mut persistence =
        nocode_core::session::persistence::SessionPersistence::new(&cwd, &session_id);

    // Install worker event channel for background agent notifications
    let worker_event_rx = {
        let (worker_tx, worker_rx) = std::sync::mpsc::channel();
        let reg = nocode_core::agent::worker::global_worker_registry();
        let mut guard = reg.lock().unwrap_or_else(|e| e.into_inner());
        guard.set_event_channel(worker_tx);
        worker_rx
    };

    loop {
        // Drain worker events before prompting
        {
            use nocode_core::agent::worker::WorkerEvent;
            loop {
                match worker_event_rx.try_recv() {
                    Ok(WorkerEvent::Finished {
                        worker_id,
                        name,
                        result,
                    }) => {
                        let preview = crate::tool_render::truncate_str(&result, 300);
                        println!(
                            "\x1b[36m● Agent '{name}' ({worker_id}) finished:\x1b[0m\n{preview}"
                        );
                    }
                    Ok(WorkerEvent::Failed {
                        worker_id,
                        name,
                        error,
                    }) => {
                        println!("\x1b[31m✖ Agent '{name}' ({worker_id}) failed: {error}\x1b[0m");
                    }
                    Ok(WorkerEvent::TimedOut { worker_id, name }) => {
                        println!("\x1b[31m✖ Agent '{name}' ({worker_id}) timed out\x1b[0m");
                    }
                    Ok(WorkerEvent::StateChanged { .. }) => {}
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                }
            }
            // Check timeouts
            let reg = nocode_core::agent::worker::global_worker_registry();
            let mut guard = reg.lock().unwrap_or_else(|e| e.into_inner());
            let _ = guard.check_timeouts();
        }

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
                    persistence.persist_full(&messages);
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
                        if let Some(new_persistence) = repl_cmd_resume(&sid, &mut messages) {
                            persistence = new_persistence;
                        }
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
                CommandAction::History => {
                    println!("(input history available in TUI mode)");
                }
                CommandAction::Permissions => {
                    println!("Permission mode: ask (default)");
                }
                CommandAction::Compact => {
                    use nocode_core::session::compaction::{
                        Compactor, RichCompactor, TailCompactor,
                    };
                    if messages.len() <= 5 {
                        println!("Nothing to compact (conversation too short)");
                    } else {
                        // Try RichCompactor (LLM-driven) first, fall back to TailCompactor
                        let rich =
                            RichCompactor::new(Box::new(provider.clone()), &current_model, 10);
                        let result = rich.compact(&messages);
                        if result.compacted_count > 0 {
                            messages = result.messages;
                            println!(
                                "Compacted {} messages, ~{} tokens saved (rich summary)",
                                result.compacted_count, result.tokens_saved
                            );
                        } else {
                            // Fallback to tail compactor
                            let tail = TailCompactor::new(10);
                            let result = tail.compact(&messages);
                            if result.compacted_count > 0 {
                                messages = result.messages;
                                println!(
                                    "Compacted {} messages, ~{} tokens saved (tail)",
                                    result.compacted_count, result.tokens_saved
                                );
                            } else {
                                println!("Nothing to compact (conversation too short)");
                            }
                        }
                    }
                }
                CommandAction::Mcp => {
                    use nocode_core::mcp::manager::global_mcp_manager;
                    let mgr = global_mcp_manager();
                    let mgr = mgr.lock().unwrap_or_else(|e| e.into_inner());
                    let servers = mgr.list_servers();
                    if servers.is_empty() {
                        println!("No MCP servers connected.");
                    } else {
                        for (name, phase, tool_count) in &servers {
                            println!("  {name}: {phase:?} ({tool_count} tools)");
                        }
                    }
                }
                CommandAction::Agents => {
                    use nocode_core::agent::worker::global_worker_registry;
                    let reg = global_worker_registry();
                    let reg = reg.lock().unwrap_or_else(|e| e.into_inner());
                    let workers = reg.list();
                    if workers.is_empty() {
                        println!("No background agents running.");
                    } else {
                        for w in &workers {
                            println!("  {} ({}): {:?}", w.name, w.id, w.state);
                        }
                    }
                }
                CommandAction::Config => {
                    let cwd = std::env::current_dir()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_else(|_| String::from("."));
                    let settings = nocode_core::config::settings::Settings::load_merged(&cwd);
                    let config =
                        nocode_core::config::runtime::RuntimeConfig::from_settings(&settings, &cwd);
                    let provider_str = settings.model_provider.as_deref().unwrap_or("auto");
                    println!("Provider: {provider_str}");
                    println!("Model: {}", config.model);
                    if let Some(ref url) = config.custom_base_url
                        && !url.is_empty()
                    {
                        println!("Custom URL: {url}");
                    }
                    if let Some(ref fmt) = config.custom_api_format
                        && !fmt.is_empty()
                    {
                        println!("API format: {fmt}");
                    }
                    println!("Permission mode: {}", config.permission_mode);
                    println!("Max turns: {}", config.max_turns);
                    println!("Max tokens: {}", config.max_tokens);
                    println!(
                        "Sandbox: {}",
                        if config.sandbox.enabled { "on" } else { "off" }
                    );
                    println!();
                    println!("Tip: use TUI mode (nocode --tui) for interactive /config editing.");
                    println!("     Or set API keys: export ANTHROPIC_API_KEY=sk-ant-...");
                }
                CommandAction::Memory => {
                    let query = args.as_deref().unwrap_or("");
                    let home = std::env::var("HOME").unwrap_or_default();
                    let mem_dir = format!("{home}/.nocode/memory");
                    let store = nocode_core::storage::memory::MemoryStore::new(&mem_dir);
                    if query.is_empty() {
                        match store.list() {
                            Ok(entries) if entries.is_empty() => println!("No memories stored."),
                            Ok(entries) => {
                                println!("Memories:");
                                for entry in &entries {
                                    println!("  {} — {}", entry.name, entry.description);
                                }
                            }
                            Err(e) => eprintln!("Error: {e}"),
                        }
                    } else {
                        match store.search(query) {
                            Ok(results) if results.is_empty() => {
                                println!("No memories matching '{query}'.")
                            }
                            Ok(results) => {
                                for entry in &results {
                                    println!("  {} — {}", entry.name, entry.description);
                                }
                            }
                            Err(e) => eprintln!("Error: {e}"),
                        }
                    }
                }
                CommandAction::Plan => {
                    use nocode_core::tool::session_tools::{enter_plan_mode, is_plan_mode};
                    if is_plan_mode() {
                        println!("Already in plan mode.");
                    } else {
                        enter_plan_mode();
                        let desc = args.as_deref().unwrap_or("(no description)");
                        println!("Plan mode activated: {desc}");
                        println!(
                            "Only read-only tools are available. Use /review or /compact when ready."
                        );
                    }
                }
                CommandAction::Review => {
                    // LLM-driven code review: get diff → ask model to review
                    let flag = args.as_deref().unwrap_or("");
                    let diff_output = if flag == "--staged" || flag == "staged" {
                        std::process::Command::new("git")
                            .args(["diff", "--staged"])
                            .output()
                    } else if flag.is_empty() {
                        std::process::Command::new("git").args(["diff"]).output()
                    } else {
                        std::process::Command::new("git")
                            .args(["diff", "--", flag])
                            .output()
                    };

                    let diff_text = match diff_output {
                        Ok(output) => {
                            let stdout = String::from_utf8_lossy(&output.stdout);
                            if stdout.is_empty() {
                                println!("No changes to review.");
                                continue;
                            }
                            stdout.into_owned()
                        }
                        Err(e) => {
                            eprintln!("Failed to run git diff: {e}");
                            continue;
                        }
                    };

                    // Truncate very large diffs
                    let diff_for_review = if diff_text.len() > 50000 {
                        let boundary = crate::tool_render::safe_char_boundary(&diff_text, 50000);
                        format!(
                            "{}\n\n... (truncated, {} chars total)",
                            &diff_text[..boundary],
                            diff_text.len()
                        )
                    } else {
                        diff_text
                    };

                    println!("Reviewing changes...\n");

                    // Build review request
                    use nocode_core::message::SystemBlock;
                    use nocode_core::provider::Provider;
                    use nocode_core::provider::types::CreateMessageRequest;

                    let review_request = CreateMessageRequest {
                        model: current_model.clone(),
                        max_tokens: 4096,
                        system: vec![SystemBlock {
                            block_type: "text".to_string(),
                            text: "You are an expert code reviewer. Review the following git diff carefully. \
                                   For each change, assess: correctness, potential bugs, security issues, \
                                   performance implications, and code style. \
                                   Be specific — reference file names and line ranges. \
                                   Prioritize actionable findings over style nits. \
                                   Format: start with a summary, then list findings by severity (critical/high/medium/low).".to_string(),
                            cache_control: None,
                        }],
                        messages: vec![Message::user_text(format!(
                            "Review these code changes:\n\n```diff\n{diff_for_review}\n```"
                        ))],
                        tools: vec![],
                        stream: false,
                        thinking: None,
                        response_format: None,
                    };

                    match provider.create_message(&review_request) {
                        Ok(response) => {
                            let text = response.text_content();
                            println!("{text}");
                        }
                        Err(e) => {
                            eprintln!("Review failed (LLM call error): {e}");
                            // Fallback: show diff stat
                            let stat_cmd = if flag == "--staged" || flag == "staged" {
                                std::process::Command::new("git")
                                    .args(["diff", "--staged", "--stat"])
                                    .output()
                            } else if flag.is_empty() {
                                std::process::Command::new("git")
                                    .args(["diff", "--stat"])
                                    .output()
                            } else {
                                std::process::Command::new("git")
                                    .args(["diff", "--stat", "--", flag])
                                    .output()
                            };
                            if let Ok(output) = stat_cmd {
                                println!(
                                    "\nFallback — diff stat:\n{}",
                                    String::from_utf8_lossy(&output.stdout)
                                );
                            }
                        }
                    }
                }
                CommandAction::Skills => {
                    let reg = crate::command_registry::CommandRegistry::with_defaults();
                    println!("{}", reg.help_text());
                    let skills = nocode_core::tool::skill::list_skills();
                    if !skills.is_empty() {
                        println!("\nUser skills:");
                        for (name, path) in &skills {
                            println!("  {name:<20} {}", path.display());
                        }
                    }
                }
                CommandAction::Env => {
                    let vars = [
                        ("NOCODE_MODEL_PROVIDER", "Provider override"),
                        ("NOCODE_MODEL", "Model override"),
                        ("NOCODE_CUSTOM_BASE_URL", "Custom endpoint"),
                        ("NOCODE_CUSTOM_API_FORMAT", "Custom API format"),
                        ("NOCODE_SYSTEM_PROMPT", "System prompt override"),
                        ("ANTHROPIC_API_KEY", "Claude API key"),
                        ("OPENAI_API_KEY", "OpenAI API key"),
                        ("GEMINI_API_KEY", "Gemini API key"),
                    ];
                    println!("Environment variables:");
                    for (var, desc) in &vars {
                        let val = std::env::var(var).ok();
                        let display = match val {
                            Some(_v) if var.contains("KEY") => "(set)".to_string(),
                            Some(v) => v,
                            None => "(not set)".to_string(),
                        };
                        println!("  {var:<30} {display:<20} {desc}");
                    }
                }
                CommandAction::Keybindings => {
                    println!(
                        "Keyboard shortcuts:\n\
                     \n\
                     Enter        — send message\n\
                     Ctrl-C       — quit\n\
                     Up/Down      — scroll\n\
                     Ctrl-P/N     — input history\n\
                     Ctrl-U       — clear input\n\
                     Ctrl-L       — clear screen\n\
                     Tab          — toggle tool output"
                    );
                }
                CommandAction::Theme => {
                    println!("(theme switching available in TUI mode — use --tui)");
                }
                CommandAction::Vim => {
                    println!("(vim mode available in TUI mode — use --tui)");
                }
                CommandAction::BugHunter | CommandAction::SecurityReview => {
                    let is_security = matches!(action, CommandAction::SecurityReview);
                    let scan_path = args.as_deref().unwrap_or(".");

                    // Discover source files to scan
                    let extensions = if is_security {
                        &["rs", "py", "js", "ts", "go", "rb", "java", "c", "cpp"][..]
                    } else {
                        &["rs", "py", "js", "ts", "go", "rb", "java", "c", "cpp", "sh"][..]
                    };

                    let mut files_to_scan = Vec::new();
                    if let Ok(entries) = std::fs::read_dir(scan_path) {
                        for entry in entries.flatten() {
                            let path = entry.path();
                            if path.is_file()
                                && let Some(ext) = path.extension().and_then(|e| e.to_str())
                                && extensions.contains(&ext)
                            {
                                files_to_scan.push(path);
                            }
                        }
                    }

                    if files_to_scan.is_empty() {
                        println!("No source files found to scan in '{scan_path}'.");
                        continue;
                    }

                    // Read and concatenate file contents (up to 80KB total)
                    let mut combined = String::new();
                    let mut scanned_count = 0usize;
                    for path in &files_to_scan {
                        if combined.len() > 80000 {
                            break;
                        }
                        if let Ok(content) = std::fs::read_to_string(path) {
                            combined.push_str(&format!(
                                "\n--- {} ---\n{}\n",
                                path.display(),
                                content
                            ));
                            scanned_count += 1;
                        }
                    }

                    let scan_type = if is_security { "security" } else { "bug" };
                    println!("Scanning {scanned_count} files for {scan_type} issues...\n");

                    // Build scan request
                    use nocode_core::message::SystemBlock;
                    use nocode_core::provider::Provider;
                    use nocode_core::provider::types::CreateMessageRequest;

                    let system_prompt = if is_security {
                        "You are an expert security researcher performing a security code review. \
                         Analyze the provided source code for security vulnerabilities including: \
                         injection flaws (SQLi, XSS, command injection), authentication/authorization bypass, \
                         cryptographic weaknesses, insecure data handling, SSRF, path traversal, \
                         hardcoded secrets, and OWASP Top 10 issues. \
                         For each finding, specify: severity (critical/high/medium/low), \
                         file path, line range, vulnerability type, and remediation advice. \
                         Prioritize real vulnerabilities over false positives.".to_string()
                    } else {
                        "You are an expert code analyst scanning for common bugs and issues. \
                         Analyze the provided source code for: null pointer dereferences, \
                         off-by-one errors, resource leaks, race conditions, error handling gaps, \
                         logic errors, unreachable code, and common anti-patterns. \
                         For each finding, specify: severity (critical/high/medium/low), \
                         file path, line range, bug type, and fix suggestion. \
                         Focus on real bugs, not style issues."
                            .to_string()
                    };

                    let scan_request = CreateMessageRequest {
                        model: current_model.clone(),
                        max_tokens: 4096,
                        system: vec![SystemBlock {
                            block_type: "text".to_string(),
                            text: system_prompt,
                            cache_control: None,
                        }],
                        messages: vec![Message::user_text(format!(
                            "Scan the following code for {scan_type} issues:\n\n{combined}"
                        ))],
                        tools: vec![],
                        stream: false,
                        thinking: None,
                        response_format: None,
                    };

                    match provider.create_message(&scan_request) {
                        Ok(response) => {
                            let text = response.text_content();
                            println!("{text}");
                        }
                        Err(e) => {
                            eprintln!("Scan failed (LLM call error): {e}");
                        }
                    }
                }
                CommandAction::Copy => {
                    // Find last assistant message and copy to clipboard
                    let last_assistant = messages
                        .iter()
                        .rev()
                        .find(|m| matches!(m.role, nocode_core::message::Role::Assistant));
                    if let Some(msg) = last_assistant {
                        let text = msg.text_content();
                        if let Err(e) = copy_to_clipboard(&text) {
                            eprintln!("Failed to copy to clipboard: {e}");
                        } else {
                            let preview = if text.len() > 60 {
                                crate::tool_render::truncate_str(&text, 60)
                            } else {
                                text.clone()
                            };
                            println!("Copied to clipboard: {preview}");
                        }
                    } else {
                        println!("No assistant response to copy.");
                    }
                }
                CommandAction::Undo => {
                    // Undo last file modification by checking file history
                    let cwd = std::env::current_dir()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    match nocode_core::storage::file_history::FileHistory::new(&cwd) {
                        Ok(mut history) => match history.undo() {
                            Ok(path) => println!("Undone: {path}"),
                            Err(e) => println!("Nothing to undo: {e}"),
                        },
                        Err(_) => println!("File history not available."),
                    }
                }
                CommandAction::Redo => {
                    let cwd = std::env::current_dir()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    match nocode_core::storage::file_history::FileHistory::new(&cwd) {
                        Ok(mut history) => match history.redo() {
                            Ok(path) => println!("Redone: {path}"),
                            Err(e) => println!("Nothing to redo: {e}"),
                        },
                        Err(_) => println!("File history not available."),
                    }
                }
                CommandAction::Rewind => {
                    // Rewind conversation to a specific message index
                    let target: usize = args
                        .as_deref()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or_else(|| {
                            if messages.is_empty() {
                                0
                            } else {
                                messages.len().saturating_sub(1)
                            }
                        });
                    if target >= messages.len() {
                        println!(
                            "Invalid index. Conversation has {} messages.",
                            messages.len()
                        );
                    } else {
                        let removed = messages.len() - target;
                        messages.truncate(target);
                        persistence.persist_full(&messages);
                        println!("Rewound to message {target} (removed {removed} messages)");
                    }
                }
                _ => {
                    println!("(command not available in REPL mode)");
                }
            }
            continue;
        }

        // Save to persistent input history
        save_repl_history_entry(input);

        messages.push(Message::user_text(input));

        let config = LoopConfig {
            model: current_model.clone(),
            max_tokens,
            max_turns,
            system: system.to_vec(),
            tools: tool_definitions_for_model(registry),
            parallel_tool_execution: true,
        };

        let mut observer = ReplObserver::new();

        match r#loop::run_agentic_loop(
            provider.as_ref(),
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
                persistence.flush_incremental(&messages);
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

fn repl_cmd_resume(
    session_id: &str,
    messages: &mut Vec<Message>,
) -> Option<nocode_core::session::persistence::SessionPersistence> {
    use nocode_core::session::persistence::SessionPersistence;
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    match SessionPersistence::resume(&cwd, session_id) {
        Ok((new_persistence, loaded)) => {
            let count = loaded.len();
            *messages = loaded;
            println!("Resumed session '{session_id}' ({count} messages)");
            Some(new_persistence)
        }
        Err(e) => {
            eprintln!("Failed to resume: {e}");
            None
        }
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
                crate::tool_render::truncate_str(content, 200)
            } else {
                content.clone()
            };
            println!("{prefix} {name}\x1b[0m {display}");
        }
    }
}

// ---------------------------------------------------------------------------
// REPL Question Prompter — stdin-based interactive questions
// ---------------------------------------------------------------------------

/// Question prompter for REPL mode — reads answers from stdin.
struct ReplQuestionPrompter;

impl QuestionPrompter for ReplQuestionPrompter {
    fn prompt_questions(&self, questions: &serde_json::Value) -> Result<UserAnswer, String> {
        let Some(arr) = questions.as_array() else {
            return Err("Invalid questions format".to_string());
        };

        let mut selections = Vec::new();
        for (i, q) in arr.iter().enumerate() {
            let question_text = q["question"].as_str().unwrap_or("(question)");
            let header = q["header"].as_str().unwrap_or("Question");
            let empty_opts = Vec::new();
            let options = q["options"].as_array().unwrap_or(&empty_opts);

            println!("\n\x1b[1m[{header}]\x1b[0m {question_text}");
            for (j, opt) in options.iter().enumerate() {
                let label = opt["label"].as_str().unwrap_or("?");
                let desc = opt["description"].as_str().unwrap_or("");
                println!("  \x1b[36m{}.\x1b[0m {label} — {desc}", j + 1);
            }

            loop {
                print!("Select (1-{}): ", options.len());
                let _ = io::stdout().flush();

                let mut answer = String::new();
                if io::stdin().lock().read_line(&mut answer).is_err() {
                    return Err("stdin read error".to_string());
                }
                let answer = answer.trim();

                if answer.eq_ignore_ascii_case("q") || answer.eq_ignore_ascii_case("cancel") {
                    return Err("Cancelled by user".to_string());
                }

                if let Ok(idx) = answer.parse::<usize>()
                    && idx >= 1
                    && idx <= options.len()
                {
                    let label = options[idx - 1]["label"]
                        .as_str()
                        .unwrap_or("N/A")
                        .to_string();
                    println!("  → Selected: {label}");
                    selections.push(label);
                    break;
                }

                if i == 0 && selections.is_empty() {
                    eprintln!(
                        "Invalid choice. Enter a number 1-{} or 'q' to cancel.",
                        options.len()
                    );
                }
            }
        }

        Ok(UserAnswer { selections })
    }
}

// ---------------------------------------------------------------------------
// Clipboard helper — cross-platform copy
// ---------------------------------------------------------------------------

fn copy_to_clipboard(text: &str) -> Result<(), String> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let candidates: &[&[&str]] = if cfg!(target_os = "macos") {
        &[&["pbcopy"]]
    } else {
        &[
            &["xclip", "-selection", "clipboard"],
            &["xsel", "--clipboard", "--input"],
            &["wl-copy"],
            &["clip.exe"],
        ]
    };

    for cmd_args in candidates {
        if let Some(program) = cmd_args.first()
            && which_exists(program)
        {
            let mut child = Command::new(program)
                .args(&cmd_args[1..])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| format!("Failed to spawn {program}: {e}"))?;

            if let Some(mut stdin) = child.stdin.take() {
                stdin
                    .write_all(text.as_bytes())
                    .map_err(|e| format!("Write to {program}: {e}"))?;
            }
            let status = child
                .wait()
                .map_err(|e| format!("Wait for {program}: {e}"))?;
            if status.success() {
                return Ok(());
            }
        }
    }

    Err("No clipboard tool found (tried xclip, xsel, wl-copy, pbcopy, clip.exe)".to_string())
}

fn which_exists(program: &str) -> bool {
    std::process::Command::new("which")
        .arg(program)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Append a single entry to the persistent input history file (shared with TUI).
fn save_repl_history_entry(entry: &str) {
    let Some(home) = std::env::var("HOME").ok() else {
        return;
    };
    let path = std::path::PathBuf::from(home)
        .join(".nocode")
        .join("input_history.txt");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let escaped = entry.replace('\n', "\\n");
    let _ = writeln!(file, "{escaped}");
}
