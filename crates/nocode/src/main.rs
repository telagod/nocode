#[allow(dead_code)]
mod command_registry;
#[allow(dead_code)]
mod markdown_render;
#[allow(dead_code, clippy::collapsible_if)]
mod markdown_stream;
mod repl;
#[allow(dead_code)]
mod spinner;
#[allow(dead_code)]
mod status_hud;
#[allow(dead_code)]
mod tool_render;
#[allow(dead_code, clippy::empty_line_after_doc_comments)]
mod tool_truncate;
mod tui;
mod tui_app;
#[allow(dead_code)]
mod tui_input;
#[allow(dead_code)]
mod tui_permission;
#[allow(dead_code)]
mod tui_theme;
#[allow(dead_code)]
mod tui_widgets;

use nocode_core::config::settings::Settings;
use nocode_core::message::Message;
use nocode_core::prompt::assembly::{self, TruncationBudget};
use nocode_core::provider::Provider;
use nocode_core::provider::claude::ClaudeProvider;
use nocode_core::provider::gemini::GeminiProvider;
use nocode_core::provider::openai::OpenAiProvider;
use nocode_core::provider::types::ModelProvider;
use nocode_core::query::r#loop::{self, LoopConfig, NoopObserver};
use nocode_core::tool::ToolRegistry;
use nocode_core::tool::executor::ToolExecutor;
use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.iter().any(|a| a == "--version" || a == "-v") {
        println!("nocode {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return;
    }

    let cwd = env::current_dir()
        .expect("current directory should be accessible")
        .to_string_lossy()
        .into_owned();

    let settings = Settings::load_merged(&cwd);
    let provider_type = resolve_provider(&settings);
    let model = resolve_model(&settings);
    let max_turns = settings.max_turns.unwrap_or(10);
    let max_tokens = settings.max_tokens.unwrap_or(16384);

    let system_blocks = assembly::assemble_system_prompt(&cwd, &[], &TruncationBudget::default());

    let registry = ToolRegistry::with_defaults(&cwd);
    let provider_box = build_provider(&provider_type, &settings);

    // -- Run mode dispatch --

    if args.iter().any(|a| a == "--status") {
        run_status(&cwd, &provider_type, &model, &settings);
        return;
    }

    if let Some(prompt) = extract_arg(&args, "--bridge-once") {
        run_bridge_once(
            provider_box.as_ref(),
            &registry,
            &system_blocks,
            &model,
            max_tokens,
            max_turns,
            &prompt,
        );
        return;
    }

    if let Some(prompt) = extract_arg(&args, "--bridge-remote-once") {
        run_bridge_remote_once(&prompt);
        return;
    }

    if args.iter().any(|a| a == "--ide-server") {
        run_ide_server();
        return;
    }

    if args.iter().any(|a| a == "--mcp-server") {
        run_mcp_server();
        return;
    }

    if args.iter().any(|a| a == "--process-agent-daemon") {
        run_agent_daemon();
        return;
    }

    if args.iter().any(|a| a == "--process-agent-host") {
        run_agent_host();
        return;
    }

    if args.iter().any(|a| a == "--tui") {
        if let Err(e) = tui::run_tui(
            provider_box,
            registry,
            system_blocks,
            model,
            max_tokens,
            max_turns,
        ) {
            eprintln!("TUI error: {e}");
        }
        return;
    }

    // Default: REPL mode (also --repl)
    repl::run_repl(
        provider_box.as_ref(),
        &registry,
        &system_blocks,
        &model,
        max_tokens,
        max_turns,
    );
}

// --- PLACEHOLDER_REST ---

fn resolve_provider(_settings: &Settings) -> ModelProvider {
    if let Ok(p) = env::var("NOCODE_MODEL_PROVIDER")
        && let Some(provider) = ModelProvider::parse(&p)
    {
        return provider;
    }
    if env::var("ANTHROPIC_API_KEY").is_ok() {
        return ModelProvider::Claude;
    }
    if env::var("OPENAI_API_KEY").is_ok() {
        return ModelProvider::OpenAi;
    }
    if env::var("GEMINI_API_KEY").is_ok() {
        return ModelProvider::Gemini;
    }
    ModelProvider::Claude
}

