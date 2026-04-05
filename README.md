# nocode

[中文文档](README_CN.md)

A fast, native AI coding assistant for the terminal. Built in Rust.

## What is nocode?

nocode is a terminal-native AI assistant that reads, writes, and runs code alongside you. It connects directly to Claude, OpenAI, or Gemini — no proxy, no wrapper, no Electron. Any OpenAI-compatible or Claude-compatible endpoint works via the Custom provider.

```bash
# Install
./install.sh

# Start coding
export ANTHROPIC_API_KEY="sk-ant-..."
nocode --repl
```

## Features

**10 built-in tools** — Read, Edit, Write, Bash, Glob, Grep, WebFetch, WebSearch, Agent, MCP

**5 model providers** — Claude, OpenAI (Chat + Responses), Gemini, Custom, Mock

**Two interfaces** — Line-mode REPL or full TUI with 4 panes, color rendering, and keyboard navigation

**Multi-agent** — Spawn parallel agent teams with `/team-create`, monitor with `/team-status`

**Safe by default** — Bash sandbox blocks destructive commands, permission rule engine gates tool access

**CLAUDE.md support** — Auto-discovers project instructions from `CLAUDE.md`, `.claude/CLAUDE.md`, `.claude/rules/*.md`, `CLAUDE.local.md`

**MCP support** — JSON-RPC client over stdio, tool discovery and execution via `mcp:` prefix

**Tool call loop** — Parses tool calls from Claude `tool_use`, OpenAI `function_call`, and Gemini `functionCall` responses, executes them, and feeds results back to the model

## Quick Start

```bash
# From source
git clone https://github.com/telagod/nocode.git
cd nocode/rust
cargo build --release
./target/release/nocode --repl

# Or use the install script
./install.sh
```

Set your API key and go:

```bash
export ANTHROPIC_API_KEY="sk-ant-..."   # or OPENAI_API_KEY or GEMINI_API_KEY
nocode --repl          # line-mode REPL
nocode --tui           # full terminal UI
nocode --status        # system diagnostics
```

## Modes

| Flag | Description |
|------|-------------|
| `--repl` | Interactive REPL with command history and streaming |
| `--tui` | 4-pane TUI: transcript, task list, task detail, events |
| `--status` | Print system status and provider capability matrix |
| `--bridge-once "prompt"` | Single-turn local bridge execution |
| `--bridge-remote-once "prompt"` | Single-turn remote bridge over HTTP |
| `--process-agent-daemon` | Run as process agent daemon (internal) |
| `--process-agent-host` | Run as process agent host (internal) |

## Commands

**Session**: `/help` `/status` `/runtime` `/history` `/quit`

**Git**: `/commit <msg>` `/diff [args]` `/branch [name]`

**Tasks**: `/task-shell <cmd>` `/task-agent <id> <prompt>` `/task-dream` `/tasks` `/task-show`

**Teams**: `/team-create <subtask1; subtask2; ...>` `/team-status`

**Account**: `/login <key>` `/logout` `/doctor`

**Editing**: `/draft` `/edit` `/append` `/send` `/queue`

**Navigation**: `/focus <pane>` `/tasks-next` `/tasks-prev`

**Plugins**: `/plugin list`

## TUI Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `Alt-1..4` | Focus transcript / task list / task detail / events |
| `Tab` / `Shift-Tab` | Cycle pane focus |
| `Up/Down` | Scroll or navigate |
| `PgUp/PgDn` | Fast scroll |
| `Ctrl-P/N` | Input history |
| `Ctrl-U` | Clear input |
| `F1` / `?` | Help overlay |
| `F2` | Inspector overlay |
| `F3` | Permission overlay (`a` approve / `d` deny) |
| `Esc` | Close overlay or quit |

## Provider Configuration

nocode auto-detects your provider from environment variables:

| Provider | Required Variable | Default Model |
|----------|-------------------|---------------|
| Claude | `ANTHROPIC_API_KEY` | `claude-sonnet-4-20250514` |
| OpenAI | `OPENAI_API_KEY` | `gpt-4.1` |
| Gemini | `GEMINI_API_KEY` | `gemini-2.5-flash` |
| Custom | `NOCODE_MODEL_PROVIDER=custom` | (user-specified) |

Override with `NOCODE_MODEL_PROVIDER` and `NOCODE_MODEL`.

Additional env vars:

| Variable | Purpose |
|----------|---------|
| `NOCODE_MODEL` | Override model name for any provider |
| `NOCODE_MODEL_PROVIDER` | Force provider: `claude`, `openai`, `gemini`, `custom`, `mock` |
| `NOCODE_CUSTOM_BASE_URL` | Base URL for Custom provider |
| `NOCODE_CUSTOM_API_FORMAT` | Wire format for Custom: `claude`, `openai`, or `gemini` |
| `NOCODE_SYSTEM_PROMPT` | Override the default system prompt |
| `NOCODE_MODEL_REASONING_EFFORT` | Reasoning effort: `low`, `medium`, `high` |
| `ANTHROPIC_MODEL` / `OPENAI_MODEL` / `GEMINI_MODEL` | Per-provider model override |
| `ANTHROPIC_BASE_URL` / `OPENAI_BASE_URL` | Per-provider base URL override |

## Architecture

```
nocode (CLI binary — crates/nocode)
  src/main.rs        — entry point, provider detection, bootstrap config
  src/repl.rs        — REPL session, ~40 slash commands, task management
  src/tui.rs         — 4-pane TUI with crossterm, async streaming, overlays
  src/claudemd.rs    — CLAUDE.md discovery and loading
  src/task_panel.rs  — task filtering and rendering

nocode-core (library — crates/nocode-core)
  provider.rs          — ModelProvider (Claude/OpenAI/Gemini/Custom/Mock), request/response/streaming
  provider_transport.rs — HTTP client, SSE parsing, retry/backoff, per-provider auth
  query_engine.rs      — conversation lifecycle, tool schema generation
  query_loop.rs        — turn execution, budget, stop hooks
  tool_execution/      — 10 tools: Read/Edit/Write/Bash/Glob/Grep/WebFetch/WebSearch/Agent/MCP
  tool_registry.rs     — tool registration, permission rules engine
  task_runtime.rs      — shell/agent/dream tasks, process daemon supervisor
  bridge_runtime.rs    — local/remote bridge, permission callbacks
  session_persistence.rs — JSONL session/transcript/history/task persistence
  mcp_client.rs        — MCP JSON-RPC client over stdio
  budget.rs            — token budget tracking and decisions
  query_config.rs      — query configuration with tool definitions
  query_deps.rs        — dependency injection, context compaction
```

## Roadmap

### v0.1 — Done

- [x] Query engine with full conversation lifecycle and tool loop
- [x] 5 provider adapters (Claude, OpenAI, Gemini, Custom, Mock)
- [x] Tool use in API requests (tools JSON schema in request body)
- [x] 10 tools (Read, Edit, Write, Bash, Glob, Grep, WebFetch, WebSearch, Agent, MCP)
- [x] REPL with ~40 slash commands
- [x] TUI with 4 panes, color rendering, overlay system
- [x] CLAUDE.md auto-discovery (user/project/rules/local)
- [x] Bash safety sandbox + permission rule engine (9 preset rules)
- [x] Context compaction (truncating compactor)
- [x] Task runtime: shell, agent, dream, process daemon with supervisor
- [x] Bridge: local + remote HTTP transport
- [x] Session/transcript/history persistence (JSONL)
- [x] Team agent: `/team-create` parallel multi-agent
- [x] Git commands: `/commit` `/diff` `/branch`
- [x] Auth: `/login` `/logout` credential storage
- [x] `/doctor` system diagnostics
- [x] Plugin skeleton with manifest.json discovery
- [x] CI pipeline (GitHub Actions: fmt, clippy, test, release build)
- [x] install.sh packaging

### v0.2 — Done

- [x] Tool call parsing from model responses (Claude `tool_use`, OpenAI `function_call`, Gemini `functionCall`)
- [x] Tool execution loop in runtime — model requests tools, engine executes and feeds results back
- [x] MCP client implementation (JSON-RPC over stdio, tool discovery, tool execution)
- [x] Provider simplification: 6 → 5 providers, removed Bedrock/Vertex as first-class, added Gemini + Custom
- [x] ApiFormat enum for Custom provider wire format routing (Claude/OpenAI/Gemini)

### v0.3 — Next

- [ ] Live chunk streaming in TUI (end-to-end verified with real API)
- [ ] IDE server mode (`--ide-server` JSON-RPC for VS Code/JetBrains)
- [ ] Summarization-based context compaction (LLM-powered)
- [ ] Task resume from persisted JSONL on session restart
- [ ] WebSocket bridge transport with reconnect/heartbeat
- [ ] Session registry and remote session resume

### Future

- [ ] Plugin execution runtime (not just discovery)
- [ ] Skill system
- [ ] Voice input mode
- [ ] Onboarding flow
- [ ] Telemetry (opt-in)
- [ ] Cross-platform packaging (macOS, Windows)

## Testing

```bash
cargo test                                      # 30 tests
cargo clippy --all-targets -- -D warnings       # lint
cargo fmt --check                               # format
```

## License

MIT
