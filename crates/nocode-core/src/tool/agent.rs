//! Agent tool — spawn subagent workers on background threads.

use crate::agent::worker::{WorkerState, global_worker_registry};
use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};

pub struct AgentTool;

impl Tool for AgentTool {
    fn name(&self) -> &str {
        "Agent"
    }
    fn description(&self) -> &str {
        "Launch a new agent to handle complex, multi-step tasks autonomously."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "description":{"type":"string","description":"A short (3-5 word) description of the task"},
            "prompt":{"type":"string","description":"The task for the agent to perform"},
            "subagent_type":{"type":"string","description":"The type of specialized agent to use for this task"},
            "model":{"type":"string","description":"Optional model override for this agent. Pass the full model name (e.g. gpt-4.1, gemini-2.5-pro, claude-sonnet-4-20250514)."},
            "run_in_background":{"type":"boolean","description":"Set to true to run this agent in the background"},
            "name":{"type":"string","description":"Name for the spawned agent. Makes it addressable via SendMessage({to: name}) while running."},
            "team_name":{"type":"string","description":"Team name for spawning. Uses current team context if omitted."},
            "mode":{"type":"string","enum":["acceptEdits","bypassPermissions","default","dontAsk","plan"],"description":"Permission mode for spawned teammate"},
            "isolation":{"type":"string","enum":["worktree"],"description":"Isolation mode. 'worktree' creates a temporary git worktree."}
        },"required":["description","prompt"]})
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(prompt) = input["prompt"].as_str() else {
            return ToolOutput::error("Missing required parameter: prompt");
        };
        let description = input["description"].as_str().unwrap_or("");
        let name = input["name"].as_str().unwrap_or("agent");
        let run_in_background = input["run_in_background"].as_bool().unwrap_or(false);
        let model_override = input["model"].as_str();
        // Permission mode for the sub-agent — propagated through the fractal so
        // a parent running in Ask mode does not implicitly grant its child
        // full Auto access. Defaults to inheriting the parent setting.
        let mode_override = input["mode"].as_str().and_then(parse_subagent_mode);
        let prompt_owned = prompt.to_string();
        let name_owned = name.to_string();
        let model_owned = model_override.map(String::from);

        // Register worker
        let registry = global_worker_registry();
        let id = {
            let mut guard = registry.lock().unwrap();
            let id = guard.register(&name_owned, &prompt_owned);
            guard.set_state(&id, WorkerState::ReadyForPrompt);
            guard.set_timeout(&id, 300); // 5 minute default
            id
        };

        let worker_id = id.clone();

        if run_in_background {
            // Async: spawn and return immediately
            std::thread::spawn(move || {
                run_worker_thread(
                    &worker_id,
                    &prompt_owned,
                    model_owned.as_deref(),
                    mode_override,
                );
            });
            ToolOutput::success(
                json!({"worker_id": id, "name": name_owned, "description": description, "status": "spawned"}).to_string(),
            )
        } else {
            // Sync: spawn, wait for completion, return result
            let handle = std::thread::spawn(move || {
                run_worker_thread(
                    &worker_id,
                    &prompt_owned,
                    model_owned.as_deref(),
                    mode_override,
                );
            });
            let _ = handle.join();

            // Read result from registry
            let guard = registry.lock().unwrap();
            if let Some(worker) = guard.get(&id) {
                match worker.state {
                    WorkerState::Finished => {
                        let result = worker.result.clone().unwrap_or_default();
                        ToolOutput::success(result)
                    }
                    WorkerState::Failed => {
                        let error = worker
                            .error
                            .clone()
                            .unwrap_or_else(|| "Unknown error".to_string());
                        ToolOutput::error(error)
                    }
                    _ => ToolOutput::error("Agent ended in unexpected state"),
                }
            } else {
                ToolOutput::error("Agent worker not found after execution")
            }
        }
    }
}

/// Map the AgentTool `mode` schema enum to a [`crate::tool::permission::PermissionMode`].
///
/// `acceptEdits` and `bypassPermissions` map to `Auto`; `dontAsk` maps to
/// `Auto` as well (model proceeds without prompts). `default` keeps the parent
/// mode (returns `None` so the worker uses its own settings). `plan` is
/// orthogonal and handled by `EnterPlanMode` — we treat it as `ReadOnly` here
/// to give a defensible read-only default.
fn parse_subagent_mode(s: &str) -> Option<crate::tool::permission::PermissionMode> {
    use crate::tool::permission::PermissionMode;
    match s {
        "acceptEdits" | "bypassPermissions" | "dontAsk" => Some(PermissionMode::Auto),
        "plan" => Some(PermissionMode::ReadOnly),
        "default" => None,
        _ => None,
    }
}

