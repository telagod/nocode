# Changelog

## [0.3.0] - 2026-05-26 — REALIGN release

> ⚠ **Breaking release.** This is the cut-over from "Claude Code parity" to
> "harness engineering bionics". Most of the breaking changes ship with an
> actionable migration message at runtime; the rest are documented below.

### Highlights

- **Skill is first-class.** `SkillRegistry` discovers `*.md` files in
  `.nocode/skills/` and `.claude/skills/` (project + user-global) and injects a
  name+description index into prompt assembly — the model sees what's
  available before it acts. Bodies are materialized lazily via the `Skill`
  tool; the index is adaptively trimmed to `max_skill_index_chars` (default
  4 KB). See `docs/03_skills.md`.
- **Three explainable gates.** The old 6-step `validation → trust → hook →
  permission → bash → sandbox` pipeline collapses into `Schema → Policy →
  Hooks`. The new `PolicyEngine` returns a `GateDecision { gate, reason }`,
  and every TUI refusal is now rendered as `Denied [<gate>: <reason>]` with
  the gate name colorized. See `docs/04_policy_gates.md`.
- **Fractal sub-agents inherit parent mode.** `Agent` spawns now thread
  `permission_mode` from the parent — pre-REALIGN every sub-agent silently ran
  in `Auto` regardless. See `docs/05_fractal_subagents.md`.
- **Named-provider tables replace `custom_*`.** codex-style
  `[providers.<name>]` with `base_url`, `wire_api`, `api_key_env`,
  `default_model`. Multiple endpoints can coexist; `--provider <name>` or
  `--profile <name>` selects at runtime. See `docs/10_provider_config.md`.
- **`nocode insight` + enriched `--status`.** antcode-inspired observability
  CLI — questions, not dashboards. `nocode insight where|sessions|tools|gates|cost`.
  See `docs/06_observer.md`.

### Added

- `crates/nocode-core/src/skill/{mod,registry}.rs` — `SkillRegistry` with
  YAML frontmatter parser, adaptive prompt-index budget, namespaced subdirs.
- `crates/nocode-core/src/tool/policy.rs` — `PolicyEngine`, `GateDecision`,
  `GateName` — the unified, explainable gate.
- `crates/nocode-core/src/config/settings.rs` — `ProviderDef`, `ProfileDef`,
  `Settings.providers: BTreeMap<String, ProviderDef>`, `Settings.profiles`,
  `Settings.default_provider`, `Settings.reject_legacy_custom()`.
- `crates/nocode-core/src/provider/resolve.rs` — `ResolvedProvider`,
  `resolve_named_provider()` with full precedence chain.
- `crates/nocode/src/init.rs` — `nocode init` scaffold for
  `~/.nocode/config.toml`.
- `crates/nocode/src/config_cli.rs` — `nocode config list|get|set|unset`.
- `crates/nocode/src/insight.rs` — `nocode insight summary|where|sessions|
  tools|gates|cost` (plain text or `--json`).
- `crates/nocode/src/tui_widgets.rs::parse_why_trail` + colored rendering for
  `Denied [<gate>: <reason>]` lines.
- `docs/` — full 11-file numbered series: `00_vision` through
  `10_provider_config` + `README.md` index.
- `--provider <name>`, `--profile <name>` CLI flags. `NOCODE_PROVIDER`,
  `NOCODE_PROFILE` env vars.

### Changed

- `ToolRegistry::with_defaults()` now registers **11 core tools** (was 9):
  added `AskUserQuestion` and `Skill`. Asserted by
  `tests/roadmap.rs::tool_registry_has_canonical_core_set`.
- `nocode-core/src/tool/executor.rs` collapsed from ~700 → ~470 LOC via
  `PolicyEngine` delegation; old `check_permission`, `check_sandbox`,
  `is_read_only_tool` removed.
- `nocode-core/src/tool/skill.rs` rewritten as a thin wrapper over
  `SkillRegistry`; gained `SkillTool::with_cwd()` for hermetic testing.
- `prompt::assembly` adds a 4th system block (skill index after CLAUDE.md /
  AGENTS.md); `TruncationBudget` gains `max_skill_index_chars` (default 4_000).
- `bash_validation::is_destructive_command` becomes the **single authoritative
  source** for destructive-pattern detection; new `DESTRUCTIVE_PATTERNS_CI`
  for case-insensitive matches (`DROP TABLE`, `TRUNCATE TABLE`, …).
- `AgentTool` schema's `mode` field is now load-bearing — see breaking
  changes.
- TUI `Denied` lines render with `gate name` in **bold warning**, reason in
  **dim**, brackets in **error** color.
- CLAUDE.md, docs reorganized into numbered series.

### Removed (breaking)

- **`Settings.custom_base_url` / `custom_api_format` / `custom_preset` /
  `model_provider = "custom"`** — all rejected at load time with a single
  actionable migration message pointing at `[providers.<name>]`.
- **`NOCODE_CUSTOM_BASE_URL` / `NOCODE_CUSTOM_API_FORMAT`** env vars — use
  `NOCODE_PROVIDER` + the named-provider table.
- **`nocode --login`** flag and the entire `crates/nocode/src/login.rs`
  interactive wizard (−1128 LOC). Replaced by `nocode init` + editing the
  TOML by hand. `nocode --login` now prints a migration message and exits 2.
