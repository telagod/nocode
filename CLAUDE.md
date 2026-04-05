# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is nocode

A terminal-native AI coding assistant built in Rust — 38K LOC, 51 modules, 25 tools, 555 tests. Connects to Claude, OpenAI, Gemini, or any compatible endpoint. Two interfaces: line-mode REPL and 4-pane TUI with full Markdown rendering, syntect syntax highlighting, and RGB color support.

## Build & Test Commands

```bash
cargo build                                     # debug build
cargo build --release                           # release build
cargo test                                      # all tests (~555)
cargo test -p nocode-core                       # core library only
cargo test -p nocode                            # CLI binary only
cargo test <test_name>                          # single test by name
cargo clippy --all-targets -- -D warnings       # lint (all + pedantic + nursery)
cargo fmt --check                               # format check
cargo fmt                                       # auto-format
```

## Workspace Layout

Two-crate Cargo workspace (edition 2024, clippy all+pedantic+nursery, unsafe forbidden):

```
crates/nocode-core/   — library (~30K LOC, 51 modules), all core logic
crates/nocode/        — binary (~8K LOC), CLI/REPL/TUI shell
```

Dependencies: serde, serde_json, reqwest, jsonschema, rusqlite (bundled), chrono, pulldown-cmark, syntect, crossterm.

## Architecture — 51 Modules

### Provider Layer
- `provider.rs` — ModelProvider enum (Mock|Claude|OpenAi|Gemini|Custom), ApiFormat routing
- `provider_transport.rs` — HTTP client, SSE parsing, retry/backoff, per-provider auth

### Query Engine
- `query_engine.rs` + `query_engine/` — conversation lifecycle, tool schema gen, 9-state loop
- `query_loop.rs` — QueryLoopRunner state machine, budget, stop hooks
- `query_deps.rs` — DI with trait objects (CallModel, Compactor, ToolRunner), RichCompactor default
- `query_config.rs` — query configuration with tool definitions

### Tool System (25 tools)

Core: Read, Edit, Write, Bash, Glob, Grep, WebFetch, WebSearch, Agent
Task: TaskGet, TaskList, TaskUpdate, TaskStop, TaskOutput
Team: TeamCreate, TeamDelete
Cron: CronCreate, CronDelete, CronList
Discovery: ToolSearch, Lsp
Memory: MemorySave, MemoryList, MemorySearch, MemoryDelete

Modules:
- `tool_execution/executor.rs` — DefaultToolExecutor dispatch with hook/sandbox/validation integration
- `tool_execution/model.rs` — ToolCallInput, ToolCallResult, ToolExecutionTrace
- `tool_execution/task_tools.rs` — 5 task tools wired to global TaskCoordinator
- `tool_execution/team_tools.rs` — TeamCreate/Delete
- `tool_execution/cron_tools.rs` — CronRegistry (OnceLock singleton) + 3 tools
- `tool_execution/mcp_bridge.rs` — McpToolBridge, mcp:server:tool prefix dispatch
- `tool_execution/lsp_tools.rs` — LSP tool with 6 actions (diagnostics/hover/definition/references/completion/symbols)
- `tool_execution/tool_search.rs` — DeferredToolRegistry + fuzzy search
- `tool_execution/memory_tools.rs` — 4 memory tools wired to real MemoryStore
- `tool_registry.rs` — PermissionMode (ReadOnly/WorkspaceWrite/DangerFullAccess), ToolDefinition with required_permission
- `tool_validation.rs` — JSON Schema validation for all tool inputs
- `bash_validation.rs` — 6 validation submodules (read_only, destructive, mode, sed, path, semantics)
- `file_safety.rs` — symlink escape prevention, binary detection, 10MB size limit

### Storage Layer
- `sql_store.rs` — rusqlite-backed storage with date-based volume partitioning (~/.nocode/data/nocode_YYYY-MM-DD.db). 5 tables: sessions, messages, memories, command_history, telemetry_events. Global singleton.
- `session_persistence.rs` — JSONL session/transcript/history persistence (legacy)
- `persistence_backend.rs` — PersistenceBackend trait, Noop/Recording implementations
- `history_store.rs` — command history
- `file_history.rs` — file change tracking

### Memory System
- `memory_store.rs` — MemoryEntry with frontmatter (name/description/type/content), file-system CRUD, MEMORY.md index management
- `memory_signals.rs` — 8 signal types (UserCorrection, UserPreference, UserRole, ProjectContext, PositiveFeedback, ExternalReference, ExplicitRemember, ExplicitForget), pattern-based detection with confidence scores
- `summary_compression.rs` — priority-based line dedup/truncation (1200 chars, 24 lines budget), core detail line preservation

### Session & Compaction
- `session_compaction.rs` — RichCompactor with structured summaries (message counts, tool names, recent requests, pending work, key files), integrated with summary_compression
- `budget.rs` + `budget_state.rs` — token budget tracking with diminishing returns

