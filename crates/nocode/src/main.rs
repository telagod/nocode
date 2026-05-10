#[allow(dead_code)]
mod command_registry;
#[allow(dead_code)]
mod config_flow;
#[allow(dead_code)]
mod login;
#[allow(dead_code)]
mod markdown_render;
#[allow(dead_code, clippy::collapsible_if)]
mod markdown_stream;
#[allow(dead_code)]
mod model_fetch;
#[allow(dead_code)]
mod protocol_detect;
mod provider_presets;
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
mod tui_commands;
#[allow(dead_code)]
mod tui_events;
#[allow(dead_code)]
mod tui_overlays;
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
use nocode_core::provider::ProviderBox;
use nocode_core::provider::claude::ClaudeProvider;
use nocode_core::provider::foundry::FoundryProvider;
use nocode_core::provider::gemini::GeminiProvider;
use nocode_core::provider::openai::OpenAiProvider;
use nocode_core::provider::openai_responses::OpenAiResponsesProvider;
use nocode_core::provider::types::ModelProvider;
use nocode_core::query::r#loop::{self, LoopConfig, NoopObserver};
use nocode_core::tool::ToolRegistry;
use nocode_core::tool::executor::ToolExecutor;
use nocode_core::tool::global_registry::{
    initialize_runtime_global_registry, tool_definitions_for_model,
};
use std::env;
use std::io::IsTerminal;
use std::sync::Arc;

fn resolve_custom_system_prompt(settings: &Settings) -> Option<String> {
    env::var("NOCODE_SYSTEM_PROMPT")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| settings.system_prompt.clone())
}