- **`resolve_custom_api_key`, `resolve_custom_base_url`,
  `resolve_custom_api_format`, `lookup_preset_env_key`, `PRESET_ENV_KEYS`**
  removed from `provider/resolve.rs`. `resolve_api_key()` kept as
  `#[deprecated]` shim.
- **`RuntimeConfig.custom_base_url` / `custom_api_format`** — no longer
  threaded through; sub-agents and main both use `resolve_named_provider`.

### Fixed

- Sub-agents now inherit the parent's `permission_mode` when the `Agent`
  tool's `mode` argument is provided. Previously every spawn silently ran in
  `Auto` — this was the load-bearing safety hole.
- `team_create_and_delete_roundtrip` and other tests that touch `$HOME` now
  serialize through `test_support::env_mutex` to avoid races with the
  skill-index tests.
- TUI legacy `Permission denied for tool 'X'` strings replaced with structured
  `Denied [<gate>: <reason>]` trails so users can triage cause at a glance.

### Migration from v0.2.x

If your `~/.nocode/config.toml` looks like this:

```toml
model_provider = "custom"
custom_base_url = "https://sub.foxnio.com"
custom_api_format = "openai-responses"
custom_preset = "OpenAI"
model = "gpt-5.5"
```

Replace it with:

```toml
default_provider = "subfox"
model = "gpt-5.5"

[providers.subfox]
base_url    = "https://sub.foxnio.com/v1"
wire_api    = "openai-responses"        # anthropic | openai-responses | openai-chat | google
api_key_env = "OPENAI_API_KEY"          # env var holding the key
default_model = "gpt-5.5"
```

Run `nocode init` to scaffold a fresh commented template (won't overwrite
existing files). The exit-code-2 error message at startup includes the same
migration block. See `docs/10_provider_config.md` for the full schema.

### Stats

- **820+ tests passing**: 160 TUI + 616 lib + 12 mock + 11 roadmap + 16
  tool_roundtrip + 10 trust_mcp.
- **clippy `-D warnings` clean** across `--all-targets`.
- **Net diff vs v0.2.55**: −1301 LOC (deletes 2558, adds 1257), 26 files
  changed, 4 docs deleted, 12 docs added, `login.rs` (−1128) and
  `executor.rs` (−145 LOC) the largest shrinks.

---

## [0.1.5] - 2026-04-06

### Added

#### Wave A — Stub Realization + Permission Enforcer
- `permission_enforcer.rs` — PermissionEnforcer with per-tool allow/deny/prompt rules, wildcard patterns, cascading policy resolution
- `tool_execution/executor.rs` — DefaultToolExecutor with full dispatch, hook integration, sandbox validation, permission checks
- `tool_execution/model.rs` — ToolCallInput/ToolCallResult/ToolExecutionTrace with timing, ToolCommandOutput, DeniedTrace
- `stale_branch.rs` — BranchFreshness detection (Fresh/Stale/Diverged), 4 policies (WarnOnly/Block/AutoRebase/AutoMergeForward), configurable thresholds
- `lsp_client.rs` — Enhanced grep-based LSP: diagnostics (bracket balance + long lines + TODO/FIXME), hover with doc comments, definition/references via pure Rust helpers, completion with file symbols + keywords, symbol extraction
- 14 tool integration tests (`tests/tool_integration.rs`)

#### Wave B — TrustResolver, PermissionPrompter, SessionControl, MCP Health
- `worker_boot.rs` — TrustResolver trait + TrustPolicy enum (AllowAll/PromptRequired/RuleBased), TrustDecision, TrustContext, TrustChain, Worker::resolve_trust_with()
- `session_control.rs` — SessionControl state machine (Idle/Active/Paused/Resuming/Draining/Terminated), pause/resume/drain lifecycle, SessionControlEvent audit trail
- `mcp_manager.rs` — McpLifecyclePhase (11 phases), McpLifecycleTracker with validated transitions, McpHealthChecker with configurable thresholds, health status reporting
- `recovery.rs` — 7 FailureScenario types, RecoveryRecipe (steps + max_attempts + escalation), RecoveryContext with attempt tracking, one-attempt-before-escalation
- `policy_engine.rs` — PolicyCondition (GreenAt/StaleBranch/And/Or), PolicyAction (MergeToDev/Escalate/Chain), priority-sorted rule evaluation, LaneContext/DiffScope/ReviewStatus
- 56 Wave B integration tests (`tests/wave_b_integration.rs`)

### Changed
- Module count: 51 → 55
- Test count: 555 → 750
- LOC: ~38K → ~44K

## [0.1.4] - 2026-04-05

### Added
- MCP server mode (`--mcp-server`) with JSON-RPC stdio transport
- REPL tab completion and multiline input support
- IDE server mode (`--ide-server`) with JSON-RPC stdio server

## [0.1.3] - 2026-04-04

### Added
- Windows ARM64 support (aarch64-pc-windows-msvc)

## [0.1.2] - 2026-04-03

### Added
- Linux ARM64 support
- CI test/lint pipeline
- npm README auto-sync

### Changed
- Standardized provider names to company names (anthropic/openai/google)

## [0.1.1] - 2026-04-02

### Added
- LLM-based session compaction with RichCompactor fallback
- 8 core module test harnesses
