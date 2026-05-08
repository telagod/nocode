# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is nocode

A terminal-native AI coding assistant built in Rust. Connects to Claude, OpenAI, Gemini, or any compatible endpoint via Custom provider. The interface is a TUI plus interactive login/bootstrap flows.

## Build & Test Commands

```bash
cargo build                                     # debug build (default: full features)
cargo build --release                           # release build
cargo build --no-default-features -p nocode-core --features minimal  # minimal core (no MCP/plugins/telemetry/oauth)
cargo test                                      # all tests (~785 across 7 test binaries)
cargo test -p nocode-core                       # core library only
cargo test -p nocode                            # CLI binary only
cargo test <test_name>                          # single test by name
cargo clippy --all-targets -- -D warnings       # lint (all + pedantic + nursery)
cargo fmt --check                               # format check
cargo fmt                                       # auto-format
```

CI runs `fmt → clippy → test` on every push/PR to main (`.github/workflows/ci.yml`). Release is tag-triggered (`v*`) via `.github/workflows/release.yml`.

## Workspace Layout

Two-crate Cargo workspace (edition 2024, clippy all+pedantic+nursery, unsafe forbidden):

```
crates/nocode-core/   — library (~100 modules, ~42K LOC), all core logic
crates/nocode/        — binary, CLI/TUI shell (~22 source files)
```

### Cargo Features

nocode-core: `full` (default) = `mcp` + `plugins` + `telemetry` + `oauth`. `minimal` = stripped-down core.
nocode: `full` (default) = `tui` + `nocode-core/full`. `minimal` = `nocode-core/minimal`.

## Architecture

### Core Data Flow

```
User Input → main.rs (mode dispatch) → QueryEngine (9-state loop)
  → Provider trait (Claude/OpenAI/Gemini/Custom) → SSE stream
  → tool calls? → ToolExecutor → permission check → hook → sandbox → execute
  → result back to QueryEngine loop
```

### Provider System

**Trait-based dispatch**: `provider/mod.rs` defines the `Provider` trait (`create_message`, `create_message_stream`, `create_message_stream_with_cancel`, `verify_key`). Concrete implementations: `ClaudeProvider`, `OpenAiResponsesProvider`, `GeminiProvider`, plus `FoundryProvider` for Anthropic-format proxies.

`ProviderBox` wraps `Arc<dyn Provider>` for owned trait objects. `build_provider()` in `main.rs` constructs the concrete provider based on `ModelProvider` enum.

**`ModelProvider` enum** (4 variants, Copy): `Claude | OpenAi | Gemini | Custom`. This is a label/selector enum — the actual dispatch goes through the `Provider` trait, not enum match arms.

**API format** is string-based (not an enum): 4 canonical values `openai-responses`, `openai-chat`, `anthropic`, `google`. Legacy values auto-normalized via `normalize_api_format()` in `config/settings.rs`. Used only for Custom provider routing.

**Provider auto-detection** priority (in `main.rs:resolve_provider`): `NOCODE_MODEL_PROVIDER` env → settings `model_provider` → Custom (if `custom_base_url` set) → `ANTHROPIC_API_KEY` → `OPENAI_API_KEY` → `GEMINI_API_KEY` → fallback Claude.

**Default models**: Claude → `claude-sonnet-4-20250514`, OpenAI → `gpt-4.1`, Gemini → `gemini-2.5-pro`.

### Query Engine

`query/loop.rs` drives the agentic conversation loop. Each iteration: send messages to the model via `provider/transport.rs` (SSE streaming), parse the response, execute any tool calls through `ToolExecutor`, feed tool results back, and loop until the model stops requesting tools or budget is exhausted.

### Dependency Injection

`query/deps.rs` defines trait objects (`CallModel`, `Compactor`, `ToolRunner`, `StopHookRunner`, `Clock`, `IdGen`) wired via `QueryDepsBuilder`. Tests swap in mocks; production uses `RichCompactor` and `DefaultToolExecutor`. This is the seam for all testability.

### Tool Execution Pipeline

Every tool call flows through: JSON Schema validation (`tool/tool_validation.rs`) → permission check against `PermissionMode` (ReadOnly/WorkspaceWrite/DangerFullAccess) → `PermissionPrompter` → hook runner (PreToolUse can deny) → sandbox enforcement → execution → PostToolUse hooks. `tool/executor.rs` orchestrates this chain.

Bash commands get extra validation through 6 submodules in `tool/bash_validation.rs` (read_only, destructive, mode, sed, path, semantics). File operations go through `tool/file_safety.rs` (symlink escape prevention, binary detection, 10MB limit).

### Configuration (3-tier hierarchy)

```
~/.nocode/settings.json              → User tier (global)
{cwd}/.nocode/settings.json          → Project tier
{cwd}/.nocode/settings.local.json    → Local tier (gitignored)
```

Later tiers override earlier. Scalars: last-wins. Maps: merged key-by-key. Vecs: replaced wholesale. Environment variables override on top. `RuntimeConfig` (in `config/runtime.rs`) holds resolved model, permission_mode, system_prompt, mcp_servers, hooks, sandbox.

### State Machines

