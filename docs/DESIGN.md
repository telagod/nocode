# nocode DESIGN

> Last updated: 2026-04-08 (v2 — Claude Code strict alignment)

## Design Goals

- Rust reimplementation of Claude Code's execution kernel.
- 21 tools strict parity with Claude Code sdk-tools.d.ts.
- Free, open, provider-agnostic AI coding assistant — no vendor lock-in.
- Compilable, testable, observable from day one.

## Non-Goals

- Pixel-perfect parity with Claude Code's Ink/React UI.
- Embedded proxy / `/free` route.
- Plugin marketplace, voice (deferred).

## Current Architecture

```
nocode (CLI/TUI shell — crates/nocode, 18 modules)
  main.rs           — entry point, 9 run modes, provider detection
  repl.rs           — REPL session, 23 slash commands, session persistence
  tui_app.rs        — 4-pane TUI, async streaming, ❯/⎿/∴/✖ visual language
  tui_theme.rs      — dark/light theme system
  tui_input.rs      — multi-line input, vim mode
  tui_permission.rs — permission overlay (y/n/a)
  tui_widgets.rs    — WelcomeBanner, StatusBar, ChatMessage, InputBox
  command_registry.rs — 23 slash commands with aliases
  tool_render.rs    — Claude Code visual tool display
  markdown_render.rs — Markdown rendering with syntect

nocode-core (library — crates/nocode-core, 70+ modules)
  provider/         — Claude (2024-06-01) / OpenAI / Gemini / Custom / Mock
  provider/transport.rs — HTTP client, SSE, retry/backoff, status codes
  query/loop.rs     — agentic loop (stop_reason driven, auto-compaction, recovery)
  query/budget.rs   — token budget, diminishing returns
  query/events.rs   — ModelStreamEvent for TUI/REPL
  tool/             — 21 tools (Claude Code parity) + extra modules
  tool/executor.rs  — 9-stage pipeline (validate→trust→hooks→permission→sandbox→execute)
  tool/bash_validation.rs — 6 submodules (read_only, destructive, mode, sed, path, semantics)
  session/          — JSONL persistence, compaction, fork/branch/resume
  config/           — 3-tier settings merge (user→project→local) + env overrides
  prompt/           — system prompt assembly, CLAUDE.md discovery, FNV dedup
  agent/            — WorkerRegistry, TaskCoordinator, background execution
  mcp/              — McpClient (JSON-RPC stdio), McpManager (11-phase lifecycle)
  recovery.rs       — 7 failure scenarios → RecoveryRecipe
  storage/          — rusqlite + memory CRUD
```

## Provider Architecture (post v0.2)

```
ModelProvider enum (Copy):  Mock | Claude | OpenAi | Gemini | Custom
ApiFormat enum (Copy):      Claude | OpenAi | Gemini
```

Design decisions:
- **Bedrock/Vertex removed as first-class providers.** They are Claude Messages format + different endpoint/auth. Use `Custom` with `NOCODE_CUSTOM_BASE_URL` and `NOCODE_CUSTOM_API_FORMAT=claude`.
- **OpenAI Chat + Responses merged** into single `OpenAi` variant (defaults to Responses format).
- **Gemini added** as first-class provider with native `generateContent` format.
- **Custom provider** is a unit variant (keeps `ModelProvider` Copy-compatible). String config lives in `CustomProviderConfig { name, base_url, format: ApiFormat }`.
- **ApiFormat** routes Custom providers to the correct request body builder and response parser.

Request paths:
| Provider | Endpoint |
|----------|----------|
| Claude / Custom | `/v1/messages` |
| OpenAI | `/v1/responses` |
| Gemini | `/v1beta/models/{model}:generateContent` |

## Tool Call Flow

After model response, `runtime.rs` extracts tool calls via `extract_tool_calls()`:
- Claude: `tool_use` content blocks → `ToolCallRequest { name, id, arguments }`
- OpenAI: `function_call` in response output → `ToolCallRequest`
- Gemini: `functionCall` in parts → `ToolCallRequest`

