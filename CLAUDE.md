# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is nocode

A terminal-native AI coding assistant built in Rust — a ground-up rewrite of a 507K-LOC TypeScript harness. Connects to Claude, OpenAI, Gemini, or any compatible endpoint via the Custom provider. Ships two interfaces: line-mode REPL and 4-pane TUI.

The reference architecture is **claw-code** (`/home/telagod/project/claw-code`), a 60K-LOC 9-crate Rust workspace with production-grade MCP lifecycle, worker boot state machine, recovery recipes, policy engine, and 40 tools. nocode is evolving toward that target.

## Build & Test Commands

```bash
cargo build                                     # debug build
cargo build --release                           # release build
cargo test                                      # all tests (~30)
cargo test -p nocode-core                       # core library only
cargo test -p nocode                            # CLI binary only
cargo test <test_name>                          # single test by name
cargo clippy --all-targets -- -D warnings       # lint (all + pedantic + nursery)
cargo fmt --check                               # format check
cargo fmt                                       # auto-format
```

## Workspace Layout

Two-crate Cargo workspace (edition 2024, clippy all+pedantic+nursery at workspace level, unsafe forbidden):

```
crates/nocode-core/   — library (~29K LOC), owns all core logic
crates/nocode/        — binary (~8K LOC), CLI/REPL/TUI shell
```

### Target Architecture (from claw-code, 9 crates)

nocode is consolidating toward this crate split as it matures:

| Crate | Purpose | nocode status |
|-------|---------|---------------|
| **runtime** | Session, conversation loop, config, permissions, MCP, hooks, recovery, worker boot, task/team/cron registries, LSP, policy engine | Partially in `nocode-core` |
| **tools** | 40 tool specs + execution, global registries (OnceLock singletons) | 10 tools in `nocode-core/tool_execution/` |
| **api** | Anthropic HTTP client, SSE streaming, OAuth, prompt caching, provider abstraction | In `nocode-core/provider.rs` + `provider_transport.rs` |
| **commands** | 15+ slash commands with manifest system | In `nocode/repl.rs` (~40 commands) |
| **plugins** | Plugin lifecycle, hook system (PreToolUse/PostToolUse/PostToolUseFailure) | Skeleton only |
| **rusty-claude-cli** | Main binary: REPL, one-shot, streaming, rendering | In `nocode/` crate |
| **mock-anthropic-service** | Deterministic mock for parity testing (12 scenarios) | Not yet |
| **compat-harness** | Tool/command manifest extraction for parity validation | Not yet |
| **telemetry** | Session tracing, usage telemetry, request profiling | `usage_tracker.rs` only |

## Architecture — Current Implementation

### Provider Layer (`nocode-core/src/provider.rs`, `provider_transport.rs`)

`ModelProvider` enum: `Mock | Claude | OpenAi | Gemini | Custom` (all Copy).
`ApiFormat` enum: `Claude | OpenAi | Gemini` — routes Custom providers to the correct request builder and response parser.

Bedrock/Vertex are not first-class; use Custom with `NOCODE_CUSTOM_API_FORMAT=claude`.

Request paths:
- Claude/Custom(claude): `/v1/messages`
- OpenAI: `/v1/responses`
- Gemini: `/v1beta/models/{model}:generateContent`

`CustomProviderConfig { name, base_url, format: ApiFormat }` lives outside the enum to keep `ModelProvider` Copy-compatible.

### Query Engine (`nocode-core/src/query_engine/`)

`QueryEngine` manages conversation lifecycle and tool schema generation. Submodules:
- `runtime.rs` — turn execution orchestration
- `state.rs` — query state management
- `persistence.rs` — session save/load
- `tests/` — submission and state resume tests

`QueryLoopRunner` (`query_loop.rs`) is a 9-state state machine: model call → tool call extraction → tool execution → result feedback → next turn. Budget tracking via `budget.rs` with diminishing returns; truncating compactor at 100K threshold, keeps 20 messages.

### Tool Call Flow (`nocode-core/src/query_engine/runtime.rs`)

`extract_tool_calls()` normalizes three provider formats into `ToolCallRequest { name, id, arguments }`:
- Claude: `tool_use` content blocks
- OpenAI: `function_call` in response output
- Gemini: `functionCall` in parts

Each becomes a `ToolCallInput` (via `with_arguments_map()`), dispatched through `execute_tool_call()`, results fed back via `QueryLoopAction::ResolveTool` then `FlushToolBatch`.

### Tool System (`nocode-core/src/tool_execution/`)