/// Execute a worker's prompt on a background thread.
/// Builds provider, tool registry, and agentic loop from global config.
pub fn run_worker_thread(
    worker_id: &str,
    prompt: &str,
    model_override: Option<&str>,
    permission_mode_override: Option<crate::tool::permission::PermissionMode>,
) {
    use crate::agent::worker::WorkerObserver;
    use crate::config::settings::Settings;
    use crate::message::Message;
    use crate::prompt::assembly::{self, TruncationBudget};
    use crate::query::budget::TokenBudget;
    use crate::query::r#loop::{self, LoopConfig};
    use crate::tool::ToolRegistry;
    use crate::tool::executor::ToolExecutor;
    use crate::tool::global_registry::{
        initialize_runtime_global_registry, tool_definitions_for_model,
    };

    let registry = global_worker_registry();

    // Grab cancel token and event sender before marking running
    let cancel_token = {
        let guard = registry.lock().unwrap();
        guard.get_cancel_token(worker_id)
    };

    let event_tx = {
        let guard = registry.lock().unwrap();
        guard.event_sender()
    };

    // Mark running + record start time
    {
        let mut guard = registry.lock().unwrap();
        guard.set_state(worker_id, WorkerState::Running);
        guard.mark_started(worker_id);
    }

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| String::from("."));

        let settings = Settings::load_merged(&cwd);
        let model = model_override
            .map(String::from)
            .or_else(|| std::env::var("NOCODE_MODEL").ok())
            .or_else(|| settings.model.clone())
            .unwrap_or_else(|| String::from("claude-sonnet-4-20250514"));
        let max_tokens = settings.max_tokens.unwrap_or(16384);
        let max_turns = settings.max_turns.unwrap_or(10);

        let provider_type = resolve_worker_provider(&settings);
        let provider = build_worker_provider(&provider_type, &settings);

        let tool_registry = ToolRegistry::with_defaults(&cwd);
        initialize_runtime_global_registry(&cwd, &settings);
        let custom_sp_env = std::env::var("NOCODE_SYSTEM_PROMPT").ok();
        let custom_sp = custom_sp_env
            .as_deref()
            .or(settings.system_prompt.as_deref());
        let system_blocks =
            assembly::assemble_system_prompt(&cwd, &[], &TruncationBudget::default(), custom_sp);

        // Build executor with the parent-supplied permission mode (if any).
        // This is the load-bearing line for fractal safety: without it, every
        // sub-agent silently runs in Auto regardless of how cautious the parent
        // is configured to be.
        let executor = match permission_mode_override {
            Some(mode) => ToolExecutor::new(&tool_registry).with_permission_mode(mode),
            None => ToolExecutor::new(&tool_registry),
        };

        let messages = vec![Message::user_text(prompt)];
        let mut budget = TokenBudget::for_model(&model, Some(max_tokens));
        let config = LoopConfig {
            model,
            max_tokens,
            max_turns,
            system: system_blocks,
            tools: tool_definitions_for_model(&tool_registry),
            parallel_tool_execution: true,
            reasoning_effort: None,
        };
        let loop_result = if let Some(tx) = event_tx {
            let mut observer = WorkerObserver {
                worker_id: worker_id.to_string(),
                tx,
            };
            r#loop::run_agentic_loop_with_cancel(
                provider.as_ref(),
                &executor,
                &config,
                messages,
                &mut observer,
                &mut budget,
                cancel_token.clone(),
            )?
        } else {
            let mut observer = r#loop::NoopObserver;
            r#loop::run_agentic_loop_with_cancel(
                provider.as_ref(),
                &executor,
                &config,
                messages,
                &mut observer,
                &mut budget,
                cancel_token.clone(),
            )?
        };

        // Extract assistant text from result
        let text: String = loop_result
            .messages
            .iter()
            .filter(|m| m.role == crate::message::Role::Assistant)
            .map(|m| m.text_content())
            .collect::<Vec<_>>()
            .join("\n");

        Ok::<String, Box<dyn std::error::Error + Send + Sync>>(text)
    }));

    // Write result back to registry (auto-sends WorkerEvent via set_result/set_error)
    let mut guard = registry.lock().unwrap();
    match result {
        Ok(Ok(text)) => {
            guard.set_result(worker_id, text);
        }
        Ok(Err(e)) => {
            guard.set_error(worker_id, format!("{e}"));
        }
        Err(_panic) => {
            guard.set_error(worker_id, "Worker panicked".to_string());
        }
    }
}

