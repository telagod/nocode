# @telagod/nocode

Terminal-native AI coding agent in Rust. 11 atomic tools, three explainable gates, skills as first-class prompt material, fractal sub-agents. Connects to Claude, OpenAI, Gemini, or any compatible endpoint.

> **v0.3.0 — breaking release.** The `custom_*` config fields and `--login` wizard have been removed. Configure via `[providers.<name>]` tables in `~/.nocode/config.toml`; run `nocode init` for a template.

## Install

```bash
npm install -g @telagod/nocode
```

## Features

- Interactive TUI with structured `Denied [<gate>: <reason>]` why-trail
- 11 atomic core tools (Read, Write, Edit, Glob, Grep, Bash, WebFetch, WebSearch, Agent, AskUserQuestion, Skill) + 18 opt-in extension tools
- Multi-provider support via named `[providers.<name>]` tables (Claude, OpenAI, Gemini, or any compatible endpoint)
- Profiles for one-flag config swapping (`--profile work`)
- First-class skill system: `*.md` files in `.nocode/skills/` are indexed into the prompt
- `nocode insight` observability CLI: sessions, tools, gates, cost — no dashboard sprawl
- Session compaction with structured summaries
- MCP client (JSON-RPC over stdio) for tool extensibility

## Setup

```bash
nocode init                          # scaffold ~/.nocode/config.toml
export OPENAI_API_KEY=sk-...         # or whichever api_key_env you pick
nocode                               # launch the TUI
```

The fastest single-key path uses a builtin alias — no config file needed:

```bash
export ANTHROPIC_API_KEY=sk-ant-...
nocode --provider claude
```

## Usage

```bash
nocode                               # interactive TUI (default)
nocode init                          # scaffold config template
nocode config list                   # show current settings
nocode --status                      # system diagnostics
nocode insight                       # sessions / tools / gates / cost
nocode --provider <name>             # one-shot provider override
nocode --profile <name>              # one-shot profile
```

## Configuration

```toml
# ~/.nocode/config.toml
default_provider = "openai"
model = "gpt-5.5"

[providers.openai]
base_url    = "https://api.openai.com"
wire_api    = "openai-responses"     # anthropic | openai-responses | openai-chat | google
api_key_env = "OPENAI_API_KEY"
```

Full schema: see [docs/10_provider_config.md](https://github.com/telagod/nocode/blob/main/docs/10_provider_config.md).

## Environment Variables

| Variable | Purpose |
|----------|---------|
| `NOCODE_PROVIDER` | Name of a provider from `[providers.<name>]` (or builtin `claude` / `openai` / `gemini`) |
| `NOCODE_PROFILE` | Name of a profile from `[profiles.<name>]` |
| `NOCODE_MODEL` | Override model name |
| `NOCODE_SYSTEM_PROMPT` | Override system prompt |

## Supported Platforms

| Platform | Package |
|----------|---------|
| Linux x64 | `@telagod/nocode-linux-x64` |
| Linux ARM64 | `@telagod/nocode-linux-arm64` |
| macOS x64 | `@telagod/nocode-darwin-x64` |
| macOS ARM64 | `@telagod/nocode-darwin-arm64` |
| Windows x64 | `@telagod/nocode-win32-x64` |
| Windows ARM64 | `@telagod/nocode-win32-arm64` |

## License

MIT

