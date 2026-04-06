# nocode

[中文文档](README_CN.md) | [Development Guide](docs/DEVELOPMENT.md)

A terminal-native AI coding assistant built in Rust. 38K LOC, 51 modules, 25 tools, 555 tests.

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

Set one API key and go:

```bash
export ANTHROPIC_API_KEY="sk-ant-..."   # Claude (default)
# or
export OPENAI_API_KEY="sk-..."          # OpenAI
# or
export GEMINI_API_KEY="..."             # Gemini
```

## Usage

```bash
nocode --repl                        # interactive REPL
nocode --tui                         # 4-pane terminal UI
nocode --status                      # system diagnostics
nocode --bridge-once "prompt"        # single-turn local execution
nocode --bridge-remote-once "prompt" # single-turn remote execution
```

## Providers

Auto-detected from environment variables. Priority: explicit override > Google > OpenAI > Anthropic > Mock.

| Provider | API Key | Default Model |
|----------|---------|---------------|
| Anthropic (Claude) | `ANTHROPIC_API_KEY` | `claude-opus-4-6` |
| OpenAI (GPT) | `OPENAI_API_KEY` | `gpt-5.4` |
| Google (Gemini) | `GEMINI_API_KEY` | `gemini-3.1-pro` |
| Custom | `NOCODE_CUSTOM_BASE_URL` | user-specified |
| Mock | (none, fallback) | `sonnet` |

Override provider or model:

```bash
export NOCODE_MODEL_PROVIDER=anthropic
export NOCODE_MODEL=gpt-5.4
```

Use any OpenAI/Claude-compatible endpoint (Ollama, vLLM, LiteLLM, etc.):

```bash
export NOCODE_MODEL_PROVIDER=custom
export NOCODE_CUSTOM_BASE_URL=http://localhost:11434/v1
export NOCODE_CUSTOM_API_FORMAT=openai   # anthropic (Messages API) | openai (Chat/Responses) | google (generateContent)
export NOCODE_MODEL=llama3
```

## REPL Commands

### Session
| Command | Description |
|---------|-------------|
| `/help` | Show all commands |
| `/status` | Provider and engine status |
| `/runtime` | Runtime diagnostics |
| `/history` | Show conversation history |
| `/inputs` | Show raw input history |
| `/quit` | Exit |

### Git
| Command | Description |
|---------|-------------|
| `/commit <message>` | `git add -A && git commit -m "..."` |
| `/diff [args]` | Run `git diff` |
| `/branch [name]` | Run `git branch` |

### Tasks
| Command | Description |
|---------|-------------|
| `/tasks [filter]` | List tasks. Filters: `all`, `completed`, `shell`, `agent`, `status:X type:Y` |
| `/task-shell <command>` | Spawn a shell task |
| `/task-agent <agent-id> <prompt>` | Spawn an agent task |
| `/task-dream [sessions] [description]` | Spawn a dream task |
| `/task-show <id\|first\|last\|latest\|prev\|next>` | Show task detail |
| `/task-open` | Open selected task |
| `/task-queue` | Show task queue |
| `/task-run-next` | Run next queued task |
| `/task-run-all` | Run all queued tasks |
| `/task-stop <task-id>` | Stop a running task |

### Teams
| Command | Description |
|---------|-------------|
| `/team-create <subtask1; subtask2; ...>` | Spawn parallel agent team |
| `/team-status` | Show team status |

### Editing
| Command | Description |
|---------|-------------|
| `/draft <text>` | Start a draft message |
| `/edit <text>` | Replace draft content |
| `/append <text>` | Append to draft |
| `/send` | Send the draft |
| `/queue <prompt>` | Queue a prompt for later |
| `/queue-slash </command>` | Queue a slash command |
| `/queue-show` | Show queued items |

### Navigation
| Command | Description |
|---------|-------------|
| `/focus <transcript\|tasks\|detail>` | Focus a TUI pane |
| `/tasks-next` `/j` | Select next task |
| `/tasks-prev` `/k` | Select previous task |
| `/enter` | Open selected task |
| `/history-prev` `/history-next` | Navigate input history |

### Account
| Command | Description |
|---------|-------------|
| `/login <api-key>` | Store API key to `~/.nocode/credentials` |
| `/logout` | Remove stored credentials |
| `/doctor` | System diagnostics (provider, tools, connectivity) |
| `/plugin list` | List discovered plugins |

## TUI

4-pane fullscreen interface with Markdown rendering (pulldown-cmark + syntect syntax highlighting), RGB color support, and overlay system.

| Key | Action |
|-----|--------|
| `Alt-1..4` | Focus pane (transcript / task list / task detail / events) |
| `Tab` / `Shift-Tab` | Cycle pane focus |
| `Up` / `Down` | Scroll or navigate |
| `PgUp` / `PgDn` | Fast scroll |
| `Ctrl-P` / `Ctrl-N` | Input history |
| `Ctrl-U` | Clear input |
| `F1` / `?` | Help overlay |
| `F2` | Inspector overlay |
| `F3` | Permission overlay (`a` approve / `d` deny) |
| `Esc` | Close overlay or quit |

## Environment Variables

| Variable | Purpose |
|----------|---------|
| `NOCODE_MODEL_PROVIDER` | Force provider: `anthropic`, `openai`, `google`, `custom`, `mock` |
| `NOCODE_MODEL` | Override model name |
| `NOCODE_CUSTOM_BASE_URL` | Base URL for Custom provider |
| `NOCODE_CUSTOM_API_FORMAT` | API wire format: `anthropic` (Messages API), `openai` (Chat Completions / Responses), `google` (generateContent) |
| `NOCODE_SYSTEM_PROMPT` | Override system prompt |
| `NOCODE_MODEL_REASONING_EFFORT` | `low`, `medium`, `high` |
| `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` / `GEMINI_API_KEY` | Provider API keys |
| `ANTHROPIC_MODEL` / `OPENAI_MODEL` / `GEMINI_MODEL` | Per-provider model override |
| `ANTHROPIC_BASE_URL` / `OPENAI_BASE_URL` | Per-provider base URL override |
| `NOCODE_BRIDGE_BASE_URL` | Remote bridge endpoint |
| `NOCODE_BRIDGE_AUTH_TOKEN` | Bearer token for remote bridge |

## Supported Platforms

| Platform | npm Package |
|----------|-------------|
| Linux x64 | `@telagod/nocode-linux-x64` |
| macOS x64 | `@telagod/nocode-darwin-x64` |
| macOS ARM64 | `@telagod/nocode-darwin-arm64` |
| Windows x64 | `@telagod/nocode-win32-x64` |

## License

MIT