10 built-in tools: Read, Edit, Write, Bash, Glob, Grep, WebFetch, WebSearch, Agent, MCP. Each tool is a module under `tool_execution/`, with `executor.rs` for dispatch and `model.rs` for shared types.

Target: 40 tools (from claw-code), adding TodoWrite, NotebookEdit, Skill, ToolSearch, LSP, Task/Team/Cron dispatch, SendUserMessage, Config, PlanMode, StructuredOutput, PowerShell, Sleep, Brief.

### Tool Registry & Permissions (`nocode-core/src/tool_registry.rs`)

`ToolPermissionContext` with 9 preset Bash safety rules. Permission rule engine gates tool access.

Target permission model (from claw-code, 3 tiers):
- `ReadOnly` — no file writes, no destructive commands
- `WorkspaceWrite` — file writes within workspace boundary only
- `DangerFullAccess` — unrestricted

With `PermissionPolicy` (rules + prompting), `PermissionEnforcer` (gate enforcement), and hook-based overrides.

### MCP Client (`nocode-core/src/mcp_client.rs`)

JSON-RPC over stdio. Tool discovery and execution via `mcp:` prefix.

Target MCP architecture (from claw-code):
- 11-phase lifecycle: ConfigLoad → ServerRegistration → SpawnConnect → InitializeHandshake → ToolDiscovery → ResourceDiscovery → Ready → Invocation → ErrorSurfacing → Shutdown → Cleanup
- `McpToolRegistry` — bridge between tools and MCP servers
- Degraded-mode reporting (partial server failures remain usable)
- Error classification per phase with `McpErrorSurface`
- Transport abstraction: Stdio, SSE, HTTP, WebSocket

### Session & Persistence (`nocode-core/src/session_persistence.rs`)

JSONL format in `.nocode/` directory: sessions, history, file-history, tasks.

Target session model (from claw-code):
- `Session` with versioned state, compaction metadata, fork provenance
- Append-only JSONL with atomic writes, rotation after 256KB (3 rotated files max)
- `SessionCompaction` — summarize old messages, preserve last 4 verbatim, trigger at 10K estimated tokens
- `SessionFork` — parent session ID + branch name for branch-aware tracking

### Task Runtime (`nocode-core/src/task_runtime.rs`)

Four task types: shell, agent, dream, process daemon. Supervisor manages lifecycle.

Target task system (from claw-code):
- `TaskRegistry` — in-memory lifecycle with `TaskPacket` (objective, scope, repo, branch_policy, acceptance_tests, commit_policy, escalation_policy)
- `TeamRegistry` — multi-agent team coordination
- `CronRegistry` — scheduled task management
- Global registries via `OnceLock<Arc<Mutex<T>>>` singletons

### CLI Shell (`nocode/src/`)

- `main.rs` — entry point, provider detection, bootstrap config, 7 execution modes
- `repl.rs` — REPL session, ~40 slash commands, task management
- `tui.rs` — 4-pane TUI (crossterm), async streaming with adaptive polling (16ms streaming / 120ms idle), overlay system (help/inspector/permission)
- `claudemd.rs` — CLAUDE.md auto-discovery (user/project/rules/local)
- `task_panel.rs` — task filtering and rendering

### Other Core Modules

- `assistant_turn.rs` — assistant turn representation
- `message.rs` — conversation message types
- `model_response.rs` — model response parsing
- `budget_state.rs` — budget state tracking
- `stop_hook.rs` — stop condition hooks
- `transcript.rs` — conversation transcript
- `usage_tracker.rs` — token usage and cost tracking
- `file_history.rs` / `history_store.rs` — file and command history
- `bridge_runtime.rs` — local/remote bridge, permission callbacks

## Architecture — Target Patterns (from claw-code)

These patterns are the reference implementation in claw-code. Adopt them as nocode matures.

### Worker Boot State Machine

States: `Spawning → TrustRequired → ReadyForPrompt → PromptAccepted → Running → Blocked/Finished/Failed`

- Trust gate detection and resolution (auto-trust for known repos, approval for unknown)
- Prompt misdelivery detection and recovery
- Structured `WorkerEvent` emission for observability

### Recovery Recipes

7 known failure scenarios with one automatic recovery attempt before escalation:

| Scenario | Recovery Step | Escalation |
|----------|--------------|------------|
| TrustPromptUnresolved | AcceptTrustPrompt | AlertHuman |
| PromptMisdelivery | RedirectPromptToAgent | AlertHuman |
| StaleBranch | RebaseBranch + CleanBuild | AlertHuman |
| CompileRedCrossCrate | CleanBuild | AlertHuman |
| McpHandshakeFailure | RetryMcpHandshake (5s) | Abort |
| PartialPluginStartup | RestartPlugin + RetryMcp (3s) | LogAndContinue |
| ProviderFailure | RestartWorker | AlertHuman |

