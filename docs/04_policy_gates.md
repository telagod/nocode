# 04 · Policy Gates (three layers, every refusal explains itself)

> Last updated: 2026-05-26 · Owner: harness · Read with [00_vision](./00_vision.md)

## The three gates

```text
ToolExecutor::execute_tool_use
    │
    ├─ 1. Schema     ── JSON-schema validation (tool/tool_validation.rs)
    ├─ 2. Policy     ── PolicyEngine::evaluate  (tool/policy.rs)
    │                   collapses: trust + plan_mode + permission_mode
    │                              + risk classifier + sandbox path/net
    │   returns GateDecision { gate, reason, [remember] }
    │
    ├─ 3. Hooks      ── PreToolUse external commands (tool/hook_runner.rs)
    │                   non-zero exit denies
    ▼
  execute → snapshot → file-history → PostHooks → ContentBlock
```

Bash-syntax level checks (`rm -rf /`, `:(){ :|:& };:` etc.) run *inside* the Policy gate via the risk classifier (`ToolClassifier::classify_bash` → `bash_validation::is_destructive_command`).

## The why-trail contract

Every `GateDecision::Deny` carries a non-empty `reason`. The executor surfaces it as:

```
Denied [<gate>: <reason>]
```

The TUI then re-colors this trail (see `tui_widgets::parse_why_trail`): `Denied` and brackets in **error**, gate name in **warning + bold**, reason in **dim**. Users triage the cause at a glance — no need to read the whole sentence.

Five gate names, in priority order:

| Gate name      | When it fires                                          |
|----------------|--------------------------------------------------------|
| `plan-mode`    | Plan mode active and tool is not read-only             |
| `trust`        | `PermissionEnforcer` resolver returned `Deny`          |
| `permission`   | `PermissionMode` + risk classifier disagree           |
| `prompter`     | Interactive prompter returned `Deny`                   |
| `sandbox`      | Path/network policy violation                          |

(`hook` is technically a fourth top-level gate at the executor level, not from `PolicyEngine` — its trail still uses the same `Denied [hook: …]` format.)

## Why three, not six

The pre-REALIGN executor inlined six checks (trust → hooks → permission → bash → sandbox → execute). Each returned a bare boolean or a custom string. The conceptual surface was opaque: a denial said "Permission denied for tool 'X'" with no clue *which* layer decided. `PolicyEngine` is purely additive — it shrank `executor.rs` from ~700 LOC to ~470 because the per-call logic became one method call.

## Authoritative sources

| Concept | File |
|---|---|
| `GateDecision`, `GateName`, `PolicyEngine` | `crates/nocode-core/src/tool/policy.rs` |
| Risk classifier (`ToolClassifier::classify`) | `crates/nocode-core/src/tool/permission.rs` |
| Destructive-pattern authority (Bash) | `crates/nocode-core/src/tool/bash_validation.rs` (`is_destructive_command` + `DESTRUCTIVE_PATTERNS_CI`) |
| TUI rendering of the why-trail | `crates/nocode/src/tui_widgets.rs` (`parse_why_trail`) |

If a new bash pattern needs to be flagged destructive, add it to `bash_validation::DESTRUCTIVE_PATTERNS` (or `_CI` if case-insensitive). Do **not** maintain a second list anywhere else.
