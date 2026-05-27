# 02 · Architecture

> Last updated: 2026-05-27 · Owner: harness · Read with [00_vision](./00_vision.md), [04_policy_gates](./04_policy_gates.md)

The module map for nocode v0.3.0. This file describes **what is** — the goals, the decisions, and the reasons live in [00_vision](./00_vision.md). The forward-looking backlog lives in [08_roadmap](./08_roadmap.md).

## Workspace

A two-crate Cargo workspace. Edition 2024. `clippy::all + pedantic + nursery = warn`. `unsafe = forbid` outside the test-support env-mutex helper.

```text
crates/
├── nocode-core/      # library (~28K LOC) — every piece of behaviour
│   └── src/
│       ├── agent/    # WorkerRegistry, TaskCoordinator, background hosts
│       ├── auth/     # OAuth flows (feature-gated)
│       ├── config/   # 3-tier settings merge + RuntimeConfig + reject_legacy_custom
│       ├── mcp/      # MCP client (JSON-RPC over stdio), 11-phase lifecycle
│       ├── prompt/   # System prompt assembly — base + CLAUDE.md + AGENTS.md + skill index
│       ├── provider/ # Claude / OpenAI / Gemini / Foundry providers + resolve_named_provider
│       ├── query/    # Agentic loop, token budget, dependency injection seam
│       ├── recovery/ # 7 failure scenarios → RecoveryRecipe (one attempt before escalation)
│       ├── session/  # JSONL persistence, RichCompactor, fork/branch/resume
│       ├── skill/    # SkillRegistry — first-class, loaded into prompt assembly
│       ├── storage/  # rusqlite (date-partitioned volumes) + memory CRUD + credentials
│       ├── tool/     # 11 atomic + 18 optional tools, executor, policy, hooks
│       ├── bridge.rs # Local + remote single-turn transport
│       ├── ide_server.rs   # JSON-RPC IDE server mode
│       ├── ws_bridge.rs    # WebSocket server mode
│       └── lib.rs
│
└── nocode/           # binary (~14.5K LOC) — TUI + CLI shell
    └── src/
        ├── main.rs           # entry, mode dispatch, provider resolution
        ├── tui.rs / tui_app.rs / tui_widgets.rs  # the TUI
        ├── tui_overlays.rs / tui_permission.rs / tui_theme.rs
        ├── tui_commands.rs   # slash-command handlers
        ├── command_registry.rs                   # slash registry
        ├── init.rs           # `nocode init` — config scaffold
        ├── config_cli.rs     # `nocode config <list|get|set|unset>`
        ├── insight.rs        # `nocode insight where|sessions|tools|gates|cost`
        ├── markdown_render.rs / markdown_stream.rs
        ├── tool_render.rs    # tool result rendering for the TUI
        └── …                 # spinner, status_hud, tool_truncate, model_fetch, etc.
```

## Core data flow

```text
User input
   │
   ▼
main.rs (mode dispatch)
   │
   ▼
QueryEngine ──▶ Provider trait ──▶ SSE stream
   │                                   │
   │  ◀── tool call ──┐                ▼
   │                  │           streamed deltas
   ▼                  │
ToolExecutor ─── policy(why-trail) ─── hooks ─── execute
   │
   └──▶ result back to QueryEngine loop
```

The agentic loop in [`query/loop.rs`](../crates/nocode-core/src/query/loop.rs) is the heart. Each iteration:

1. Send `messages + tools + system` to the model via `provider/transport.rs` (SSE).
2. Parse the streamed response.
3. Extract any tool calls and run them through `ToolExecutor`.
4. Feed tool results back as user-role `tool_result` blocks.
5. Loop until the model stops requesting tools, hits a `stop_reason`, or the budget is exhausted.

## Provider system

Trait-based dispatch — `provider/mod.rs` defines:

```rust
pub trait Provider: Send + Sync {
    fn create_message(...);
    fn create_message_stream(...);
    fn create_message_stream_with_cancel(...);
    fn verify_key(...);
}
```

Concrete implementations live next to the trait: `ClaudeProvider`, `OpenAiResponsesProvider`, `OpenAiProvider` (chat-completions), `GeminiProvider`, `FoundryProvider` (Anthropic-format proxy).

`ProviderBox` wraps `Arc<dyn Provider>` for owned trait objects. The construction goes through the **single sanctioned resolver** [`provider::resolve::resolve_named_provider`](../crates/nocode-core/src/provider/resolve.rs):