Each tool call becomes a `ToolCallInput` (via `with_arguments_map()`), gets dispatched through `execute_tool_call()`, results fed back via `QueryLoopAction::ResolveTool`, then `FlushToolBatch` before completion.

## Naming Decisions

Session-level structured output uses canonical result term:
- Display name: `response-result`
- Rust / wire field: `response_result`
- Task panel aggregation: `result`

Legacy `structured_output` retained only in provider JSON schema request name and bridge backward-compatible decode alias.

## Comparison with redcode

| Dimension | redcode baseline | nocode current | Status |
|-----------|-----------------|----------------|--------|
| Query kernel | TS QueryEngine + query.ts | Rust query_engine / query_loop | Done — submit, tool batch, budget, stop hook, persistence |
| Providers | 6 (Claude, OpenAI×2, Bedrock, Vertex, Mock) | 5 (Claude, OpenAI, Gemini, Custom, Mock) | Done — simplified, Gemini added |
| Tool runtime | 42 tools, deep UI coupling | 10 tools + MCP, independent | Core tools done, 32 tools deferred |
| Tool call loop | Model → tool → result → model | Same pattern in runtime.rs | Done — all 3 provider formats |
| Task runtime | Shell/agent/dream/daemon | Same 4 hosts + supervisor | Done — lacks persistence/resume |
| Bridge | Deep remote + session system | Runner + HTTP transport demo | Functional, not production |
| TUI | Ink REPL, 349 TSX components | 4-pane crossterm, overlays | Usable, not replacement-grade |
| MCP | Full client + auth + resources | JSON-RPC client, tool exec | Core done, lacks auth/resources |
| Release | install.sh, variants, flags, doctor | CI + install.sh | Minimal |

## Remaining Work (prioritized)

### P1. Provider Production Readiness
- [ ] Live chunk streaming transport (not just full SSE body parse)
- [ ] Stream event state machine: delta, tool call, turn finish, abort
- [ ] Finer error classification: auth, quota, rate limit, timeout, decode
- [ ] Provider integration tests with real API shapes

### P2. Task Runtime Hardening
- [ ] Cross-session task persistence table
- [ ] Task resume/reconnect after restart
- [ ] Cancellation / cleanup / kill escalation / timeout policy
- [ ] Task audit trail: spawn, permission, retry, restart, kill, final status

### P3. Bridge / Session
- [ ] Concrete bridge service (not just demo transport)
- [ ] Session registry, remote session pointer, resume
- [ ] WebSocket or equivalent long-connection transport
- [ ] Reconnect, heartbeat, timeout, auth refresh

### P4. TUI Completeness
- [ ] Permission prompt full lifecycle
- [ ] Transcript renderer: assistant/tool/progress/error typed rendering
- [ ] Input editor: selection, richer keybindings
- [ ] Error panel, diagnostics panel, richer footer
- [ ] Task panel: action keys, batch operations, auto-refresh

### P5. Platform / Release
- [ ] Doctor / compat / resume UX
- [ ] Packaging: binary releases, platform installers
- [ ] CI matrix: smoke tests, integration tests
- [ ] Configuration migration and rollback strategy

## Launch Criteria

Before `nocode` can replace `redcode`:

1. Provider has live streaming, capability matrix, clear error surface
2. Tasks have persistence, resume, cancel, daemon/service paths
3. Bridge has real remote transport, resume, reconnect
4. TUI can independently complete query/task/permission/bridge workflows
5. Release has packaging, install, doctor, smoke, rollback

Until all 5 are met, `nocode` remains internal preview.

## Change Log

### 2026-04-06
- Full document rewrite to reflect post-v0.2 state
- Updated provider architecture: 5 providers with ApiFormat routing
- Added tool call flow documentation
- Updated comparison table with current coverage
- Reorganized remaining work as P1-P5

### 2026-04-04
- Initial DESIGN.md with P1-P6 TODO structure
- Naming convention: `response-result` / `response_result`
