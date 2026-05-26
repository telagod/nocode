# nocode PHILOSOPHY

> The smallest harness that lets a fractal code agent stay explainable.
> Bones (atomic tools) · Flesh (skills loaded as prompt) · Skin (thin gates).

> Last updated: 2026-05-26

---

## What nocode is

A terminal-native AI coding agent in Rust whose **value lives in its harness, not its tools**. It studies and borrows from pi-mono / oh-pi (TUI minimalism), codex (`SKILL.md` as executable contract), and Claude Code (loop shape, permission UX), then deliberately picks a different center: **harness engineering bionics**. Every layer must earn its keep; if it cannot explain itself, it is removed or compressed.

## The four invariants

1. **Skill is a first-class citizen, not a tool.** Skills are loaded into prompt assembly with the same priority as `CLAUDE.md` and `AGENTS.md`. The model sees the index *before* it acts; bodies are materialized lazily via the `Skill` tool. See [`skill::SkillRegistry`](../crates/nocode-core/src/skill/registry.rs).
2. **Minimum-viable tool surface.** The default registry exposes **11 atomic tools** with no overlap (see `ToolRegistry::core_tool_names()` in [`tool/mod.rs`](../crates/nocode-core/src/tool/mod.rs)). Everything else (Cron, Team, Notebook, Memory, MCP shim, …) is opt-in — an extension, not a default.
3. **Three gates, every refusal explains itself.** `Schema → Policy → Hooks`. The Policy gate ([`tool/policy.rs`](../crates/nocode-core/src/tool/policy.rs)) collapses what used to be six inline checks into one [`PolicyEngine::evaluate`] that returns a [`GateDecision`] carrying both the verdict *and a reason*. Every "Denied" message in the TUI is now a `[gate: reason]` why-trail.
4. **Fractal by construction.** Sub-agents share the parent's `ToolRegistry`, prompt assembly (so the skill index propagates), and — as of REALIGN — the parent's permission mode. Recursive trees inherit both capability and constraint. See [`tool/agent.rs::run_worker_thread`](../crates/nocode-core/src/tool/agent.rs).

## The closed loop (one picture)

```text
User Prompt
    │
    ▼
┌──────────────┐     ┌────────────────────────┐
│ SkillRegistry│────▶│  Prompt Assembly       │
│ (.nocode +   │     │  base + CLAUDE.md +    │
│  .claude     │     │  AGENTS.md + skill idx │
│  skills)     │     └────────────┬───────────┘
└──────────────┘                  │
                                  ▼
                             QueryEngine
                                  │
                                  ▼
              ┌─── Schema ──▶ Policy ──▶ Hooks ───┐
              │   (JSON      (trust+mode+         │
              │    schema)    classifier+         │
              │               sandbox; emits a    │
              │               why-trail on deny)  │
              │                                   ▼
              │                              ToolExecutor
              │                                   │
              │                  ┌────────────────┴────────────────┐
              │                  ▼                                 ▼
              │         Atomic tool (one of                Sub-agent (Agent tool)
              │         the 11 core, e.g.                  ───── inherits ─────
              │         Read, Bash, Skill, …)              ToolRegistry +
              │                                            SkillIndex +
              │                                            PermissionMode
              │                                                    │
              └──────────────── recursive ─────────────────────────┘
```

The whole product is this picture. Bones (tools) at the bottom. Skin (gates) in the middle. Flesh (skills) at the top, feeding the loop. The arrow back from sub-agent to the same `Schema → Policy → Hooks` chain is the fractal: a child runs the exact same harness as its parent, with the parent's mode bolted on.

## The 11 core tools