/// Run self-update: prefer NOCODE_SOURCE_DIR (dev override),
/// otherwise clone/pull `https://github.com/telagod/nocode.git` to `~/.nocode/update-workspace`.
fn run_self_update() -> Result<String, String> {
    use nocode_core::update_checker::UpdateChecker;

    if let Ok(local) = env::var("NOCODE_SOURCE_DIR")
        && !local.is_empty()
        && std::path::Path::new(&local).join("Cargo.toml").exists()
    {
        eprintln!("Using local source: {local}");
        return UpdateChecker::self_update_local(&local);
    }

    let home = env::var("HOME").map_err(|_| "HOME env var not set".to_string())?;
    let workspace = format!("{home}/.nocode/update-workspace");
    let repo_url = "https://github.com/telagod/nocode.git";
    UpdateChecker::self_update_remote(repo_url, &workspace)
}

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

    // --update: pull from remote, build, replace current binary
    if args.iter().any(|a| a == "--update") {
        match run_self_update() {
            Ok(version) => {
                eprintln!();
                eprintln!("Updated to {version} — restart nocode to use the new version.");
            }
            Err(e) => {
                eprintln!("Update failed: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    if let Some(flag) = args
        .iter()
        .find(|a| a.as_str() == "--repl" || a.as_str() == "--tui")
    {
        eprintln!("Error: Unknown option: {flag}");
        eprintln!("Run `nocode --help` for usage.");
        std::process::exit(2);
    }

    let cwd = env::current_dir()
        .expect("current directory should be accessible")
        .to_string_lossy()
        .into_owned();

    let settings = Settings::load_merged(&cwd);

    // Start async model capabilities cache fetch
    nocode_core::provider::model_caps::init_cache_async();

    // Spawn background update check
    let _update_rx = {
        let home = std::env::var("HOME").unwrap_or_default();
        let cache_path = format!("{home}/.nocode/update_cache.json");
        let current_version = env!("CARGO_PKG_VERSION").to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let checker = nocode_core::update_checker::UpdateChecker::new(
                &current_version,
                &cache_path,
                "telagod/nocode",
            );
            let _ = tx.send(checker.check());
        });
        rx
    };

    // Load stored credentials into env (overrides shell env with login-configured keys)
    let cred_path = nocode_core::storage::credentials::CredentialStore::default_path();
    if let Ok(creds) = nocode_core::storage::credentials::CredentialStore::load(&cred_path) {
        creds.load_into_env();
    }

    // --login: run interactive login flow (select provider → key → model → save)
    if args.iter().any(|a| a == "--login") {
        login::run_login(&cwd);
        return;
    }

    // --resume / -c: resume a previous session
    let resume_session_id: Option<String> = if let Some(sid) = extract_arg(&args, "--resume") {
        Some(sid)
    } else if args.iter().any(|a| a == "--resume" || a == "-c") {
        // --resume (no arg) or -c: resume last session from .nocode/last_session
        let marker = std::path::Path::new(&cwd).join(".nocode/last_session");
        match std::fs::read_to_string(&marker) {
            Ok(sid) if !sid.trim().is_empty() => Some(sid.trim().to_string()),
            _ => {
                eprintln!("No previous session found. Start a new one with `nocode`.");
                return;
            }
        }
    } else {
        None
    };

    // No API key available → auto-launch login before TUI
    let needs_onboarding =
        !has_any_api_key() && !args.iter().any(|a| a == "--status" || a == "--help");
    if needs_onboarding {
        eprintln!("No API key found. Starting setup...\n");
        login::run_login(&cwd);
        // Reload credentials into env after login
        let cred_path2 = nocode_core::storage::credentials::CredentialStore::default_path();
        if let Ok(creds) = nocode_core::storage::credentials::CredentialStore::load(&cred_path2) {
            creds.load_into_env();
        }
        if !has_any_api_key() {
            eprintln!("No provider configured. Run `nocode --login` to set up.");
            return;
        }
    }
    // Reload settings in case login changed them
    let settings = if needs_onboarding {
        Settings::load_merged(&cwd)
    } else {
        settings
    };

    let provider_type = resolve_provider(&settings);
    let model = match resolve_model(&settings, &provider_type) {
        Some(m) => m,
        None => {
            eprintln!("No model configured. Starting setup...\n");
            login::run_login(&cwd);
            let settings = Settings::load_merged(&cwd);
            match resolve_model(&settings, &resolve_provider(&settings)) {
                Some(m) => m,
                None => {
                    eprintln!("No model configured. Run `nocode --login` to set up.");
                    return;
                }
            }
        }
    };
    let max_turns = settings.max_turns.unwrap_or(200);
    let caps = nocode_core::provider::model_caps::lookup(&model);
    let max_tokens = settings.max_tokens.unwrap_or(caps.max_output_tokens);

    let custom_sp = resolve_custom_system_prompt(&settings);
    let system_blocks = assembly::assemble_system_prompt(
        &cwd,
        &[],
        &TruncationBudget::default(),
        custom_sp.as_deref(),
    );

    let registry = ToolRegistry::with_defaults(&cwd);
    initialize_runtime_global_registry(&cwd, &settings);
    let (provider_box, provider_warnings) = build_provider(&provider_type, &settings);

    let is_tui_mode = !args.iter().any(|a| a.starts_with("--bridge"))
        && !args.iter().any(|a| a == "--status")
        && !args.iter().any(|a| a == "--ide-server")
        && !args.iter().any(|a| a == "--mcp-server")
        && !args.iter().any(|a| a == "--process-agent-daemon")
        && !args.iter().any(|a| a == "--process-agent-host")
        && std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal();
    if !is_tui_mode {
        for w in &provider_warnings {
            eprintln!("Warning: {w}");
        }
    }

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

    if let Some(bind_addr) = extract_arg(&args, "--ws-server") {
        run_ws_server_mode(
            ProviderBox::from_arc(Arc::from(provider_box)),
            &cwd,
            &system_blocks,
            &model,
            max_tokens,
            max_turns,
            &bind_addr,
        );
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

    if is_tui_mode {
        // Provider not usable → run login flow first, then re-resolve
        let (provider_box, model, max_tokens, registry, system_blocks) = if !provider_warnings
            .is_empty()
        {
            for w in &provider_warnings {
                eprintln!("Warning: {w}");
            }
            login::run_login(&cwd);
            // Reload everything after login
            let cred_path2 = nocode_core::storage::credentials::CredentialStore::default_path();
            if let Ok(creds) = nocode_core::storage::credentials::CredentialStore::load(&cred_path2)
            {
                creds.load_into_env();
            }
            let settings = Settings::load_merged(&cwd);
            let provider_type = resolve_provider(&settings);
            let model = resolve_model(&settings, &provider_type).unwrap_or_else(|| {
                eprintln!("No model configured. Run `nocode --login`.");
                std::process::exit(1);
            });
            let caps = nocode_core::provider::model_caps::lookup(&model);
            let max_tokens = settings.max_tokens.unwrap_or(caps.max_output_tokens);
            let custom_sp = resolve_custom_system_prompt(&settings);
            let system_blocks = assembly::assemble_system_prompt(
                &cwd,
                &[],
                &TruncationBudget::default(),
                custom_sp.as_deref(),
            );
            let registry = ToolRegistry::with_defaults(&cwd);
            initialize_runtime_global_registry(&cwd, &settings);
            let (provider_box, _) = build_provider(&provider_type, &settings);
            (provider_box, model, max_tokens, registry, system_blocks)
        } else {
            (provider_box, model, max_tokens, registry, system_blocks)
        };

        if let Err(e) = tui::run_tui(
            provider_box,
            registry,
            system_blocks,
            model,
            max_tokens,
            max_turns,
            vec![],
            resume_session_id.as_deref(),
        ) {
            eprintln!("TUI error: {e}");
        }

        // /update was triggered from inside the TUI — run build now that TUI has cleaned up
        if tui_commands::UPDATE_REQUESTED.load(std::sync::atomic::Ordering::SeqCst) {
            match run_self_update() {
                Ok(version) => {
                    eprintln!();
                    eprintln!("Updated to {version} — restart nocode to use the new version.");
                }
                Err(e) => {
                    eprintln!("Update failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        return;
    }

    eprintln!("nocode now runs in TUI mode only and requires an interactive TTY.");
    std::process::exit(2);
}

// --- PLACEHOLDER_REST ---

fn has_any_api_key() -> bool {
    nocode_core::provider::resolve::has_any_api_key()
}

fn resolve_provider(settings: &Settings) -> ModelProvider {
    if let Ok(p) = env::var("NOCODE_MODEL_PROVIDER")
        && let Some(provider) = ModelProvider::parse(&p)
    {
        return provider;
    }
    if let Some(provider) = settings
        .model_provider
        .as_deref()
        .and_then(ModelProvider::parse)
    {
        return provider;
    }
    if settings.custom_base_url.is_some() {
        return ModelProvider::Custom;
    }
    if env::var("NOCODE_CUSTOM_BASE_URL").is_ok() {
        return ModelProvider::Custom;
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

fn resolve_model(settings: &Settings, provider: &ModelProvider) -> Option<String> {
    // 1. Explicit global override
    if let Ok(m) = env::var("NOCODE_MODEL") {
        return Some(m);
    }
    // 2. Per-provider env var
    let per_provider_var = match provider {
        ModelProvider::Claude => "ANTHROPIC_MODEL",
        ModelProvider::OpenAi => "OPENAI_MODEL",
        ModelProvider::Gemini => "GEMINI_MODEL",
        ModelProvider::Custom => "",
    };
    if !per_provider_var.is_empty()
        && let Ok(m) = env::var(per_provider_var)
    {
        return Some(m);
    }
    // 3. Settings file
    settings.model.clone()
}

fn build_provider(
    provider: &ModelProvider,
    settings: &Settings,
) -> (Box<dyn Provider>, Vec<String>) {
    let mut warnings = Vec::new();
    let resolve_key = nocode_core::provider::resolve::resolve_api_key;
    let result: Box<dyn Provider> = match provider {
        ModelProvider::Claude => {
            let key = resolve_key(ModelProvider::Claude, settings);
            let base = env::var("ANTHROPIC_BASE_URL")
                .unwrap_or_else(|_| String::from("https://api.anthropic.com"));
            Box::new(ClaudeProvider::with_base_url(base, key))
        }
        ModelProvider::OpenAi => {
            let key = resolve_key(ModelProvider::OpenAi, settings);
            let base = env::var("OPENAI_BASE_URL")
                .unwrap_or_else(|_| String::from("https://api.openai.com"));
            Box::new(OpenAiResponsesProvider::with_base_url(base, key))
        }
        ModelProvider::Gemini => {
            let key = resolve_key(ModelProvider::Gemini, settings);
            Box::new(GeminiProvider::new(key))
        }
        ModelProvider::Custom => {
            use nocode_core::provider::resolve::{
                resolve_custom_api_format, resolve_custom_base_url,
            };
            let key = resolve_key(ModelProvider::Custom, settings);
            let base = match resolve_custom_base_url(settings) {
                Ok(url) => url,
                Err(msg) => {
                    eprintln!("Error: {msg}");
                    std::process::exit(1);
                }
            };
            let format = resolve_custom_api_format(settings);
            match format.as_str() {
                "anthropic" => {
                    if let Some(foundry_id) = base
                        .strip_prefix("https://")
                        .and_then(|s| s.strip_suffix(".foundry.anthropic.com"))
                    {
                        Box::new(FoundryProvider::new(foundry_id, &key))
                    } else {
                        Box::new(ClaudeProvider::with_base_url(base, key))
                    }
                }
                "openai-responses" => Box::new(OpenAiResponsesProvider::with_base_url(base, key)),
                "openai-chat" => Box::new(OpenAiProvider::with_base_url(base, key)),
                "google" => Box::new(GeminiProvider::new(key)),
                other => {
                    warnings.push(format!(
                        "Unknown custom_api_format '{other}', defaulting to openai-responses.\n\
                         Valid values: openai-responses, openai-chat, anthropic, google"
                    ));
                    Box::new(OpenAiResponsesProvider::with_base_url(base, key))
                }
            }
        }
    };
    (result, warnings)
}

// --- PLACEHOLDER_MODES ---

fn run_status(cwd: &str, provider: &ModelProvider, model: &str, settings: &Settings) {
    println!("nocode v{}", env!("CARGO_PKG_VERSION"));
    println!();
    println!("Working directory: {cwd}");
    println!("Provider: {}", provider.as_str());
    println!("Model: {model}");
    println!("Max turns: {}", settings.max_turns.unwrap_or(200));
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
        (format!("{home}/.nocode/config.toml"), "User"),
        (format!("{cwd}/.nocode/config.toml"), "Project"),
        (format!("{cwd}/.nocode/config.local.toml"), "Local"),
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
    match execute_prompt(
        provider,
        registry,
        system_blocks,
        model,
        max_tokens,
        max_turns,
        prompt,
    ) {
        Ok(text) => {
            if !text.is_empty() {
                println!("{text}");
            }
        }
        Err(e) => eprintln!("Error: {e}"),
    }
}

fn execute_prompt(
    provider: &dyn Provider,
    registry: &ToolRegistry,
    system_blocks: &[nocode_core::message::SystemBlock],
    model: &str,
    max_tokens: u32,
    max_turns: u32,
    prompt: &str,
) -> Result<String, String> {
    let messages = vec![Message::user_text(prompt)];
    let executor = ToolExecutor::new(registry);
    let config = LoopConfig {
        model: model.to_string(),
        max_tokens,
        max_turns,
        system: system_blocks.to_vec(),
        tools: tool_definitions_for_model(registry),
        parallel_tool_execution: true,
        reasoning_effort: None,
    };
    let mut observer = NoopObserver;
    let result = r#loop::run_agentic_loop(provider, &executor, &config, messages, &mut observer)
        .map_err(|e| e.to_string())?;

    Ok(result
        .messages
        .iter()
        .filter(|msg| msg.role == nocode_core::message::Role::Assistant)
        .map(|msg| msg.text_content())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n"))
}

fn run_bridge_remote_once(prompt: &str) {
    let base_url = env::var("NOCODE_BRIDGE_BASE_URL")
        .unwrap_or_else(|_| String::from("http://localhost:3000"));
    let auth_token = env::var("NOCODE_BRIDGE_AUTH_TOKEN").ok();

    if prompt.is_empty() {
        eprintln!("Usage: nocode --bridge-remote-once \"<prompt>\"");
        return;
    }

    let url = format!("{base_url}/v1/query");
    let mut request = serde_json::json!({
        "prompt": prompt,
    });

    // Add model if specified
    if let Ok(model) = env::var("NOCODE_MODEL") {
        request["model"] = serde_json::Value::String(model);
    }

    let client = reqwest::blocking::Client::new();
    let mut builder = client.post(&url).json(&request);

    if let Some(token) = &auth_token {
        builder = builder.header("Authorization", format!("Bearer {token}"));
    }

    match builder.send() {
        Ok(resp) => {
            if resp.status().is_success() {
                match resp.text() {
                    Ok(body) => {
                        // Try to parse as JSON and extract text
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                            if let Some(text) = json["text"].as_str() {
                                println!("{text}");
                            } else {
                                println!("{body}");
                            }
                        } else {
                            println!("{body}");
                        }
                    }
                    Err(e) => eprintln!("Failed to read response: {e}"),
                }
            } else {
                eprintln!(
                    "Bridge error: {} {}",
                    resp.status(),
                    resp.text().unwrap_or_default()
                );
            }
        }
        Err(e) => eprintln!("Failed to connect to bridge at {url}: {e}"),
    }
}

fn run_ws_server_mode(
    provider: ProviderBox,
    cwd: &str,
    system_blocks: &[nocode_core::message::SystemBlock],
    model: &str,
    max_tokens: u32,
    max_turns: u32,
    bind_addr: &str,
) {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("Failed to start tokio runtime for ws server: {e}");
            return;
        }
    };

    let cwd = cwd.to_string();
    let system = system_blocks.to_vec();
    let model = model.to_string();
    let provider = provider.clone();
    let config = nocode_core::ws_bridge::WsBridgeConfig {
        bind_addr: bind_addr.to_string(),
        ..Default::default()
    };

    let handler: nocode_core::ws_bridge::QueryHandler = Arc::new(move |query_id, prompt, tx| {
        let registry = ToolRegistry::with_defaults(&cwd);
        let executor = ToolExecutor::new(&registry);
        let config = LoopConfig {
            model: model.clone(),
            max_tokens,
            max_turns,
            system: system.clone(),
            tools: tool_definitions_for_model(&registry),
            parallel_tool_execution: true,
            reasoning_effort: None,
        };
        let mut observer =
            nocode_core::ws_bridge::WsEventObserver::new(query_id.clone(), tx.clone());
        let result = r#loop::run_agentic_loop(
            provider.as_ref(),
            &executor,
            &config,
            vec![Message::user_text(&prompt)],
            &mut observer,
        )
        .map_err(|e| e.to_string())?;

        let stop_reason = match result.stop_reason {
            nocode_core::provider::types::StopReason::EndTurn => "end_turn",
            nocode_core::provider::types::StopReason::ToolUse => "tool_use",
            nocode_core::provider::types::StopReason::MaxTokens => "max_tokens",
            nocode_core::provider::types::StopReason::PauseTurn => "pause_turn",
        };
        tx.send(nocode_core::ws_bridge::WsMessage::Complete {
            id: query_id,
            stop_reason: stop_reason.to_string(),
        })
        .map_err(|_| "failed to send complete event".to_string())
    });

    eprintln!(
        "nocode WebSocket bridge v{} — {}",
        env!("CARGO_PKG_VERSION"),
        bind_addr
    );
    if let Err(e) = runtime.block_on(nocode_core::ws_bridge::run_ws_server(config, handler)) {
        eprintln!("WebSocket bridge error: {e}");
    }
}

fn run_ide_server() {
    // IDE server: JSON-RPC over stdio for IDE extensions (VS Code, JetBrains).
    // Delegates to IdeRequestHandler for real query execution.
    use nocode_core::ide_server::{IdeRequestHandler, IdeServerConfig, parse_ide_request};
    use std::io::{self, BufRead, Write};

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    eprintln!(
        "nocode IDE server v{} — JSON-RPC over stdio",
        env!("CARGO_PKG_VERSION")
    );

    let cwd = env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| String::from("."));
    let settings = Settings::load_merged(&cwd);
    let provider_type = resolve_provider(&settings);
    let model = resolve_model(&settings, &provider_type).unwrap_or_else(|| {
        eprintln!("No model configured. Run `nocode --login`.");
        std::process::exit(1);
    });
    let registry = ToolRegistry::with_defaults(&cwd);
    let custom_sp = resolve_custom_system_prompt(&settings);
    let system_blocks = assembly::assemble_system_prompt(
        &cwd,
        &[],
        &TruncationBudget::default(),
        custom_sp.as_deref(),
    );
    let (provider_box, ide_warnings) = build_provider(&provider_type, &settings);
    for w in &ide_warnings {
        eprintln!("Warning: {w}");
    }
    let max_tokens = settings.max_tokens.unwrap_or(16384);
    let max_turns = settings.max_turns.unwrap_or(200);

    let handler = IdeRequestHandler::new(
        IdeServerConfig::default(),
        nocode_core::provider::ProviderBox::from_arc(Arc::from(provider_box)),
        registry,
        model,
        system_blocks,
        max_tokens,
        max_turns,
    );

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }

        let request = match parse_ide_request(&line) {
            Ok(r) => r,
            Err(e) => {
                let err_resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": { "code": -32700, "message": format!("Parse error: {e}") }
                });
                let _ = writeln!(stdout, "{}", err_resp);
                let _ = stdout.flush();
                continue;
            }
        };

        // Check for shutdown before handler (to break the loop)
        let is_shutdown = request.method == "shutdown";

        if let Some(response) = handler.handle(&request) {
            let _ = writeln!(stdout, "{}", response);
            let _ = stdout.flush();
        }

        if is_shutdown {
            break;
        }
    }
}

