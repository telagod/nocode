# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is nocode

A terminal-native AI coding assistant built in Rust. Connects to Claude, OpenAI, Gemini, or any compatible endpoint. Two interfaces: line-mode REPL and 4-pane TUI with Markdown rendering, syntect syntax highlighting, RGB color support, vim mode, dark/light themes, and thinking block display.

## Build & Test Commands

```bash
cargo build                                     # debug build
cargo build --release                           # release build
cargo test                                      # all tests (~909)
cargo test -p nocode-core                       # core library only
cargo test -p nocode                            # CLI binary only
cargo test <test_name>                          # single test by name
cargo clippy --all-targets -- -D warnings       # lint (all + pedantic + nursery)
cargo fmt --check                               # format check
cargo fmt                                       # auto-format
```

CI runs fmt → clippy → test on every push/PR to main (`.github/workflows/ci.yml`). Release is tag-triggered (`v*`) via `.github/workflows/release.yml` — builds 6 platform binaries, creates GitHub Release, publishes npm packages.

## Workspace Layout

Two-crate Cargo workspace (edition 2024, clippy all+pedantic+nursery, unsafe forbidden):

```
crates/nocode-core/   — library (55 modules), all core logic
crates/nocode/        — binary (14 modules), CLI/REPL/TUI shell
```

Key dependencies: serde, serde_json, reqwest, jsonschema, rusqlite (bundled), chrono, pulldown-cmark, syntect, crossterm.

## Architecture

### Core Data Flow

```
User Input → main.rs (mode dispatch) → QueryEngine (9-state loop)
  → ModelProvider (Claude/OpenAI/Gemini/Custom/Mock) → SSE stream
  → ModelResponse → tool calls? → ToolExecutor → permission check
  → hook runner → sandbox → execute → result back to QueryEngine loop
```

The query engine (`query_engine.rs`) drives an agentic conversation loop. Each iteration: send messages to the model via `provider_transport.rs` (SSE streaming), parse the response, execute any tool calls through `DefaultToolExecutor`, feed tool results back to the model, and loop until the model stops requesting tools or budget is exhausted.

### Dependency Injection

`query_deps.rs` defines trait objects (`CallModel`, `Compactor`, `ToolRunner`, `StopHookRunner`, `Clock`, `IdGen`) wired together via `QueryDepsBuilder`. Tests swap in mocks; production uses `RichCompactor` and `DefaultToolExecutor`. This is the seam for all testability.

### Tool Execution Pipeline

Every tool call flows through: JSON Schema validation (`tool_validation.rs`) → permission check against `PermissionMode` (ReadOnly/WorkspaceWrite/DangerFullAccess) → `PermissionPrompter` (AutoApprove/AutoDeny/Interactive) → hook runner (PreToolUse can deny) → sandbox enforcement → actual execution → PostToolUse hooks. The `tool_execution/executor.rs` orchestrates this chain.

Bash commands get extra validation through 6 submodules in `bash_validation.rs` (read_only, destructive, mode, sed, path, semantics). File operations go through `file_safety.rs` (symlink escape prevention, binary detection, 10MB limit).

### Tool Categories (25 tools)

- Core: Read, Edit, Write, Bash, Glob, Grep, WebFetch, WebSearch, Agent
- Task: TaskGet, TaskList, TaskUpdate, TaskStop, TaskOutput
- Team: TeamCreate, TeamDelete
- Cron: CronCreate, CronDelete, CronList
- Discovery: ToolSearch, Lsp
- Memory: MemorySave, MemoryList, MemorySearch, MemoryDelete

MCP tools are bridged via `mcp_bridge.rs` with `mcp:server:tool` prefix dispatch. All tools (base + plugin + MCP + runtime) are unified in `GlobalToolRegistry`.

### Provider Abstraction

`ModelProvider` enum (Mock|Claude|OpenAi|Gemini|Custom) with `ApiFormat` routing. Each provider maps to a different wire format (Messages API, Chat Completions/Responses, generateContent). `provider_transport.rs` handles HTTP client, SSE parsing, retry/backoff, and per-provider auth headers.

### Configuration (3-tier hierarchy)

```
~/.nocode/settings.json              → User tier (global)
{cwd}/.nocode/settings.json          → Project tier
{cwd}/.nocode/settings.local.json    → Local tier (gitignored)
```

Later tiers override earlier. Scalars: last-wins. Maps: merged key-by-key. Vecs: replaced wholesale. Environment variables override on top. `RuntimeConfig` holds model, permission_mode, system_prompt, mcp_servers, hooks, sandbox.

### State Machines

The codebase uses explicit enum state machines rather than inferred state:
- Worker lifecycle: Spawning → TrustRequired → ReadyForPrompt → Running → Finished/Failed
- MCP lifecycle: 11 phases from Registered → Shutdown (with Degraded/Reconnecting branches)
- Plugin lifecycle: Unconfigured → Validated → Healthy/Degraded/Failed

### Global Singletons

Several subsystems use `OnceLock<Arc<Mutex<T>>>` singletons: `TaskCoordinator`, `WorkerRegistry`, `McpManager`, `HookRunner`, `CronRegistry`, `PluginRegistry`, `SqlStore`, `GlobalToolRegistry`, `LspRegistry`. Access via `global_*()` functions.

### Storage

- Primary: `sql_store.rs` — rusqlite with date-based volume partitioning (`~/.nocode/data/nocode_YYYY-MM-DD.db`). Tables: sessions, messages, memories, command_history, telemetry_events.
- Memory: `memory_store.rs` — file-system CRUD with Markdown+YAML frontmatter, MEMORY.md index.
- Session: `session_persistence.rs` — JSONL transcript/history/task persistence with auto-persist on submission complete. `persist_transcript()` / `load_transcript()` / `list_sessions()` for session resume.

