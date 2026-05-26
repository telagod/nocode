# 10 · Provider Configuration

> Last updated: 2026-05-26 · Owner: harness · Read with [00_vision](./00_vision.md), [06_observer](./06_observer.md)
>
> Inspired by [codex](https://github.com/openai/codex)'s named-provider table.

## TL;DR

Edit `~/.nocode/config.toml`. No TUI wizard, no `--login` flag, no `nocode config set` for everything — just a TOML file that you read top-to-bottom and edit by hand. Run `nocode init` once to scaffold a commented template.

## Schema (post-REALIGN)

```toml
# Default provider — name of a [providers.<name>] table below, or one of the
# builtins: claude / openai / gemini.
default_provider = "subfox"

# Default model — overridable per-provider (default_model) and per-call (--model).
model = "gpt-5.5"

# permission_mode: auto | ask | deny | read-only
permission_mode = "ask"

# ----- Providers -----
[providers.subfox]
base_url     = "https://sub.foxnio.com/v1"
wire_api     = "openai-responses"      # anthropic | openai-responses | openai-chat | google
api_key_env  = "OPENAI_API_KEY"        # env var holding the key — explicit, no fallback chain
default_model = "gpt-5.5"

[providers.local-vllm]
base_url     = "http://localhost:8000/v1"
wire_api     = "openai-chat"
api_key_env  = "VLLM_API_KEY"
default_model = "Qwen2.5-Coder-32B-Instruct"

# ----- Profiles -----
[profiles.work]
provider = "subfox"
model    = "gpt-5.5"
permission_mode = "ask"

[profiles.home]
provider = "local-vllm"
permission_mode = "auto"
```

## Precedence

For each value, the chain is run **left to right**. First non-empty rung wins.

| Value | Precedence chain |
|---|---|
| Provider name | `--provider <name>` → `NOCODE_PROVIDER` → active profile's `provider` → `default_provider` → `model_provider` builtin alias |
| Model | `NOCODE_MODEL` → `ANTHROPIC_MODEL`/`OPENAI_MODEL`/`GEMINI_MODEL` → active profile's `model` → top-level `model` → provider's `default_model` |
| Permission mode | `--permission-mode <m>` → active profile's `permission_mode` → top-level `permission_mode` → builtin default (`ask`) |
| API key | `[providers.<name>].api_key_env` (env var lookup) → fallback by wire_api (`ANTHROPIC_API_KEY` / `OPENAI_API_KEY` / `GEMINI_API_KEY`) |

## CLI

```
nocode init                      # scaffold ~/.nocode/config.toml (no-op if file exists)
nocode init --force              # overwrite existing file
nocode config list               # print current config
nocode config list --json        # print as JSON
nocode config get <key>          # print one scalar (model / default_provider / ...)
nocode config set <key> <val>    # write one scalar (whitelisted keys only)
nocode config unset <key>        # remove one scalar

nocode --provider <name>         # one-shot override of default_provider
nocode --profile <name>          # apply a named profile from [profiles.<name>]
nocode --model <name>            # one-shot model override
```

### Scalar keys mutable from `nocode config set`

`default_provider`, `model`, `permission_mode`, `max_turns`, `max_tokens`, `reasoning_effort`, `system_prompt`, `telemetry_enabled`.

`providers`, `profiles`, `mcp_servers`, `hooks`, `sandbox` are tables — edit them in the TOML file. CLI mutation of tables would be lossy (comments, formatting) and is intentionally not supported.

## Builtin aliases

Three names work out of the box without a `[providers.*]` table — convenient for "I just have one key":

| Alias | Wire | Default base_url | Key env |
|---|---|---|---|
| `claude` / `anthropic` | `anthropic` | `https://api.anthropic.com` (or `$ANTHROPIC_BASE_URL`) | `ANTHROPIC_API_KEY` |
| `openai` | `openai-responses` | `https://api.openai.com` (or `$OPENAI_BASE_URL`) | `OPENAI_API_KEY` |
| `gemini` / `google` | `google` | `https://generativelanguage.googleapis.com` | `GEMINI_API_KEY` |

```bash
export OPENAI_API_KEY=sk-...
nocode --provider openai   # works, no config file needed
```

## Migration from `custom_*` (pre-REALIGN)

The old single-slot scheme is **rejected at load time** with a hard error. If you see:

```
Error: Your ~/.nocode/config.toml uses the deprecated `custom_*` scheme...
```

…replace those keys with a named provider:

```diff
- model_provider     = "custom"
- custom_base_url    = "https://sub.foxnio.com"
- custom_api_format  = "openai-responses"
- custom_preset      = "OpenAI"
- model              = "gpt-5.5"
+ default_provider = "subfox"
+ model            = "gpt-5.5"
+
+ [providers.subfox]
+ base_url    = "https://sub.foxnio.com/v1"
+ wire_api    = "openai-responses"
+ api_key_env = "OPENAI_API_KEY"
```

The wizards (`--login`, automatic onboarding flow) have been removed; their replacement is `nocode init`.

## Why no wizard

A TUI wizard sounds friendlier but encodes a specific *flow* into UI state — every time the schema gains a field, the wizard must be edited or it lies. A TOML file with a commented template:

- has a canonical place to look (the file)
- composes naturally with `git`, `chezmoi`, secret-store substitution
- never falls behind the schema — `nocode init` re-scaffolds when you need it
- mirrors codex's mental model so people switching tools have one less surprise

Trade-off: first-time users have to read a file instead of pressing arrows. That's the right trade for a code agent's user base.