fn run_mcp_server() {
    // MCP server: expose nocode tools via MCP protocol over stdio.
    // Implements tools/list, tools/call, resources/list, resources/read, and query.
    use nocode_core::mcp::server::McpServer;
    use nocode_core::provider::ProviderBox;
    use std::io::{self, BufRead, Write};

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    eprintln!(
        "nocode MCP server v{} — stdio transport",
        env!("CARGO_PKG_VERSION")
    );

    let cwd = env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| String::from("."));
    let settings = Settings::load_merged(&cwd);
    let provider_type = resolve_provider(&settings);
    let model = resolve_model(&settings, &provider_type).unwrap_or_else(|| {
        eprintln!("No model configured. Run `nocode --login`.");
        std::process::exit(1);
    });
    let registry = ToolRegistry::with_defaults(&cwd);
    let custom_sp = resolve_custom_system_prompt(&settings);
    let system_blocks = assembly::assemble_system_prompt(
        &cwd,
        &[],
        &TruncationBudget::default(),
        custom_sp.as_deref(),
    );
    let (provider_box, mcp_warnings) = build_provider(&provider_type, &settings);
    for w in &mcp_warnings {
        eprintln!("Warning: {w}");
    }
    let max_tokens = settings.max_tokens.unwrap_or(16384);
    let max_turns = settings.max_turns.unwrap_or(200);

    let server = McpServer::with_provider(
        registry,
        ProviderBox::from_arc(Arc::from(provider_box)),
        model,
        system_blocks,
        max_tokens,
        max_turns,
        cwd,
    );

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }

        let request: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let err_resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": { "code": -32700, "message": format!("Parse error: {e}") }
                });
                let _ = writeln!(stdout, "{}", err_resp);
                let _ = stdout.flush();
                continue;
            }
        };

        // Parse into JsonRpcRequest
        let rpc_req = nocode_core::mcp::server::JsonRpcRequest {
            jsonrpc: request["jsonrpc"].as_str().unwrap_or("2.0").to_string(),
            id: request.get("id").cloned(),
            method: request["method"].as_str().unwrap_or("").to_string(),
            params: request
                .get("params")
                .cloned()
                .unwrap_or(serde_json::json!({})),
        };

        let is_shutdown = rpc_req.method == "shutdown";

        if let Some(response) = server.handle_request(&rpc_req) {
            let _ = writeln!(stdout, "{}", response);
            let _ = stdout.flush();
        }

        if is_shutdown {
            break;
        }
    }
}

