# nocode

[中文文档](README_CN.md) | [Development Guide](docs/DEVELOPMENT.md)

A fast, native AI coding assistant for the terminal. Built in Rust.

## Install

```bash
npm install -g @telagod/nocode
```

Or build from source:

```bash
git clone https://github.com/telagod/nocode.git
cd nocode && cargo build --release
cp target/release/nocode ~/.local/bin/
```

## Setup

Set your API key:

```bash
export ANTHROPIC_API_KEY="sk-ant-..."   # Claude (default)
export OPENAI_API_KEY="sk-..."          # OpenAI
export GEMINI_API_KEY="..."             # Gemini
```

## Usage

```bash
nocode --repl          # interactive REPL
nocode --tui           # 4-pane terminal UI
nocode --status        # system diagnostics
```

## Providers

nocode auto-detects your provider from environment variables:

| Provider | API Key Variable | Default Model |
|----------|-----------------|---------------|
| Claude | `ANTHROPIC_API_KEY` | `claude-sonnet-4-20250514` |
| OpenAI | `OPENAI_API_KEY` | `gpt-4.1` |
| Gemini | `GEMINI_API_KEY` | `gemini-2.5-flash` |
| Custom | `NOCODE_CUSTOM_BASE_URL` | (user-specified) |

Override with:

```bash
export NOCODE_MODEL_PROVIDER=openai    # force provider
export NOCODE_MODEL=gpt-4.1            # force model
```

Any OpenAI-compatible or Claude-compatible endpoint works via Custom provider:

```bash
export NOCODE_MODEL_PROVIDER=custom
export NOCODE_CUSTOM_BASE_URL=http://localhost:11434/v1
export NOCODE_CUSTOM_API_FORMAT=openai
export NOCODE_MODEL=llama3
```

## Features

**25 built-in tools** — Read, Edit, Write, Bash, Glob, Grep, WebFetch, WebSearch, Agent, Tasks, Teams, MCP, LSP, Memory, and more

**Multi-provider** — Claude, OpenAI, Gemini, or any compatible endpoint

**Two interfaces** — Line-mode REPL or 4-pane TUI with Markdown rendering and syntax highlighting

**Multi-agent** — Spawn parallel agent teams for complex tasks

**Safe by default** — Bash sandbox blocks destructive commands, 3-tier permission model gates tool access

**MCP support** — Connect external tools via Model Context Protocol (JSON-RPC over stdio)

**CLAUDE.md support** — Auto-discovers project instructions from `CLAUDE.md`, `.claude/CLAUDE.md`, `.claude/rules/*.md`

**Persistent memory** — Remembers context across sessions with auto-detection signals

**SQL storage** — Clean session/memory storage with date-based volume partitioning

## Commands

| Category | Commands |
|----------|----------|
| Session | `/help` `/status` `/runtime` `/history` `/quit` |
| Git | `/commit <msg>` `/diff` `/branch` |
| Tasks | `/tasks` `/task-shell <cmd>` `/task-agent <id> <prompt>` `/task-dream` |
| Teams | `/team-create <subtask1; subtask2; ...>` `/team-status` |
| Account | `/login <key>` `/logout` `/doctor` |
| Editing | `/draft` `/edit` `/append` `/send` |

## TUI Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `Alt-1..4` | Focus pane (transcript / tasks / detail / events) |
| `Tab` / `Shift-Tab` | Cycle pane focus |
| `Up/Down` `PgUp/PgDn` | Scroll |
| `Ctrl-P/N` | Input history |
| `F1` | Help overlay |
| `F2` | Inspector overlay |
| `F3` | Permission overlay |
| `Esc` | Close overlay or quit |

## Supported Platforms

| Platform | npm Package |
|----------|-------------|
| Linux x64 | `@telagod/nocode-linux-x64` |
| macOS x64 | `@telagod/nocode-darwin-x64` |
| macOS ARM64 | `@telagod/nocode-darwin-arm64` |

## License

MIT