### Runtime Infrastructure
- `worker_boot.rs` — Worker state machine (Spawning→TrustRequired→ReadyForPrompt→Running→Finished/Failed), WorkerEvent audit trail, WorkerRegistry singleton
- `recovery.rs` — 7 failure scenarios, RecoveryRecipe (steps + max_attempts + escalation), one-attempt-before-escalation
- `policy_engine.rs` — composable conditions (GreenAt/StaleBranch/And/Or), chainable actions (MergeToDev/Escalate/Chain), priority-sorted rule evaluation
- `task_runtime.rs` — TaskCoordinator (shell/agent/dream/daemon), global singleton
- `hook_runner.rs` — HookRunner executing PreToolUse/PostToolUse/PostToolUseFailure commands, global singleton, deny-stops-chain
- `plugin_system.rs` — PluginState lifecycle (Unconfigured→Validated→Healthy/Degraded/Failed), PluginRegistry, hook dispatch
- `sandbox.rs` — FilesystemIsolationMode, container detection, namespace probing, capability-aware degradation

### Configuration & Auth
- `config_loader.rs` — 3-tier config hierarchy (User/Project/Local), RuntimeConfig with MCP/hooks/sandbox sections
- `prompt_assembly.rs` — SystemPromptBuilder with dynamic boundary, instruction file discovery (CLAUDE.md variants), dedup by FNV hash, truncation budgets
- `oauth.rs` — PKCE code pair generation, token persistence/refresh, authorization URL builder
- `command_registry.rs` — SlashCommandSpec with aliases/summary/argument_hint, 20+ pre-registered commands

### Observability
- `telemetry.rs` — 9 TelemetryEvent variants, TelemetryRecord with sequence, SessionTracer, global telemetry log
- `model_pricing.rs` — Haiku/Sonnet/Opus pricing tables, CostEstimate, format_usd
- `usage_tracker.rs` — UsageSnapshot with token counts and totals
- `prompt_cache.rs` — FNV-1a fingerprint, TTL-based cache (256 entries, 30s default), hit/miss/eviction stats

### Branch & Workflow
- `stale_branch.rs` — BranchFreshness (Fresh/Stale/Diverged), 4 policies (WarnOnly/Block/AutoRebase/AutoMergeForward)
- `lane_events.rs` — 16 LaneEventName variants, 11 status variants, 11 failure classes
- `green_contract.rs` — GreenLevel hierarchy (TargetedTests→Package→Workspace→MergeReady)

### MCP & LSP
- `mcp_client.rs` — JSON-RPC over stdio, initialize/list_tools/call_tool
- `mcp_manager.rs` — McpManager with per-server lifecycle, tool discovery, global singleton
- `lsp_client.rs` — LspRegistry with file-system based implementation (grep-based definition/references, bracket diagnostics, keyword completion)
- `global_registry.rs` — GlobalToolRegistry (OnceLock singleton), ToolSource (Base/Plugin/Mcp/Runtime), fuzzy search

### Testing
- `mock_service.rs` — MockAnthropicService with 12 deterministic scenarios, CapturedRequest recording, ParityTestRunner
- `tests/integration.rs` — end-to-end parity scenarios + memory roundtrip

### TUI (nocode crate, 3,412 LOC)
- `tui.rs` — 4-pane fullscreen (Transcript/TaskList/TaskDetail/Events), StyledContent with RGB rendering, overlay system, adaptive polling
- `markdown_render.rs` — pulldown-cmark + syntect, headings/code/lists/quotes/links/rules
- `markdown_stream.rs` — MarkdownStreamState with fence-aware boundary detection
- `tool_render.rs` — per-tool box-drawing formatting (Bash/Read/Write/Edit/Glob/Grep)
- `tool_truncate.rs` — configurable truncation (80 lines/6K for read, 60 lines/4K for tools)
- `spinner.rs` — 10-frame braille animation
- `status_hud.rs` — token/cost/elapsed/model/session HUD strip

### Other
- `message.rs` — QueryMessage types
- `model_response.rs` — model response parsing
- `assistant_turn.rs` — assistant turn representation
- `transcript.rs` — conversation transcript
- `stop_hook.rs` — stop condition hooks
- `bridge_runtime.rs` — local/remote bridge
- `roadmap.rs` — roadmap tracking

## Key Conventions

- Workspace edition 2024. Clippy all+pedantic+nursery. Unsafe forbidden.
- `ModelProvider` and `ApiFormat` are Copy enums. Custom config stored separately.
- Global registries use `OnceLock<Arc<Mutex<T>>>` singleton pattern.
- State machines over inferred state: worker, MCP, plugin lifecycles use explicit enum states.
- Events over scraped prose: structured typed events for observability.
- One recovery attempt before escalation — never silently retry indefinitely.
- SQL storage with date-based volume partitioning for clean data management.
- Memory entries use Markdown with YAML frontmatter (name/description/type).

## Environment Variables

- `NOCODE_MODEL_PROVIDER` — force provider (`claude`, `openai`, `gemini`, `custom`, `mock`)
- `NOCODE_MODEL` — override model name
- `NOCODE_CUSTOM_BASE_URL` / `NOCODE_CUSTOM_API_FORMAT` — Custom provider config (`claude`=Messages API, `openai`=Chat Completions/Responses, `gemini`=generateContent)
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