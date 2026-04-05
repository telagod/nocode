# nocode

[中文文档](README_CN.md)

A fast, native AI coding assistant for the terminal. Built in Rust.

## What is nocode?

nocode is a terminal-native AI assistant that reads, writes, and runs code alongside you. It connects directly to Claude, OpenAI, AWS Bedrock, or Google Vertex — no proxy, no wrapper, no Electron.

```bash
# Install
./install.sh

# Start coding
export ANTHROPIC_API_KEY="sk-ant-..."
nocode --repl
```

## Features

**10 built-in tools** — Read, Edit, Write, Bash, Glob, Grep, WebFetch, WebSearch, Agent, MCP

**6 model providers** — Claude Messages, OpenAI Chat, OpenAI Responses, AWS Bedrock, Google Vertex, Mock

**Two interfaces** — Line-mode REPL or full TUI with 4 panes, color rendering, and keyboard navigation

**Multi-agent** — Spawn parallel agent teams with `/team-create`, monitor with `/team-status`

**Safe by default** — Bash sandbox blocks destructive commands, permission rule engine gates tool access

**CLAUDE.md support** — Auto-discovers project instructions from `CLAUDE.md`, `.claude/CLAUDE.md`, `.claude/rules/*.md`

## Quick Start

```bash
# From source
git clone https://github.com/telagod/nocode.git
cd nocode
cargo build --release
./target/release/nocode --repl

# Or use the install script
./install.sh
```

Set your API key and start:

```bash
export ANTHROPIC_API_KEY="sk-ant-..."
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
| `F3` | Permission overlay (approve/deny with `a`/`d`) |
| `Esc` | Close overlay or quit |

## Provider Configuration

nocode auto-detects your provider from environment variables:

| Provider | Required Variable | Default Model |
|----------|-------------------|---------------|
| Claude | `ANTHROPIC_API_KEY` | `claude-sonnet-4-20250514` |
| OpenAI | `OPENAI_API_KEY` | `gpt-4.1` |
| Gemini | `GEMINI_API_KEY` | `gemini-2.5-flash` |
| Custom | `ANTHROPIC_API_KEY` + `NOCODE_MODEL_PROVIDER=custom` | (user-specified) |

Override with `NOCODE_MODEL_PROVIDER` and `NOCODE_MODEL`. Custom providers use `NOCODE_CUSTOM_BASE_URL` for endpoint override.

## Architecture

```
nocode (CLI binary)
  src/main.rs        — entry points, bootstrap config
  src/repl.rs        — REPL session, slash commands, task management
  src/tui.rs         — 4-pane TUI with crossterm
  src/claudemd.rs    — CLAUDE.md discovery and loading
  src/task_panel.rs  — task filtering and rendering

nocode-core (library)
  provider.rs          — Claude/OpenAI/Bedrock/Vertex adapters
  provider_transport.rs — HTTP client, SSE streaming, retry/backoff
  query_engine.rs      — conversation lifecycle, tool schema generation
  query_loop.rs        — turn execution, budget, stop hooks
  tool_execution/      — Read/Edit/Write/Bash/Glob/Grep/WebFetch/Agent
  tool_registry.rs     — tool registration, permission rules
  task_runtime.rs      — shell/agent/dream tasks, daemon supervisor
  bridge_runtime.rs    — local/remote bridge, permission callbacks
  session_persistence.rs — JSONL transcript/history/task persistence
```

## Roadmap

### Done

- [x] Query engine with conversation lifecycle and tool loop
- [x] 6 provider adapters (Claude, OpenAI Chat, OpenAI Responses, Bedrock, Vertex, Mock)
- [x] Tool use in API requests (tools JSON schema in request body)
- [x] 10 tools (Read, Edit, Write, Bash, Glob, Grep, WebFetch, WebSearch, Agent, MCP stub)
- [x] REPL with ~40 slash commands
- [x] TUI with 4 panes, color rendering, overlays
- [x] CLAUDE.md auto-discovery (user/project/rules/local)
- [x] Bash safety sandbox + permission rule engine
- [x] Context compaction (truncating)
- [x] Task runtime: shell, agent, dream, process daemon
- [x] Process agent supervisor with configurable restart/backoff
- [x] Bridge: local + remote HTTP transport
- [x] Session/transcript/task persistence (JSONL)
- [x] Team agent: `/team-create` parallel multi-agent
- [x] Git commands: `/commit` `/diff` `/branch`
- [x] Auth: `/login` `/logout` credential storage
- [x] `/doctor` system diagnostics
- [x] Plugin skeleton with manifest.json discovery
- [x] CI pipeline (GitHub Actions: fmt, clippy, test, release build)
- [x] install.sh packaging

### Next — v0.2

- [ ] Live chunk streaming in TUI (end-to-end verified with real API)
- [ ] Tool call parsing from model responses (Claude `tool_use` blocks, OpenAI `function_call`)
- [ ] MCP client implementation (JSON-RPC over stdio)
- [ ] IDE server mode (`--ide-server` JSON-RPC for VS Code/JetBrains)
- [ ] Summarization-based context compaction (call LLM to summarize dropped messages)
- [ ] Task resume from persisted JSONL on session restart
- [ ] Bedrock SigV4 auth / Vertex OAuth token refresh

### Future — v0.3+

- [ ] WebSocket bridge transport with reconnect/heartbeat
- [ ] Session registry and remote session resume
- [ ] Plugin execution runtime (not just discovery)
- [ ] Skill system
- [ ] Voice input mode
- [ ] Onboarding flow
- [ ] Telemetry (opt-in)
- [ ] Cross-platform packaging (macOS, Windows)

## Testing

```bash
cargo test          # 225 tests
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

## License

MIT
