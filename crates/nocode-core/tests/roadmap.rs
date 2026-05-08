//! Integration test: architecture roadmap verification.
//!
//! Validates that all key modules, singletons, and features exist.

use std::path::Path;

fn workspace_root() -> String {
    // CARGO_MANIFEST_DIR points to crates/nocode-core, go up two levels
    let manifest = env!("CARGO_MANIFEST_DIR");
    Path::new(manifest)
        .parent()
        .and_then(|p| p.parent())
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

fn core_src() -> String {
    format!("{}/crates/nocode-core/src", workspace_root())
}

fn cli_src() -> String {
    format!("{}/crates/nocode/src", workspace_root())
}

// ---------------------------------------------------------------------------
// Module existence checks
// ---------------------------------------------------------------------------

#[test]
fn core_modules_exist() {
    let required = [
        "lib.rs",
        "message.rs",
        "recovery.rs",
        "provider/mod.rs",
        "provider/claude.rs",
        "provider/openai.rs",
        "provider/gemini.rs",
        "provider/transport.rs",
        "provider/types.rs",
        "tool/mod.rs",
        "tool/bash.rs",
        "tool/read.rs",
        "tool/write.rs",
        "tool/edit.rs",
        "tool/glob.rs",
        "tool/grep.rs",
        "tool/agent.rs",
        "tool/web.rs",
        "tool/executor.rs",
        "tool/permission.rs",
        "tool/trust.rs",
        "tool/definitions.rs",
        "tool/tool_validation.rs",
        "tool/bash_validation.rs",
        "tool/file_safety.rs",
        "tool/hook_runner.rs",
        "tool/global_registry.rs",
        "tool/plugin_registry.rs",
        "tool/lsp_registry.rs",
        "tool/task_tools.rs",
        "tool/team_tools.rs",
        "tool/memory_tools.rs",
        "tool/cron_tools.rs",
        "tool/discovery_tools.rs",
        "tool/mcp_tools.rs",
        "tool/session_tools.rs",
        "tool/interactive_tools.rs",
        "tool/send_message.rs",
        "tool/skill.rs",
        "query/mod.rs",
        "query/loop.rs",
        "query/budget.rs",
        "query/events.rs",
        "query/deps.rs",
        "session/mod.rs",
        "session/persistence.rs",
        "session/compaction.rs",
        "session/control.rs",
        "config/mod.rs",
        "config/settings.rs",
        "config/claude_md.rs",
        "config/runtime.rs",
        "prompt/mod.rs",
        "prompt/system.rs",
        "prompt/assembly.rs",
        "agent/mod.rs",
        "agent/worker.rs",
        "agent/task.rs",
        "mcp/mod.rs",
        "mcp/client.rs",
        "mcp/bridge.rs",
        "mcp/manager.rs",
        "storage/mod.rs",
        "storage/sql.rs",
        "storage/memory.rs",
    ];

    for module in &required {
        let core = core_src();
        let path = format!("{core}/{module}");
        assert!(Path::new(&path).exists(), "Missing core module: {module}");
    }
}

#[test]
fn cli_modules_exist() {
    let required = [
        "main.rs",
        "tui.rs",
        "tui_app.rs",
        "tui_theme.rs",
        "tui_widgets.rs",
        "tui_input.rs",
        "tui_permission.rs",
        "command_registry.rs",
        "login.rs",
        "model_fetch.rs",
        "provider_presets.rs",
        "markdown_render.rs",
        "markdown_stream.rs",
        "status_hud.rs",
        "spinner.rs",
        "tool_render.rs",
        "tool_truncate.rs",
    ];

    for module in &required {
        let cli = cli_src();
        let path = format!("{cli}/{module}");
        assert!(Path::new(&path).exists(), "Missing CLI module: {module}");
    }
}

// ---------------------------------------------------------------------------
// Feature verification
// ---------------------------------------------------------------------------

#[test]
fn tool_registry_has_21_tools() {
    let reg = nocode_core::tool::ToolRegistry::with_defaults("/tmp");
    let names = reg.names();
    assert_eq!(
        names.len(),
        21,
        "Expected exactly 21 tools (Claude Code parity), got {}",
        names.len()
    );
}

#[test]
fn tool_categories_complete() {
    let reg = nocode_core::tool::ToolRegistry::with_defaults("/tmp");
    // All 21 Claude Code tools
    for name in &[
        "Agent",
        "AskUserQuestion",
        "Bash",
        "Config",
        "EnterWorktree",
        "ExitPlanMode",
        "ExitWorktree",
        "FileEdit",
        "FileRead",
        "FileWrite",
        "Glob",
        "Grep",
        "ListMcpResources",
        "Mcp",
        "NotebookEdit",
        "ReadMcpResource",
        "TaskOutput",
        "TaskStop",
        "TodoWrite",
        "WebFetch",
        "WebSearch",
    ] {
        assert!(reg.get(name).is_some(), "Missing Claude Code tool: {name}");
    }
}

#[test]
fn provider_types_complete() {
    use nocode_core::provider::types::ModelProvider;
    assert_eq!(ModelProvider::parse("claude"), Some(ModelProvider::Claude));
    assert_eq!(ModelProvider::parse("openai"), Some(ModelProvider::OpenAi));
    assert_eq!(ModelProvider::parse("gemini"), Some(ModelProvider::Gemini));
    assert_eq!(ModelProvider::parse("custom"), Some(ModelProvider::Custom));
}

#[test]
fn stop_reasons_complete() {
    use nocode_core::provider::types::StopReason;
    let _end = StopReason::EndTurn;
    let _tool = StopReason::ToolUse;
    let _max = StopReason::MaxTokens;
    let _pause = StopReason::PauseTurn;
}

#[test]
fn content_block_variants_complete() {
    use nocode_core::message::ContentBlock;
    let _text = ContentBlock::text("hello");
    let _tool_use = ContentBlock::ToolUse {
        id: "id".to_string(),
        name: "Bash".to_string(),
        input: serde_json::json!({}),
    };
    let _tool_result = ContentBlock::tool_result("id", "output");
    let _tool_error = ContentBlock::tool_error("id", "error");
    let _thinking = ContentBlock::Thinking {
        thinking: "hmm".to_string(),
    };
}

#[test]
fn worker_states_complete() {
    use nocode_core::agent::worker::WorkerState;
    let _spawning = WorkerState::Spawning;
    let _trust = WorkerState::TrustRequired;
    let _ready = WorkerState::ReadyForPrompt;
    let _running = WorkerState::Running;
    let _finished = WorkerState::Finished;
    let _failed = WorkerState::Failed;
}

#[test]
fn plugin_states_complete() {
    use nocode_core::tool::plugin_registry::PluginState;
    let _unconfigured = PluginState::Unconfigured;
    let _validated = PluginState::Validated;
    let _healthy = PluginState::Healthy;
    let _degraded = PluginState::Degraded;
    let _failed = PluginState::Failed;
}

#[test]
fn recovery_scenarios_complete() {
    use nocode_core::recovery::{FailureScenario, RecoveryRecipe};
    let scenarios = [
        FailureScenario::TransientNetwork,
        FailureScenario::RateLimited,
        FailureScenario::AuthFailure,
        FailureScenario::ModelOverloaded,
        FailureScenario::ContextOverflow,
        FailureScenario::ToolFailure,
        FailureScenario::Fatal,
    ];
    for s in &scenarios {
        let recipe = RecoveryRecipe::for_scenario(*s);
        assert!(!recipe.actions.is_empty(), "Empty recipe for {s:?}");
        assert!(recipe.max_attempts >= 1);
    }
}

#[test]
fn sql_schema_has_all_tables() {
    let dir = std::env::temp_dir().join("nocode_roadmap_sql_test");
    let _ = std::fs::remove_dir_all(&dir);
    let store = nocode_core::storage::sql::SqlStore::new(dir.to_str().unwrap()).unwrap();

    // Verify all 5 tables work
    store.create_session("roadmap-test", "mock").unwrap();
    store
        .insert_message("roadmap-test", "user", "hi", 1, 5)
        .unwrap();
    store
        .insert_command("test cmd", Some("roadmap-test"))
        .unwrap();
    store
        .insert_memory("test", "user", None, "content", None)
        .unwrap();
    store.insert_telemetry("test_event", None, None).unwrap();

    let _ = std::fs::remove_dir_all(&dir);
}