fn resolve_model(settings: &Settings) -> String {
    if let Ok(m) = env::var("NOCODE_MODEL") {
        return m;
    }
    settings
        .model
        .clone()
        .unwrap_or_else(|| String::from("claude-sonnet-4-20250514"))
}

fn build_provider(provider: &ModelProvider, settings: &Settings) -> Box<dyn Provider> {
    match provider {
        ModelProvider::Claude => {
            let key = env::var("ANTHROPIC_API_KEY").unwrap_or_default();
            let base = env::var("ANTHROPIC_BASE_URL")
                .unwrap_or_else(|_| String::from("https://api.anthropic.com"));
            Box::new(ClaudeProvider::with_base_url(base, key))
        }
        ModelProvider::OpenAi => {
            let key = env::var("OPENAI_API_KEY").unwrap_or_default();
            let base = env::var("OPENAI_BASE_URL")
                .unwrap_or_else(|_| String::from("https://api.openai.com"));
            Box::new(OpenAiProvider::with_base_url(base, key))
        }
        ModelProvider::Gemini => {
            let key = env::var("GEMINI_API_KEY").unwrap_or_default();
            Box::new(GeminiProvider::new(key))
        }
        ModelProvider::Custom => {
            let key = env::var("ANTHROPIC_API_KEY")
                .or_else(|_| env::var("OPENAI_API_KEY"))
                .unwrap_or_default();
            let base = settings
                .custom_base_url
                .clone()
                .or_else(|| env::var("NOCODE_CUSTOM_BASE_URL").ok())
                .unwrap_or_else(|| String::from("http://localhost:8080"));
            let format = settings
                .custom_api_format
                .clone()
                .or_else(|| env::var("NOCODE_CUSTOM_API_FORMAT").ok())
                .unwrap_or_else(|| String::from("openai"));
            match format.as_str() {
                "anthropic" | "claude" => Box::new(ClaudeProvider::with_base_url(base, key)),
                _ => Box::new(OpenAiProvider::with_base_url(base, key)),
            }
        }
    }
}

// --- PLACEHOLDER_MODES ---

fn run_status(cwd: &str, provider: &ModelProvider, model: &str, settings: &Settings) {
    println!("nocode v{}", env!("CARGO_PKG_VERSION"));
    println!();
    println!("Working directory: {cwd}");
    println!("Provider: {}", provider.as_str());
    println!("Model: {model}");
    println!("Max turns: {}", settings.max_turns.unwrap_or(10));
    println!("Max tokens: {}", settings.max_tokens.unwrap_or(16384));
    println!();
    let keys = [
        ("ANTHROPIC_API_KEY", "Claude"),
        ("OPENAI_API_KEY", "OpenAI"),
        ("GEMINI_API_KEY", "Gemini"),
    ];
    println!("API keys:");
    for (var, name) in &keys {
        let status = if env::var(var).is_ok() {
            "set"
        } else {
            "not set"
        };
        println!("  {name}: {status}");
    }
    println!();
    let home = env::var("HOME").unwrap_or_default();
    let paths = [
        (format!("{home}/.nocode/settings.json"), "User"),
        (format!("{cwd}/.nocode/settings.json"), "Project"),
        (format!("{cwd}/.nocode/settings.local.json"), "Local"),
    ];
    println!("Settings files:");
    for (path, tier) in &paths {
        let exists = std::path::Path::new(path).exists();
        let mark = if exists { "found" } else { "not found" };
        println!("  {tier}: {mark}");
    }
    println!();
    let md_files = assembly::discover_claude_md(cwd);
    println!("CLAUDE.md files: {}", md_files.len());
    let sessions = nocode_core::session::persistence::SessionPersistence::list_sessions(cwd);
    println!("Saved sessions: {}", sessions.len());
}