```text
cli flag --provider <name>
  └─→ NOCODE_PROVIDER env var
        └─→ active profile.provider
              └─→ settings.default_provider
                    └─→ settings.model_provider (legacy alias)
```

The chosen name is then looked up in `[providers.<name>]` (or one of the builtin aliases `claude`/`openai`/`gemini`) for `base_url`, `wire_api`, `api_key_env`, `default_model`. Schema details: [10_provider_config.md](./10_provider_config.md).

The pre-REALIGN `ModelProvider::Custom` variant and `custom_*` Settings fields are **rejected at load time** with an actionable migration message — see `Settings::reject_legacy_custom` in `config/settings.rs`.

### Wire formats

A simple string with four canonical values: `anthropic`, `openai-responses`, `openai-chat`, `google`. Anything else is a hard error in the resolver. Legacy values (`claude` / `openai` / `gemini`) are normalised to one of the four via `normalize_api_format()`.

| Wire | Endpoint shape |
|---|---|
| `anthropic` | `POST {base_url}/v1/messages` |
| `openai-responses` | `POST {base_url}/v1/responses` |
| `openai-chat` | `POST {base_url}/v1/chat/completions` |
| `google` | `POST {base_url}/v1beta/models/{model}:generateContent` |

## Tool execution pipeline (three gates)

`tool/executor.rs` runs every tool call through:

1. **Lookup** — `ToolRegistry::get(name)`, then `GlobalToolRegistry` for bridged `mcp:*` / `plugin:*` names.
2. **Schema** — JSON-schema validation in `tool/tool_validation.rs`.
3. **Policy** — the unified gate in [`tool/policy.rs`](../crates/nocode-core/src/tool/policy.rs). One `PolicyEngine::evaluate()` call collapses what used to be five inline checks (trust + plan-mode + permission-mode + classifier + sandbox) and returns a `GateDecision { gate, reason, remember }`. The why-trail is the design contract — see [04_policy_gates](./04_policy_gates.md).
4. **PreToolUse hooks** — external commands; non-zero exit denies.
5. **Bash classifier** — extra syntax-level guards via `bash_validation::is_destructive_command`.
6. **Execute** — `tool.execute(input)`.
7. **Snapshot for undo** — `FileEdit` / `FileWrite` snapshot old + new content into the file-history store.
8. **PostToolUse hooks** — informational; cannot deny.
9. **Render** as a `ContentBlock::ToolResult` (with `is_error` and optional `structured_content`).

### Tool surface

Default: 11 atomic tools (the harness contract):

```
FileRead  FileWrite  FileEdit       # files
Glob      Grep                       # search
Bash                                 # execute
WebFetch  WebSearch                  # world
Agent     AskUserQuestion  Skill     # fractal + dialogue + skill
```

Asserted by `tests/roadmap.rs::tool_registry_has_canonical_core_set`. Add or remove and that test fails.

Optional (registered explicitly when the host wants them): `Memory`, `TodoWrite`, `Task`, `Mcp`, `Cron{Create,List,Delete}`, `Team{Create,Delete}`, `EnterPlanMode`/`ExitPlanMode`, `EnterWorktree`/`ExitWorktree`, `Config`, `NotebookEdit`, `ToolSearch`, `Lsp`, `SendMessage`.

## Configuration (3 tiers)

Loaded by `Settings::load_merged(cwd)`, in this order, with later layers winning for scalars and merging key-by-key for maps:

```text
~/.nocode/config.toml                 # user (global)
{cwd}/.nocode/config.toml             # project
{cwd}/.nocode/config.local.toml       # local (gitignored)
```

Then `RuntimeConfig::from_settings` applies env-var overrides on top.

## State machines (explicit, not inferred)

Every long-running lifecycle is an explicit enum with logged transitions. Infer-state-from-fields is forbidden.

| Subject | Phases |
|---|---|
| `WorkerState` | `Spawning → TrustRequired → ReadyForPrompt → Running → Finished/Failed` |
| `McpLifecyclePhase` | 11 phases (`Registered → … → Shutdown`), with `Degraded`/`Reconnecting` branches |
| `PluginState` | `Unconfigured → Validated → Healthy/Degraded/Failed` |
| `SessionControl` | `Idle → Active → Paused → Resuming → Draining → Terminated` |

## Global singletons

The `OnceLock<Arc<Mutex<T>>>` pattern, accessed via `global_*()` functions in their owning modules:

```
TaskCoordinator     WorkerRegistry        McpManager
HookRunner          CronRegistry          PluginRegistry
SqlStore            GlobalToolRegistry    LspRegistry
```

Tests that touch any of these (or `$HOME` / `cwd`) serialize through `crate::test_support::env_mutex()` to avoid races.

## Storage

| Store | Path | Purpose |
|---|---|---|
| **SQL** | `~/.nocode/data/nocode_YYYY-MM-DD.db` | sessions, messages, telemetry, commands, memories. Date-partitioned volumes — never one giant file. |
| **Memory** | `~/.nocode/memory/` (file-system) | Markdown + YAML frontmatter, `MEMORY.md` index. CRUD via `storage/memory.rs`. |
| **Sessions** | `~/.nocode/data/nocode_YYYY-MM-DD.db` (sessions table) + `.nocode/last_session` marker | JSONL transcript / history / task persistence with auto-persist on submission. |
| **Credentials** | `~/.nocode/credentials.json` | Encrypted API key storage (used when keys come from non-env sources). |

## Session control

`session/compaction.rs` (`RichCompactor`) produces structured summaries when context grows too large.

`session/persistence.rs` writes a **transcript-shaped JSONL** plus per-session metadata to SQL. Resume flows: `session/control.rs` supports fork / branch / resume / suspend / complete with `parent_id` tracking.

Token budget tracked in `query/budget.rs` with diminishing-returns logic — once the model's output is shrinking faster than its input, the loop breaks.

## Recovery

`recovery.rs` maps **7 failure scenarios** to a `RecoveryRecipe { steps, max_attempts, escalation }`. The hard rule: **one attempt before escalation, never silently retry indefinitely.**

| Scenario | Recipe |
|---|---|
| ProviderAuthFailure | refresh creds → retry once → escalate |
| ProviderRateLimited | back off → retry once → defer |
| ProviderTimeout | retry once → escalate |
| ToolCallMalformed | reject with structured error → no retry |
| BashCommandFailed | informational only — model decides |
| HookDenied | bubble up the trail to the user → no retry |
| StreamDecodeError | retry once with reduced context → escalate |

## Testing strategy

- **Unit tests** — inline `#[cfg(test)] mod tests` next to the code they cover.
- **Integration tests** — `crates/nocode-core/tests/` (4 binaries):
  - `mock_service.rs` — `MockAnthropicService` parity, `CapturedRequest` recording.
  - `tool_roundtrip.rs` — full executor pipeline, gate trails, hooks, sandbox.
  - `trust_mcp.rs` — trust/permission/MCP health.
  - `roadmap.rs` — module-presence and core-tool-set contracts.
- **Hermetic env mutation** — every test that writes `$HOME`, `cwd`, or `NOCODE_*` env vars holds a process-wide `Mutex` from `test_support::env_mutex()`.

Total: **825 tests** as of v0.3.0. `cargo clippy --all-targets --no-deps -- -D warnings` is clean across both crates.

## Run modes

```bash
nocode                                 # interactive TUI (default)
nocode init [--force]                  # scaffold config
nocode config <list|get|set|unset>     # CLI config mutator
nocode insight [<sub>] [--json]        # observability subcommands
nocode --status                        # diagnostics + active sqlite volume
nocode --resume [<id>]                 # resume a previous session (-c shorthand)
nocode --bridge-once "prompt"          # single-turn local execution
nocode --bridge-remote-once "prompt"   # single-turn HTTP
nocode --ws-server <bind>              # WebSocket server
nocode --ide-server                    # IDE server (JSON-RPC)
nocode --mcp-server                    # MCP server
nocode --process-agent-daemon          # background daemon (internal)
nocode --process-agent-host            # agent host (internal)
```

## Key conventions

- **`ModelProvider`** is a Copy enum (`Claude | OpenAi | Gemini | Custom`). Used only for legacy paths; new code uses `ResolvedProvider`.
- **API format** is a string with 4 canonical values, not a Rust enum (deliberate — string is forgiving across versions).
- **System prompt assembly** is dynamic — `prompt/assembly.rs` discovers `CLAUDE.md` / `AGENTS.md` variants, deduplicates by FNV hash, applies truncation budgets.
- **Slash commands** registered in `command_registry.rs` with aliases / summary / argument hints.
- **Source paths** use module subdirectories: `provider/`, `tool/`, `mcp/`, `query/`, `config/`, `session/`, `storage/`, `auth/`, `agent/`, `prompt/`, `skill/`.
- **Structured typed events** for observability — never scrape prose.