Explicit enum state machines rather than inferred state:
- Worker lifecycle: Spawning → TrustRequired → ReadyForPrompt → Running → Finished/Failed
- MCP lifecycle: 11 phases from Registered → Shutdown (with Degraded/Reconnecting branches)
- Plugin lifecycle: Unconfigured → Validated → Healthy/Degraded/Failed

### Global Singletons

`OnceLock<Arc<Mutex<T>>>` pattern for: `TaskCoordinator`, `WorkerRegistry`, `McpManager`, `HookRunner`, `CronRegistry`, `PluginRegistry`, `SqlStore`, `GlobalToolRegistry`, `LspRegistry`. Access via `global_*()` functions.

### Storage

- SQL: `storage/sql.rs` — rusqlite with date-based volume partitioning (`~/.nocode/data/nocode_YYYY-MM-DD.db`)
- Memory: `storage/memory.rs` — file-system CRUD with Markdown+YAML frontmatter, MEMORY.md index
- Session: `session/persistence.rs` — JSONL transcript/history/task persistence with auto-persist on submission
- Credentials: `storage/credentials.rs` — encrypted API key storage

### Session Management

`session/compaction.rs` (RichCompactor) produces structured summaries when context grows too large. `session/control.rs` supports fork/branch/resume/suspend/complete with parent_id tracking. Token budget tracked in `query/budget.rs` with diminishing returns logic.

### Recovery

`recovery.rs` maps 7 failure scenarios to `RecoveryRecipe` (steps + max_attempts + escalation). Hard rule: one attempt before escalation, never silently retry indefinitely.

### Testing Strategy

- Unit tests: inline `#[cfg(test)]` modules throughout
- Integration tests: `crates/nocode-core/tests/` — 4 files: `mock_service.rs` (mock parity), `tool_roundtrip.rs` (tool execution), `trust_mcp.rs` (trust/permission/MCP health), `roadmap.rs`
- `MockAnthropicService` provides deterministic scenarios with `CapturedRequest` recording
- Query engine tests in `query/` module test suite

## Key Conventions

- `ModelProvider` is a Copy enum (4 variants). Provider dispatch is trait-based (`dyn Provider`), not enum-based.
- API format is string-based with 4 canonical values, not a Rust enum.
- Global singletons: `OnceLock<Arc<Mutex<T>>>`, accessed via `global_*()` functions.
- Explicit enum state machines over inferred state for all lifecycles.
- Structured typed events for observability — never scrape prose.
- One recovery attempt before escalation — never silently retry indefinitely.
- System prompt assembled dynamically by `prompt/assembly.rs` — discovers CLAUDE.md variants, deduplicates by FNV hash, applies truncation budgets.
- Slash commands registered in `command_registry.rs` with aliases/summary/argument_hint.
- Source paths use module subdirectories: `provider/`, `tool/`, `mcp/`, `query/`, `config/`, `session/`, `storage/`, `auth/`, `agent/`, `prompt/`.

## Environment Variables

- `NOCODE_MODEL_PROVIDER` — force provider (`anthropic`, `openai`, `google`, `custom`)
- `NOCODE_MODEL` — override model name
- `NOCODE_CUSTOM_BASE_URL` — Custom provider base URL (required for `custom` provider)
- `NOCODE_CUSTOM_API_FORMAT` — Custom provider API format (`openai-responses`, `openai-chat`, `anthropic`, `google`)
- `NOCODE_SYSTEM_PROMPT` — override system prompt
- `NOCODE_MODEL_REASONING_EFFORT` — `low`, `medium`, `high`
- `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` / `GEMINI_API_KEY` — provider API keys
- `ANTHROPIC_MODEL` / `OPENAI_MODEL` / `GEMINI_MODEL` — per-provider model override
- `ANTHROPIC_BASE_URL` / `OPENAI_BASE_URL` — per-provider base URL override
- `NOCODE_BRIDGE_BASE_URL` / `NOCODE_BRIDGE_AUTH_TOKEN` — remote bridge config
- Preset env keys: `OPENROUTER_API_KEY`, `TOGETHER_API_KEY`, `GROQ_API_KEY`, `FIREWORKS_API_KEY`, `DEEPSEEK_API_KEY`, `MISTRAL_API_KEY`, `VLLM_API_KEY`, `LITELLM_API_KEY`

## Run Modes

```bash
nocode                               # interactive TUI
nocode --status                      # system diagnostics
nocode --bridge-once "prompt"        # single-turn local
nocode --bridge-remote-once "prompt" # single-turn HTTP
nocode --ide-server                  # IDE server mode (JSON-RPC)
nocode --mcp-server                  # MCP server mode
nocode --process-agent-daemon        # background daemon (internal)
nocode --process-agent-host          # agent host (internal)
```

## Release Process

Tag-triggered via GitHub Actions. Bump version in workspace `Cargo.toml`, commit, then:

```bash
git tag v0.x.x
git push origin main --tags
```

The release workflow syncs version from tag to all Cargo.toml files, builds 6 platform binaries (linux/mac/windows × x64/arm64), creates GitHub Release, and publishes 6 npm platform packages + main `@telagod/nocode` package with provenance.