fn resolve_worker_provider(
    settings: &crate::config::settings::Settings,
) -> crate::provider::types::ModelProvider {
    // Sub-agents share the parent's named-provider table — same precedence
    // chain, no env-var probing.
    crate::provider::resolve::resolve_named_provider(settings, None, None)
        .map(|r| r.legacy_model_provider())
        .unwrap_or(crate::provider::types::ModelProvider::Claude)
}

fn build_worker_provider(
    _provider: &crate::provider::types::ModelProvider,
    settings: &crate::config::settings::Settings,
) -> Box<dyn crate::provider::Provider> {
    use crate::provider::claude::ClaudeProvider;
    use crate::provider::gemini::GeminiProvider;
    use crate::provider::openai::OpenAiProvider;
    use crate::provider::openai_responses::OpenAiResponsesProvider;
    use crate::provider::resolve::resolve_named_provider;

    let resolved = match resolve_named_provider(settings, None, None) {
        Ok(r) => r,
        Err(msg) => {
            eprintln!("Agent error: {msg}");
            // Best-effort fallback so the worker still has *something* to call;
            // misconfigured provider will fail at request time with a proper
            // status code instead of a panic.
            return Box::new(OpenAiResponsesProvider::with_base_url(
                "https://api.openai.com".to_owned(),
                String::new(),
            ));
        }
    };

    match resolved.wire_api.as_str() {
        "anthropic" => Box::new(ClaudeProvider::with_base_url(
            resolved.base_url,
            resolved.api_key,
        )),
        "openai-chat" => Box::new(OpenAiProvider::with_base_url(
            resolved.base_url,
            resolved.api_key,
        )),
        "google" => Box::new(GeminiProvider::new(resolved.api_key)),
        _ => Box::new(OpenAiResponsesProvider::with_base_url(
            resolved.base_url,
            resolved.api_key,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn agent_missing_prompt() {
        let tool = AgentTool;
        let result = tool.execute(&json!({"description": "test"}));
        assert!(result.is_error);
    }

    #[test]
    fn parse_subagent_mode_maps_strings_to_permission_modes() {
        use crate::tool::permission::PermissionMode;
        assert_eq!(
            parse_subagent_mode("acceptEdits"),
            Some(PermissionMode::Auto)
        );
        assert_eq!(
            parse_subagent_mode("bypassPermissions"),
            Some(PermissionMode::Auto)
        );
        assert_eq!(parse_subagent_mode("dontAsk"), Some(PermissionMode::Auto));
        assert_eq!(parse_subagent_mode("plan"), Some(PermissionMode::ReadOnly));
        assert_eq!(parse_subagent_mode("default"), None);
        assert_eq!(parse_subagent_mode("nonsense"), None);
    }

    #[test]
    fn agent_spawns_worker() {
        let tool = AgentTool;
        let result = tool.execute(&json!({
            "description": "test task",
            "prompt": "echo hello",
            "name": "test-agent",
            "run_in_background": true
        }));
        assert!(!result.is_error);
        assert!(result.content.contains("worker_id"));
        assert!(result.content.contains("test-agent"));
        assert!(result.content.contains("spawned"));
    }

    #[test]
    fn agent_default_name() {
        let tool = AgentTool;
        let result = tool.execute(&json!({
            "description": "d",
            "prompt": "p",
            "run_in_background": true
        }));
        assert!(!result.is_error);
        assert!(result.content.contains("agent"));
    }

    #[test]
    fn agent_model_override() {
        let tool = AgentTool;
        let result = tool.execute(&json!({
            "description": "d",
            "prompt": "p",
            "model": "gpt-4.1",
            "run_in_background": true
        }));
        assert!(!result.is_error);
    }
}
