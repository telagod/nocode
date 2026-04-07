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

use nocode_core::config::claude_md;
use nocode_core::config::settings::Settings;
use nocode_core::prompt::system::assemble_system_prompt;
use nocode_core::provider::Provider;
use nocode_core::provider::claude::ClaudeProvider;
use nocode_core::provider::types::ModelProvider;
use nocode_core::tool::ToolRegistry;
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
    let provider = resolve_provider(&settings);
    let model = resolve_model(&settings);
    let max_turns = settings.max_turns.unwrap_or(10);
    let max_tokens = settings.max_tokens.unwrap_or(16384);

    let claude_md_contents = claude_md::discover_claude_md(&cwd);
    let claude_md_prompt = claude_md::format_claude_md_prompt(&claude_md_contents);
    let system = assemble_system_prompt(&cwd, claude_md_prompt.as_deref(), None);

    let registry = ToolRegistry::with_defaults(&cwd);

    let provider_box = build_provider(&provider, &settings);

    if args.iter().any(|a| a == "--tui") {
        if let Err(e) = tui::run_tui(provider_box, registry, system, model, max_tokens, max_turns) {
            eprintln!("TUI error: {e}");
        }
        return;
    }

    if args.iter().any(|a| a == "--repl") {
        repl::run_repl(
            provider_box.as_ref(),
            &registry,
            &system,
            &model,
            max_tokens,
            max_turns,
        );
        return;
    }

    // Default: REPL mode
    repl::run_repl(
        provider_box.as_ref(),
        &registry,
        &system,
        &model,
        max_tokens,
        max_turns,
    );
}

fn resolve_provider(_settings: &Settings) -> ModelProvider {
    if let Ok(p) = env::var("NOCODE_MODEL_PROVIDER")
        && let Some(provider) = ModelProvider::parse(&p)
    {
        return provider;
    }
    // Auto-detect from available API keys
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

fn build_provider(provider: &ModelProvider, _settings: &Settings) -> Box<dyn Provider> {
    match provider {
        ModelProvider::Claude => {
            let key = env::var("ANTHROPIC_API_KEY").unwrap_or_default();
            let base = env::var("ANTHROPIC_BASE_URL")
                .unwrap_or_else(|_| String::from("https://api.anthropic.com"));
            Box::new(ClaudeProvider::with_base_url(base, key))
        }
        _ => {
            let key = env::var("ANTHROPIC_API_KEY").unwrap_or_default();
            Box::new(ClaudeProvider::new(key))
        }
    }
}

fn print_help() {
    println!(
        "nocode — terminal-native AI coding assistant\n\
         \n\
         Usage: nocode [OPTIONS]\n\
         \n\
         Options:\n\
         \x20 --repl       Interactive REPL mode (default)\n\
         \x20 --tui        Terminal UI mode\n\
         \x20 --version    Show version\n\
         \x20 --help       Show this help\n\
         \n\
         Environment:\n\
         \x20 ANTHROPIC_API_KEY    Claude API key\n\
         \x20 OPENAI_API_KEY       OpenAI API key\n\
         \x20 GEMINI_API_KEY       Gemini API key\n\
         \x20 NOCODE_MODEL         Override model name\n\
         \x20 NOCODE_MODEL_PROVIDER  Force provider (claude/openai/gemini)"
    );
}
