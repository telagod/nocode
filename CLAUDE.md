# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is nocode

A terminal-native AI coding agent in Rust positioned as **a reference implementation of harness engineering bionics for fractal code agents**. The value lives in the harness — minimal tools, three explainable gates, skills as first-class prompt material, sub-agents that share the parent's loop+registry+gates by construction.

For the philosophy, read `docs/00_vision.md`. For the alignment trail (what changed and why), read `docs/01_realign.md`. The full index is in `docs/README.md`.

Connects to Claude, OpenAI, Gemini, or any compatible endpoint via Custom provider. The interface is a TUI plus interactive login/bootstrap flows.

## Build & Test Commands

```bash
cargo build                                     # debug build (default: full features)
cargo build --release                           # release build
cargo build --no-default-features -p nocode-core --features minimal  # minimal core (no MCP/plugins/telemetry/oauth)
cargo test                                      # all tests (~786 across 6 test binaries)
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
crates/nocode-core/   — library (~90 modules, ~28K LOC), all core logic
crates/nocode/        — binary, CLI/TUI shell (~22 source files, ~14.5K LOC)
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

**Provider resolution** (in `main.rs:resolve_provider`, see also `docs/10_provider_config.md`): `--provider <name>` flag → `NOCODE_PROVIDER` env → active profile's `provider` → `settings.default_provider` → `settings.model_provider` interpreted as a builtin alias (`claude` / `openai` / `gemini`). The chosen name is then looked up in `[providers.<name>]` (or a builtin alias) for `base_url` / `wire_api` / `api_key_env`. The legacy `custom_*` scheme is rejected at load time.

**Default models**: Claude → `claude-sonnet-4-20250514`, OpenAI → `gpt-4.1`, Gemini → `gemini-2.5-pro`.

### Query Engine

`query/loop.rs` drives the agentic conversation loop. Each iteration: send messages to the model via `provider/transport.rs` (SSE streaming), parse the response, execute any tool calls through `ToolExecutor`, feed tool results back, and loop until the model stops requesting tools or budget is exhausted.

### Dependency Injection

`query/deps.rs` defines trait objects (`CallModel`, `Compactor`, `ToolRunner`, `StopHookRunner`, `Clock`, `IdGen`) wired via `QueryDepsBuilder`. Tests swap in mocks; production uses `RichCompactor` and `DefaultToolExecutor`. This is the seam for all testability.

### Tool Execution Pipeline

Every tool call flows through three explainable gates: **Schema** (`tool/tool_validation.rs`) → **Policy** (`tool/policy.rs`, the unified `PolicyEngine` covering trust + permission-mode + risk classifier + sandbox + plan-mode, returning a `GateDecision { gate, reason }`) → **Hooks** (`tool/hook_runner.rs` PreToolUse, can deny). PostToolUse hooks run after execution. Every refusal in the TUI carries a `Denied [gate: reason]` why-trail — there are no silent gates.

`tool/executor.rs` orchestrates Lookup → Schema → Policy → Hooks → Bash-classifier → Execute → snapshot/file-history → PostHooks → render `ContentBlock::ToolResult`. Bash command syntax-level checks live in `tool/bash_validation.rs` (read-only / destructive / mode / sed / path / semantics) and surface through the bash classifier inside the Policy gate. File operations also go through `tool/file_safety.rs` (symlink escape, binary detection, 10MB limit).

### Skill System (first-class)

Skills are markdown files under `.nocode/skills/` and `.claude/skills/` (project + user-global). The `SkillRegistry` (`crates/nocode-core/src/skill/registry.rs`) discovers them at session start, parses optional YAML frontmatter (`name` / `description` / `triggers`), and renders an *index block* (name + description only) that gets injected into the system prompt alongside `CLAUDE.md` and `AGENTS.md` via `prompt/assembly.rs`. The model sees what's available before it decides to call the `Skill` tool — only the chosen skill's body is materialized. The index is adaptively trimmed to fit `TruncationBudget::max_skill_index_chars` (default 4 KB), keeping the densest entries first.

### Default Tool Surface (11 atomic tools)

`ToolRegistry::with_defaults(cwd)` registers the canonical 11: `FileRead, FileWrite, FileEdit, Glob, Grep, Bash, WebFetch, WebSearch, Agent, AskUserQuestion, Skill`. The set is asserted by `tests/roadmap.rs::tool_registry_has_canonical_core_set` so any accidental shrink/sprawl forces a deliberate decision. Optional tools (`Memory`, `TodoWrite`, `Task`, `Mcp`, `Cron*`, `Team*`, `EnterPlanMode`/`ExitPlanMode`, `EnterWorktree`/`ExitWorktree`, `Config`, `NotebookEdit`, `ToolSearch`, `Lsp`, `SendMessage`) live in their own modules and are registered explicitly when the host wants them — most are session-state primitives the TUI surfaces as slash commands.

### Fractal Sub-agents

`AgentTool` spawns a sub-agent that runs `run_worker_thread` — a recursive instance of the same harness: same `Provider` trait, same `ToolRegistry::with_defaults` (so the 11 core tools and skill index propagate), same `assemble_system_prompt`. As of REALIGN the sub-agent **inherits the parent's permission_mode** when the `mode` argument is provided (`acceptEdits`/`bypassPermissions`/`dontAsk` → `Auto`, `plan` → `ReadOnly`, `default` → inherit). Without this, every spawn silently jumped to `Auto` regardless of how cautious the parent was configured.

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

## TUI Features

- **Working indicator**: spinner with elapsed time + token counts (input/output), stall warning at 30s idle
- **Stream cancel**: Escape key sets `Arc<AtomicBool>` cancel token, checked at top of each agentic loop turn
- **Tool output folding**: tool results default collapsed (3-line preview + "…+N lines"), Ctrl+O to expand/collapse
- **Thinking blocks**: collapsed by default on resume, expandable with Ctrl+O
- **Message gaps**: visual separation between User/Assistant/Tool message blocks
- **Session resume**: `--resume <id>` or `-c` restores full message history with proper tool/thinking rendering
- **Timeouts**: effectively disabled (86400s) to support long-running agent tasks (10h+); max_turns=200
- **Brand logo**: unified 6-row ASCII art with lavender→teal gradient, shared between TUI banner and login flow

## Environment Variables

- `NOCODE_PROVIDER` — name of a provider defined in `[providers.<name>]` (or builtin alias `claude` / `openai` / `gemini`)
- `NOCODE_PROFILE` — name of a profile defined in `[profiles.<name>]`
- `NOCODE_MODEL` — override model name
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
nocode --resume [session_id]         # resume a previous session
nocode -c                            # shorthand for --resume (continue last)
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