fn run_bridge_once(
    provider: &dyn Provider,
    registry: &ToolRegistry,
    system_blocks: &[nocode_core::message::SystemBlock],
    model: &str,
    max_tokens: u32,
    max_turns: u32,
    prompt: &str,
) {
    if prompt.is_empty() {
        eprintln!("Usage: nocode --bridge-once \"<prompt>\"");
        return;
    }
    let messages = vec![Message::user_text(prompt)];
    let executor = ToolExecutor::new(registry);
    let config = LoopConfig {
        model: model.to_string(),
        max_tokens,
        max_turns,
        system: system_blocks.to_vec(),
        tools: registry.definitions(),
        parallel_tool_execution: true,
    };
    let mut observer = NoopObserver;
    match r#loop::run_agentic_loop(provider, &executor, &config, messages, &mut observer) {
        Ok(result) => {
            for msg in &result.messages {
                if msg.role == nocode_core::message::Role::Assistant {
                    let text = msg.text_content();
                    if !text.is_empty() {
                        println!("{text}");
                    }
                }
            }
        }
        Err(e) => eprintln!("Error: {e}"),
    }
}

fn run_bridge_remote_once(prompt: &str) {
    let base_url = env::var("NOCODE_BRIDGE_BASE_URL")
        .unwrap_or_else(|_| String::from("http://localhost:3000"));
    let auth_token = env::var("NOCODE_BRIDGE_AUTH_TOKEN").ok();
    eprintln!("Bridge remote: {base_url}");
    eprintln!("Prompt: {prompt}");
    if auth_token.is_some() {
        eprintln!("Auth: token set");
    }
    eprintln!("(bridge-remote-once: not yet wired — use --bridge-once for local)");
}

fn run_ide_server() {
    eprintln!("IDE server mode — not yet implemented. Use --repl or --tui.");
}

fn run_mcp_server() {
    eprintln!("MCP server mode — not yet implemented.");
}

fn run_agent_daemon() {
    eprintln!("Agent daemon mode — not yet implemented.");
}

fn run_agent_host() {
    eprintln!("Agent host mode — not yet implemented.");
}

fn extract_arg(args: &[String], flag: &str) -> Option<String> {
    for (i, a) in args.iter().enumerate() {
        if a == flag {
            return args.get(i + 1).cloned();
        }
        if let Some(val) = a.strip_prefix(&format!("{flag}=")) {
            return Some(val.to_string());
        }
    }
    None
}

fn print_help() {
    println!(
        "nocode — terminal-native AI coding assistant\n\
         \n\
         Usage: nocode [OPTIONS]\n\
         \n\
         Modes:\n\
         \x20 --repl                    Interactive REPL (default)\n\
         \x20 --tui                     Terminal UI mode\n\
         \x20 --status                  System diagnostics\n\
         \x20 --bridge-once \"prompt\"    Single-turn local execution\n\
         \x20 --bridge-remote-once \"p\"  Single-turn HTTP bridge\n\
         \x20 --ide-server              IDE server mode\n\
         \x20 --mcp-server              MCP server mode\n\
         \x20 --process-agent-daemon    Background agent daemon\n\
         \x20 --process-agent-host      Agent host process\n\
         \n\
         Options:\n\
         \x20 --version, -v             Show version\n\
         \x20 --help, -h                Show this help\n\
         \n\
         Environment:\n\
         \x20 ANTHROPIC_API_KEY         Claude API key\n\
         \x20 OPENAI_API_KEY            OpenAI API key\n\
         \x20 GEMINI_API_KEY            Gemini API key\n\
         \x20 NOCODE_MODEL              Override model name\n\
         \x20 NOCODE_MODEL_PROVIDER     Force provider (claude/openai/gemini/custom)\n\
         \x20 NOCODE_CUSTOM_BASE_URL    Custom provider base URL\n\
         \x20 NOCODE_CUSTOM_API_FORMAT  Custom provider API format\n\
         \x20 NOCODE_SYSTEM_PROMPT      Override system prompt\n\
         \x20 NOCODE_MODEL_REASONING_EFFORT  Reasoning effort (low/medium/high)\n\
         \x20 ANTHROPIC_BASE_URL        Override Anthropic API base URL\n\
         \x20 OPENAI_BASE_URL           Override OpenAI API base URL\n\
         \x20 NOCODE_BRIDGE_BASE_URL    Remote bridge URL\n\
         \x20 NOCODE_BRIDGE_AUTH_TOKEN  Remote bridge auth token"
    );
}