| Tool            | Purpose                                          |
|-----------------|--------------------------------------------------|
| FileRead        | observe a single file                            |
| FileWrite       | create / replace a file                          |
| FileEdit        | structured in-place edits                        |
| Glob            | path search                                      |
| Grep            | content search                                   |
| Bash            | execute shell commands                           |
| WebFetch        | retrieve URL content                             |
| WebSearch       | search the web                                   |
| Agent           | spawn a fractal sub-agent (recursive harness)    |
| AskUserQuestion | request structured human input                  |
| Skill           | invoke a registered skill                        |

The set is fixed — `tests/roadmap.rs::tool_registry_has_canonical_core_set` will fail any accidental shrink or sprawl.

## Why this isn't Claude Code parity

The earlier `DESIGN.md` aimed for "21 tools strict parity with Claude Code's `sdk-tools.d.ts`". That goal was retired in REALIGN. Parity is a path-dependent metric: if Claude Code adds NotebookEdit tomorrow, parity says you must add it; harness-bionics says *only if it earns its keep*. We pick the second discipline.

## Where the bones live

| Layer | File | Lines (LOC) | What changed in REALIGN |
|---|---|---|---|
| Skill registry | `crates/nocode-core/src/skill/registry.rs` | ~440 | new — first-class index + adaptive prompt budget |
| Skill tool wrapper | `crates/nocode-core/src/tool/skill.rs` | ~190 | shrunk to a thin wrapper over the registry |
| Policy gate | `crates/nocode-core/src/tool/policy.rs` | ~430 | new — unifies trust/permission/sandbox with explainable trail |
| Tool executor | `crates/nocode-core/src/tool/executor.rs` | ~470 (was ~700) | 6-stage pipeline collapsed to 3; old `check_permission` shimmed for back-compat |
| Prompt assembly | `crates/nocode-core/src/prompt/assembly.rs` | ~370 | 4th block: skill index with adaptive budget |
| Sub-agent | `crates/nocode-core/src/tool/agent.rs` | ~410 | propagates parent permission_mode to children |

## What we kept

The tooling we already had — the agentic loop in [`query/loop.rs`](../crates/nocode-core/src/query/loop.rs), the 4-provider trait, `ContentBlock` event model, `RuntimeConfig` 3-tier merging, the explicit-state-machine pattern — was the load-bearing structure. None of it changed in REALIGN. Ten of the eleven core tools predate this work; only `Skill` is new at the surface.

## Reading order

1. Start at this file.
2. Skim [`docs/REALIGN.md`](./REALIGN.md) for the why and the trail of decisions.
3. Read [`tool/policy.rs`](../crates/nocode-core/src/tool/policy.rs) — that's the harness gate, top to bottom.
4. Read [`skill/registry.rs`](../crates/nocode-core/src/skill/registry.rs) — that's how skills become first-class.
5. Then the rest of the codebase as needed.

## Non-goals

- Pixel-perfect parity with any other agent's UI or tool surface.
- Plugin marketplace, voice (deferred indefinitely).
- Embedded `/free` proxy.

## What "elegant" means here

Not "clever". Three things, in order:

1. **Each layer is one file you can read top to bottom.** No layer hides a hot path in a sub-sub-module. If it's important, it's not buried.
2. **Every refusal carries its reason.** The TUI does not say "Permission denied for tool 'X'"; it says `Denied [permission: read-only mode rejects tool classified Write]`. The user always knows why.
3. **Adding a feature shrinks the next adjacent feature** more often than it grows it. Skill-as-first-class let `tool/skill.rs` lose ~70 lines; PolicyEngine let `tool/executor.rs` lose ~145.

## Change log

### 2026-05-26 — REALIGN
- Skill becomes first-class (loaded into prompt assembly).
- 6-gate pipeline collapsed to 3 with explainable `GateDecision`.
- Default tool surface fixed at 11 core tools, contract enforced by integration test.
- Sub-agents inherit parent permission_mode (was: silently Auto).
- Adaptive skill-index budget (handles 60+ skills gracefully).
- Tests: 617 lib + 16 tool_roundtrip + 10 trust_mcp = **643/643 passing**.