fn run_agent_daemon() {
    // Agent daemon: long-running background process that manages agent workers.
    // Listens on stdin for spawn/stop/status commands, manages worker lifecycle.
    use std::io::{self, BufRead, Write};

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    eprintln!(
        "nocode agent daemon v{} — background process",
        env!("CARGO_PKG_VERSION")
    );

    use nocode_core::agent::worker::global_worker_registry;

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }

        let request: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let resp = serde_json::json!({ "error": format!("Parse error: {e}") });
                let _ = writeln!(stdout, "{}", resp);
                let _ = stdout.flush();
                continue;
            }
        };

        let action = request["action"].as_str().unwrap_or("");
        let response = match action {
            "status" => {
                let reg = global_worker_registry();
                let reg = reg.lock().unwrap_or_else(|e| e.into_inner());
                let workers = reg.list();
                let list: Vec<serde_json::Value> = workers
                    .iter()
                    .map(|w| {
                        serde_json::json!({
                            "id": w.id,
                            "name": w.name,
                            "state": format!("{:?}", w.state),
                        })
                    })
                    .collect();
                serde_json::json!({ "workers": list })
            }
            "spawn" => {
                let name = request["name"].as_str().unwrap_or("worker");
                let prompt = request["prompt"].as_str().unwrap_or("");
                let reg = global_worker_registry();
                let mut reg = reg.lock().unwrap_or_else(|e| e.into_inner());
                let id = reg.register(name, prompt);
                serde_json::json!({ "spawned": id })
            }
            "stop" => {
                let id = request["id"].as_str().unwrap_or("");
                let reg = global_worker_registry();
                let mut reg = reg.lock().unwrap_or_else(|e| e.into_inner());
                let removed = reg.remove(id).is_some();
                serde_json::json!({ "stopped": removed })
            }
            "shutdown" => {
                let resp = serde_json::json!({ "shutdown": true });
                let _ = writeln!(stdout, "{}", resp);
                let _ = stdout.flush();
                break;
            }
            _ => {
                serde_json::json!({ "error": format!("Unknown action: {action}") })
            }
        };

        let _ = writeln!(stdout, "{}", response);
        let _ = stdout.flush();
    }
}

