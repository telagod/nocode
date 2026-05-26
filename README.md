# nocode

[中文文档](README_CN.md) | [Architecture](docs/02_architecture.md) | [Provider Config](docs/10_provider_config.md) | [CHANGELOG](CHANGELOG.md)

A terminal-native AI coding agent in Rust. Harness-engineering-bionics philosophy: 11 atomic tools, three explainable gates, skills as first-class prompt material, fractal sub-agents.

> **v0.3.0 — breaking release.** The legacy `custom_*` config scheme and `--login` wizard have been removed in favor of codex-style named providers + `nocode init` / `nocode config`. Upgrading from 0.2.x? See the migration block in [CHANGELOG](CHANGELOG.md#030---2026-05-26--realign-release) or just run `nocode` — the startup error message walks you through it.

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

```bash
nocode init                           # scaffold ~/.nocode/config.toml
export OPENAI_API_KEY="sk-..."        # or whichever api_key_env you pick
nocode                                # launch the TUI
```

The fastest single-key path uses one of three builtin aliases — no config file needed:

```bash
export ANTHROPIC_API_KEY=sk-ant-...   && nocode --provider claude
export OPENAI_API_KEY=sk-...          && nocode --provider openai
export GEMINI_API_KEY=...             && nocode --provider gemini
```

## Usage

```bash
nocode                               # interactive TUI (default)
nocode init                          # scaffold config template
nocode config list                   # show current settings
nocode --status                      # system diagnostics + active sqlite volume
nocode insight                       # observability summary (sessions, tools, gates, cost)
nocode --provider <name>             # one-shot provider override
nocode --profile <name>              # one-shot profile (provider + model + mode)
nocode --bridge-once "prompt"        # single-turn non-interactive
```

## Providers

Configured in `~/.nocode/config.toml` under `[providers.<name>]`. Multiple
endpoints coexist; switch with `--provider` / `--profile` / `NOCODE_PROVIDER`.

```toml
default_provider = "subfox"
model = "gpt-5.5"

[providers.subfox]
base_url    = "https://sub.foxnio.com/v1"
wire_api    = "openai-responses"      # anthropic | openai-responses | openai-chat | google
api_key_env = "OPENAI_API_KEY"
default_model = "gpt-5.5"

[providers.local-vllm]
base_url    = "http://localhost:8000/v1"
wire_api    = "openai-chat"
api_key_env = "VLLM_API_KEY"
default_model = "Qwen2.5-Coder-32B-Instruct"

[profiles.work]
provider = "subfox"

[profiles.home]
provider = "local-vllm"
permission_mode = "auto"
```

Three **builtin aliases** work without a `[providers.*]` table — useful for "I just have one key":

| Alias | Wire | Key env |
|---|---|---|
| `claude` / `anthropic` | `anthropic` | `ANTHROPIC_API_KEY` |
| `openai` | `openai-responses` | `OPENAI_API_KEY` |
| `gemini` / `google` | `google` | `GEMINI_API_KEY` |

Full schema and precedence chain in [`docs/10_provider_config.md`](docs/10_provider_config.md).

## Commands

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
| `/tasks-next` `/j` | Select next task |
| `/tasks-prev` `/k` | Select previous task |
| `/enter` | Open selected task |
| `/history-prev` `/history-next` | Navigate input history |

### Account
| Command | Description |
|---------|-------------|
| `/config` | Open the settings center (`/settings`, legacy `/login`) |
| `/logout` | Remove stored credentials |
| `/doctor` | System diagnostics (provider, tools, connectivity) |
| `/plugin list` | List discovered plugins |

## TUI

`nocode` runs as a TUI-first interface. To reconfigure: edit `~/.nocode/config.toml` directly, or use `nocode config set <key> <value>`.

| Key | Action |
|-----|--------|
| `Up` / `Down` | Scroll or navigate |
| `PgUp` / `PgDn` | Fast scroll |
| `Ctrl-P` / `Ctrl-N` | Input history |
| `Ctrl-U` | Clear input |
| `F1` / `?` | Help overlay |
| `F2` | Inspector overlay |
| `F3` | Permission overlay |
| `Esc` | Close overlay or quit |

## Environment Variables

| Variable | Purpose |
|----------|---------|
| `NOCODE_PROVIDER` | Name of a provider from `[providers.<name>]` (or builtin alias `claude` / `openai` / `gemini`) |
| `NOCODE_PROFILE` | Name of a profile from `[profiles.<name>]` |
| `NOCODE_MODEL` | Override model name |
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
| Linux ARM64 | `@telagod/nocode-linux-arm64` |
| macOS x64 | `@telagod/nocode-darwin-x64` |
| macOS ARM64 | `@telagod/nocode-darwin-arm64` |
| Windows x64 | `@telagod/nocode-win32-x64` |
| Windows ARM64 | `@telagod/nocode-win32-arm64` |

## License

MIT
