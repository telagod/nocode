<div align="center">

# nocode

**The smallest harness for a fractal code agent.**

A terminal-native AI coding agent in Rust — 11 atomic tools, three explainable gates, skills as first-class prompt material, sub-agents that share the parent's harness by construction.

[![npm](https://img.shields.io/npm/v/@telagod/nocode?label=npm&color=a78bfa)](https://www.npmjs.com/package/@telagod/nocode)
[![tests](https://img.shields.io/badge/tests-825%20green-86efac)](#status)
[![clippy](https://img.shields.io/badge/clippy-D%20warnings%20clean-86efac)](https://github.com/telagod/nocode/blob/main/.github/workflows/ci.yml)
[![release](https://img.shields.io/github/v/release/telagod/nocode?color=a78bfa)](https://github.com/telagod/nocode/releases/latest)
[![license](https://img.shields.io/npm/l/@telagod/nocode?color=6e6a7d)](LICENSE)

[**Website**](https://telagod.github.io/nocode/) ·
[中文](README_CN.md) ·
[CHANGELOG](CHANGELOG.md) ·
[Docs](docs/README.md)

</div>

---

> **v0.3.0 — breaking release.** The legacy `custom_*` config scheme and `--login` wizard have been removed in favor of codex-style named providers + `nocode init` / `nocode config`. Upgrading from 0.2.x? Just run `nocode` — the startup error walks you through the migration with a precise diff. Or skim [CHANGELOG](CHANGELOG.md).

## 30-second start

```bash
npm install -g @telagod/nocode
nocode init                           # scaffold ~/.nocode/config.toml
export OPENAI_API_KEY=sk-...          # or whichever key your provider needs
nocode                                # launch the TUI
```

Have just one API key and one endpoint? Skip the config file:

```bash
export ANTHROPIC_API_KEY=sk-ant-...
nocode --provider claude              # builtin alias, works out of the box
```

Builtin aliases: `claude` · `openai` · `gemini`.

## Four invariants

The whole product is these four. Everything else is consequence.

1. **Skill is first-class** — `*.md` files in `.nocode/skills/` are loaded into prompt assembly with the same priority as `CLAUDE.md`. The model sees the index *before* it acts. Bodies materialize lazily. → [docs/03_skills.md](docs/03_skills.md)
2. **11 atomic tools** — One job per tool, no overlap. Default registry is fixed and asserted by an integration test. Everything else is opt-in. → [docs/02_architecture.md](docs/02_architecture.md)
3. **Three explainable gates** — `Schema → Policy → Hooks`. Every refusal carries `Denied [<gate>: <reason>]`. No silent denies. → [docs/04_policy_gates.md](docs/04_policy_gates.md)
4. **Fractal sub-agents** — `Agent` spawn runs the *same* harness as its parent. Recursion inherits both capability and constraint. → [docs/05_fractal_subagents.md](docs/05_fractal_subagents.md)

## The eleven core tools

| | | |
|---|---|---|
| `FileRead` observe a file | `FileWrite` create | `FileEdit` mutate |
| `Glob` find paths | `Grep` find content | `Bash` execute |
| `WebFetch` retrieve URL | `WebSearch` discover | `Agent` spawn sub-agent |
| `AskUserQuestion` ask | `Skill` invoke skill | |

Optional tools (`Memory`, `TodoWrite`, `Cron*`, `Team*`, `Mcp`, `NotebookEdit`, `Lsp`, …) live in their own modules and are registered explicitly when the host wants them.

## Configuration

One TOML file. Run `nocode init` once for a commented template.

```toml
# ~/.nocode/config.toml
default_provider = "subfox"
model = "gpt-5.5"
permission_mode = "ask"

[providers.subfox]
base_url    = "https://sub.foxnio.com/v1"
wire_api    = "openai-responses"      # anthropic | openai-responses | openai-chat | google
api_key_env = "OPENAI_API_KEY"        # name of env var holding the key — explicit, no fallback chain
default_model = "gpt-5.5"

[providers.local-vllm]
base_url    = "http://localhost:8000/v1"
wire_api    = "openai-chat"
api_key_env = "VLLM_API_KEY"

[profiles.work]
provider = "subfox"

[profiles.home]
provider = "local-vllm"
permission_mode = "auto"
```

Switch in one flag:

```bash
nocode --provider local-vllm     # one-shot
nocode --profile work            # apply a named profile
NOCODE_PROVIDER=openai nocode    # via env
```

Full schema, precedence chain, and migration guide: **[docs/10_provider_config.md](docs/10_provider_config.md)**.

## CLI surface

```bash
nocode                              # interactive TUI (default)
nocode init [--force]               # scaffold ~/.nocode/config.toml
nocode config <list|get|set|unset>  # inspect / mutate scalar settings
nocode --status                     # diagnostics + active sqlite volume + top tools/gates
nocode insight [where|sessions|tools|gates|cost]  # observability — questions, not dashboards
nocode --provider <name>            # one-shot provider override
nocode --profile <name>             # one-shot profile (provider + model + mode)
nocode --resume [<session-id>]      # resume a previous session (-c shorthand)
nocode --bridge-once "<prompt>"     # single-turn non-interactive
nocode --help                       # full flag reference
```

## TUI keys

| Key | Action |
|---|---|
| `↑` / `↓` · `PgUp` / `PgDn` | Scroll |
| `Ctrl-P` / `Ctrl-N` | Input history |
| `Ctrl-U` | Clear input |
| `Ctrl-O` | Toggle tool output / thinking expansion |
| `Esc` | Cancel current stream / close overlay |
| `F1` / `?` | Help overlay |

## Environment variables

| Variable | Purpose |
|---|---|
| `NOCODE_PROVIDER` | Name of a provider from `[providers.<name>]` (or builtin alias) |
| `NOCODE_PROFILE` | Name of a profile from `[profiles.<name>]` |
| `NOCODE_MODEL` | Override the model |
| `NOCODE_SYSTEM_PROMPT` | Override the system prompt |
| `NOCODE_MODEL_REASONING_EFFORT` | `low`, `medium`, `high` |
| `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` / `GEMINI_API_KEY` | Provider API keys |
| `ANTHROPIC_BASE_URL` / `OPENAI_BASE_URL` | Per-provider base URL override |

## Status

| | |
|---|---|
| Latest release | **v0.3.0** ([CHANGELOG](CHANGELOG.md)) |
| Tests | **825 green** (160 TUI · 616 lib · 12 mock · 11 roadmap · 16 tool_roundtrip · 10 trust_mcp) |
| clippy | `--all-targets -- -D warnings` clean |
| Edition | Rust 2024 · MSRV 1.85 |
| Platforms | linux/macOS/Windows · x64 + arm64 (6 binaries, 7 npm packages) |

## Build from source

```bash
git clone https://github.com/telagod/nocode.git
cd nocode
cargo build --release
cp target/release/nocode ~/.local/bin/
```

Two-crate workspace:

- `crates/nocode-core/` — library (~28K LOC), all core logic
- `crates/nocode/` — TUI/CLI shell

```bash
cargo test                            # full test sweep
cargo clippy --all-targets -- -D warnings
cargo build --no-default-features -p nocode-core --features minimal  # minimal core
```

## Documentation

The docs are split into a numbered series — short, topic-locked, evolvable. **Start at [docs/README.md](docs/README.md)** for the index.

| | |
|---|---|
| [00_vision.md](docs/00_vision.md) | Why nocode exists, four invariants, closed-loop diagram |
| [01_realign.md](docs/01_realign.md) | The repositioning PRD (May 2026) |
| [02_architecture.md](docs/02_architecture.md) | Module map, provider/loop/storage layout |
| [03_skills.md](docs/03_skills.md) | First-class skill model |
| [04_policy_gates.md](docs/04_policy_gates.md) | Three-gate execution + why-trail |
| [05_fractal_subagents.md](docs/05_fractal_subagents.md) | Sub-agent inheritance |
| [06_observer.md](docs/06_observer.md) | Observability philosophy |
| [07_release.md](docs/07_release.md) | Release process and CI |
| [08_roadmap.md](docs/08_roadmap.md) | What's next |
| [10_provider_config.md](docs/10_provider_config.md) | Named-provider schema + `nocode init` / `config` |

## License

MIT — see [LICENSE](LICENSE).

<div align="center">

<sub>made with care, not configuration · v0.3.0</sub>

</div>