### Session Management

`session_compaction.rs` (RichCompactor) produces structured summaries when context grows too large. `session_control.rs` supports fork/branch/resume/suspend/complete with parent_id tracking. Token budget tracked in `budget.rs` with diminishing returns logic.

### Trust & Permission System

`worker_boot.rs` defines `TrustResolver` with chainable policies (AllowAll, PromptRequired, RuleBased). `permission_enforcer.rs` enforces tool-level permissions. The TUI has a dedicated permission overlay (`tui_permission.rs`) for interactive approve/deny.

### Recovery

`recovery.rs` maps 7 failure scenarios to `RecoveryRecipe` (steps + max_attempts + escalation). Hard rule: one attempt before escalation, never silently retry indefinitely.

### Testing Strategy

- Unit tests: inline `#[cfg(test)]` modules throughout
- Integration tests: `crates/nocode-core/tests/` — 4 files covering mock service parity, tool execution roundtrips, trust/permission/MCP health, roadmap
- `MockAnthropicService` provides 12 deterministic scenarios with `CapturedRequest` recording
- Query engine has dedicated test suite in `query_engine/tests/` (state resume, submission, support)

### TUI Features

The TUI (`tui_app.rs`, `tui_widgets.rs`) provides a rich terminal interface:

- **Vim mode**: Normal/Insert mode with `h/j/k/l/w/b/e/x/dd/C` motions, Esc toggle, `[NORMAL]` indicator
- **Theme switching**: Dark/light themes via `Ctrl-T`, `RwLock`-based runtime switching (`tui_theme.rs`)
- **Thinking blocks**: Collapsible `∴` blocks showing model reasoning, `Ctrl-O` expand/collapse
- **Paste support**: Bracketed paste mode for multi-line paste with newline preservation
- **Streaming**: Real-time incremental text rendering, tool start/done/result events
- **Tool display**: Claude Code visual language (`❯/⎿/●/∴/•/✖/⚠`), collapsible tool output with `Tab`
- **Inline permissions**: `⚠ y/n/a` prompt for tool approval
- **Input**: Multi-line with `Shift-Enter`, input history `Ctrl-P/N`, `Ctrl-U` clear
- **Slash commands**: `/help /clear /status /model /sessions /resume /mcp /agents /quit`
- **Session resume**: `/sessions` lists saved sessions, `/resume <id>` restores transcript
- **MCP status**: `/mcp` shows connected MCP servers and discovered tools
- **Agent status**: `/agents` shows background agent workers and their state

### Model Stream Events

`ModelStreamEvent` enum drives real-time TUI updates during model calls:

- `Start` / `Delta` / `Complete` — standard streaming text
- `ToolUseStart` / `ToolUseDone` — tool call lifecycle (name, args summary)
- `ToolResult` — tool execution result pushed in real-time during agentic loop
- `ThinkingDelta` — reasoning/thinking content from extended thinking models
- `StreamError` — retryable/non-retryable error surface

### Agentic Tool Loop

The query engine (`runtime.rs`) implements a multi-turn agentic loop:

```
model call → tool requests? → execute tools → push ToolResult to stream
  → flush results into conversation → loop back to model call
  → repeat until model stops requesting tools or max_turns reached
```

Agent tool calls spawn on background threads via `WorkerRegistry` for parallel execution.

### MCP Integration

MCP servers configured in `settings.json` under `mcp_servers` are auto-connected at TUI startup. `McpManager` handles lifecycle (11 phases), health checks, reconnection. `McpClient` communicates via JSON-RPC over stdio. Tools discovered via `tools/list` are routable through `mcp_bridge.rs`.

## Key Conventions

- `ModelProvider` and `ApiFormat` are Copy enums. Custom config stored separately.
- Global singletons: `OnceLock<Arc<Mutex<T>>>`, accessed via `global_*()` functions.
- Explicit enum state machines over inferred state for all lifecycles.
- Structured typed events for observability — never scrape prose.
- One recovery attempt before escalation — never silently retry indefinitely.
- Memory entries: Markdown files with YAML frontmatter (name/description/type), indexed in MEMORY.md.
- System prompt assembled dynamically by `prompt_assembly.rs` — discovers CLAUDE.md variants, deduplicates by FNV hash, applies truncation budgets.
- Slash commands registered in `command_registry.rs` with aliases/summary/argument_hint (20+ commands).

## Environment Variables

- `NOCODE_MODEL_PROVIDER` — force provider (`anthropic`, `openai`, `google`, `custom`, `mock`)
- `NOCODE_MODEL` — override model name
- `NOCODE_CUSTOM_BASE_URL` / `NOCODE_CUSTOM_API_FORMAT` — Custom provider config
- `NOCODE_SYSTEM_PROMPT` — override system prompt
- `NOCODE_MODEL_REASONING_EFFORT` — `low`, `medium`, `high`
- `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` / `GEMINI_API_KEY` — provider API keys
- `ANTHROPIC_BASE_URL` / `OPENAI_BASE_URL` — per-provider base URL override
- `NOCODE_BRIDGE_BASE_URL` / `NOCODE_BRIDGE_AUTH_TOKEN` — remote bridge config

## Run Modes

```bash
nocode --repl                        # interactive REPL
nocode --tui                         # 4-pane terminal UI
nocode --status                      # system diagnostics
nocode --bridge-once "prompt"        # single-turn local
nocode --bridge-remote-once "prompt" # single-turn HTTP
nocode --ide-server                  # IDE server mode
nocode --mcp-server                  # MCP server mode
nocode --process-agent-daemon        # background daemon (internal)
nocode --process-agent-host          # agent host (internal)
```