fn run_agent_host() {
    // Agent host: single-shot agent execution for spawned sub-agents.
    // Reads a task from stdin, executes it, writes result to stdout.
    use std::io::{self, Read, Write};

    let mut stdin_buf = String::new();
    if io::stdin().read_to_string(&mut stdin_buf).is_err() {
        eprintln!("Failed to read stdin");
        std::process::exit(1);
    }

    let request: serde_json::Value = match serde_json::from_str(&stdin_buf) {
        Ok(v) => v,
        Err(e) => {
            let err = serde_json::json!({ "error": format!("Parse error: {e}") });
            println!("{}", err);
            std::process::exit(1);
        }
    };

    let prompt = request["prompt"].as_str().unwrap_or("");
    if prompt.is_empty() {
        let err = serde_json::json!({ "error": "Missing prompt" });
        println!("{}", err);
        std::process::exit(1);
    }

    let cwd = env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| String::from("."));
    let settings = Settings::load_merged(&cwd);
    let provider_type = resolve_provider(&settings);
    let model = resolve_model(&settings, &provider_type).unwrap_or_else(|| {
        eprintln!("No model configured. Run `nocode --login`.");
        std::process::exit(1);
    });
    let registry = ToolRegistry::with_defaults(&cwd);
    let custom_sp = resolve_custom_system_prompt(&settings);
    let system_blocks = assembly::assemble_system_prompt(
        &cwd,
        &[],
        &TruncationBudget::default(),
        custom_sp.as_deref(),
    );
    let (provider_box, host_warnings) = build_provider(&provider_type, &settings);
    for w in &host_warnings {
        eprintln!("Warning: {w}");
    }
    let executor = ToolExecutor::new(&registry);
    let max_tokens = settings.max_tokens.unwrap_or(16384);
    let max_turns = settings.max_turns.unwrap_or(200);

    let messages = vec![Message::user_text(prompt)];
    let config = r#loop::LoopConfig {
        model,
        max_tokens,
        max_turns,
        system: system_blocks,
        tools: tool_definitions_for_model(&registry),
        parallel_tool_execution: true,
        reasoning_effort: settings.reasoning_effort.clone(),
    };
    let mut observer = r#loop::NoopObserver;

    let result = match r#loop::run_agentic_loop(
        provider_box.as_ref(),
        &executor,
        &config,
        messages,
        &mut observer,
    ) {
        Ok(result) => {
            let text: String = result
                .messages
                .iter()
                .filter(|m| m.role == nocode_core::message::Role::Assistant)
                .map(|m| m.text_content())
                .collect::<Vec<_>>()
                .join("\n");
            serde_json::json!({
                "text": text,
                "input_tokens": result.total_input_tokens,
                "output_tokens": result.total_output_tokens,
            })
        }
        Err(e) => {
            serde_json::json!({ "error": format!("{e}") })
        }
    };

    let mut stdout = io::stdout();
    let _ = writeln!(stdout, "{}", result);
    let _ = stdout.flush();
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
         Usage: nocode\n\
         \x20       nocode [OPTIONS]\n\
         \n\
         Interactive:\n\
         \x20 nocode                    Terminal UI (default and only interactive mode)\n\
         \n\
         Modes:\n\
         \x20 --status                  System diagnostics\n\
         \x20 --bridge-once \"prompt\"    Single-turn local execution\n\
         \x20 --bridge-remote-once \"p\"  Single-turn HTTP bridge\n\
         \x20 --ws-server <addr>        WebSocket bridge server\n\
         \x20 --ide-server              IDE server mode\n\
         \x20 --mcp-server              MCP server mode\n\
         \x20 --process-agent-daemon    Background agent daemon\n\
         \x20 --process-agent-host      Agent host process\n\
         \n\
         Options:\n\
         \x20 --version, -v             Show version\n\
         \x20 --update                  Pull latest source from GitHub and rebuild binary\n\
         \x20 --login                   Interactive provider setup\n\
         \x20 --resume [session_id]     Resume a previous session (omit id for last session)\n\
         \x20 -c                        Shorthand for --resume (continue last session)\n\
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn resolve_provider_prefers_custom_settings() {
        let settings = Settings {
            custom_base_url: Some("https://example.invalid".to_string()),
            ..Default::default()
        };
        assert_eq!(resolve_provider(&settings), ModelProvider::Custom);
    }

    #[test]
    fn resolve_model_returns_none_without_config() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        let nocode_model = std::env::var("NOCODE_MODEL").ok();
        let anthropic_model = std::env::var("ANTHROPIC_MODEL").ok();
        let openai_model = std::env::var("OPENAI_MODEL").ok();
        let gemini_model = std::env::var("GEMINI_MODEL").ok();

        unsafe {
            std::env::remove_var("NOCODE_MODEL");
            std::env::remove_var("ANTHROPIC_MODEL");
            std::env::remove_var("OPENAI_MODEL");
            std::env::remove_var("GEMINI_MODEL");
        }

        let settings = Settings::default();
        assert!(resolve_model(&settings, &ModelProvider::Claude).is_none());
        assert!(resolve_model(&settings, &ModelProvider::OpenAi).is_none());
        assert!(resolve_model(&settings, &ModelProvider::Gemini).is_none());
        assert!(resolve_model(&settings, &ModelProvider::Custom).is_none());

        unsafe {
            if let Some(value) = nocode_model {
                std::env::set_var("NOCODE_MODEL", value);
            }
            if let Some(value) = anthropic_model {
                std::env::set_var("ANTHROPIC_MODEL", value);
            }
            if let Some(value) = openai_model {
                std::env::set_var("OPENAI_MODEL", value);
            }
            if let Some(value) = gemini_model {
                std::env::set_var("GEMINI_MODEL", value);
            }
        }
    }

    #[test]
    fn resolve_model_settings_override() {
        let settings = Settings {
            model: Some("my-custom-model".to_string()),
            ..Default::default()
        };
        assert_eq!(
            resolve_model(&settings, &ModelProvider::OpenAi),
            Some("my-custom-model".to_string())
        );
    }
}