### Policy Engine

Lane-based automation with composable conditions and chainable actions:
- Conditions: `GreenAt { level }`, `StaleBranch`, `StartupBlocked`, `LaneCompleted`, `ReviewPassed`, `And(vec)`, `Or(vec)`
- Actions: `MergeToDev`, `MergeForward`, `RecoverOnce`, `Escalate`, `CloseoutLane`, `Notify`, `Block`, `Chain(vec)`
- Green levels: TargetedTests → Package → Workspace → MergeReady

### Hook System

Plugin hooks at three lifecycle points:
- `PreToolUse` — gate or modify tool invocation
- `PostToolUse` — react to tool results
- `PostToolUseFailure` — handle tool failures

`HookRunner` executes hook commands with abort signals and permission overrides.

### Bash Validation (6 submodules in claw-code)

- readOnlyValidation — enforce read-only mode
- destructiveCommandWarning — flag dangerous commands
- modeValidation — permission mode checks
- sedValidation — sed command safety
- pathValidation — workspace boundary enforcement
- commandSemantics — semantic command analysis

### Sandbox & Isolation

- `FilesystemIsolationMode`: Off, WorkspaceOnly (default), AllowList
- Linux namespace/network isolation with capability detection
- Container detection (dockerenv, containerenv, cgroup markers)
- Capability-aware: degrades gracefully when unshare unavailable

### Config Hierarchy (3-tier, highest wins)

1. User — `~/.claude/settings.json`
2. Project — `.claude/settings.json` in repo root
3. Local — `.claude.local/settings.json` for machine-specific overrides

### System Prompt Assembly

Layered and deterministic with dynamic boundary marker:
1. Static scaffolding (intro, output style, system, task, actions sections)
2. `__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__`
3. Runtime context (cwd, date, platform, model, git status/diff, instruction files)
4. Config sections (hooks, plugins, MCP servers)

Instruction file discovery walks up directory tree, looks for CLAUDE.md variants, deduplicates, limits to 4K chars/file, 12K total.

### Mock Parity Harness

Deterministic Anthropic-compatible mock service for end-to-end testing:
- 12 scripted scenarios (streaming_text, read_file_roundtrip, write_file_allowed/denied, bash_permission_prompt, multi_tool_turn, plugin_tool, auto_compact, token_cost)
- Spawns TCP listener on dynamic port, captures requests for assertion
- No external API dependency, fully reproducible

## Key Conventions

- Workspace edition 2024. Clippy all+pedantic+nursery. Unsafe forbidden.
- `ModelProvider` and `ApiFormat` are Copy enums. Custom config stored separately.
- Canonical result term: display `response-result`, Rust/wire `response_result`, task panel `result`. Legacy `structured_output` only in provider JSON schema request name and bridge backward-compat decode alias.
- Trait-based extensibility: `ApiClient`, `ToolExecutor`, `PermissionPrompter`, `TelemetrySink` for pluggable behavior.
- Global registries use `OnceLock<Arc<Mutex<T>>>` singleton pattern for shared state across tool invocations.
- State machines over inferred state: worker lifecycle, MCP lifecycle, plugin lifecycle all use explicit enum states.
- Events over scraped prose: structured typed events (`RecoveryEvent`, `LaneEvent`, `WorkerEvent`) for observability.
- One recovery attempt before escalation — never silently retry indefinitely.

## Environment Variables

Provider auto-detected from API key presence. Override with:
- `NOCODE_MODEL_PROVIDER` — force provider (`claude`, `openai`, `gemini`, `custom`, `mock`)
- `NOCODE_MODEL` — override model name
- `NOCODE_CUSTOM_BASE_URL` / `NOCODE_CUSTOM_API_FORMAT` — Custom provider config
- `NOCODE_SYSTEM_PROMPT` — override system prompt
- `NOCODE_MODEL_REASONING_EFFORT` — `low`, `medium`, `high`
- `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` / `GEMINI_API_KEY` — provider API keys
- `ANTHROPIC_BASE_URL` / `OPENAI_BASE_URL` — per-provider base URL override

## Run Modes

```bash
nocode --repl                        # interactive REPL
nocode --tui                         # 4-pane terminal UI
nocode --status                      # system diagnostics
nocode --bridge-once "prompt"        # single-turn local
nocode --bridge-remote-once "prompt" # single-turn HTTP
nocode --process-agent-daemon        # background daemon (internal)
nocode --process-agent-host          # agent host (internal)
```