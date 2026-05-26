# Phase 5 Polish — Lessons from antcode

> Generated: 2026-05-26 · Source: study of `~/project/antcode` v0.8.2

## What antcode does well (and we should borrow)

| antcode | Why it works | nocode borrow |
|---|---|---|
| Numbered docs `00_*` … `11_*` | Each topic owns one file; navigable; evolution-friendly | Reorganize `docs/` to numbered set |
| "Observer is questions, not dashboards" — show-policy / show-genomes / show-mutations | Resists UI sprawl; every command answers a *concrete question* | Add `nocode insight` family of focused subcommands |
| File-first state in `.antcode/*.jsonl` | Replayable, git-friendly, no DB to migrate | nocode already writes sqlite — surface the **path + content snapshot** in `--status` |
| Reward + retry workflow (`run-experiment`, `review-attempt`, `approve-attempt`, `rollback-attempt`) | Safe self-modification with review gates | We don't need self-evolution, but the *artifact + review* discipline maps to **session resume + branch/fork** |
| Architecture file makes the closed loop *visible* (Goal → ExperienceKey → Sampler → Pheromones) | One ASCII diagram > a thousand prose paragraphs | Add a closed-loop diagram to `PHILOSOPHY.md` |

## Specific deltas to land

### 5.A — Reorganize `docs/` into numbered set

```
docs/
  00_vision.md            ← was PHILOSOPHY.md (renamed for ordering)
  01_realign.md           ← was REALIGN.md
  02_architecture.md      ← was DESIGN.md (trimmed; load-bearing diagrams here)
  03_skills.md            ← new — first-class skill model
  04_policy_gates.md      ← new — three explainable gates
  05_fractal_subagents.md ← new — recursion contract
  06_observer.md          ← new — what to ask `nocode --insight` for
  07_release.md           ← was development notes (formalized)
  08_roadmap.md           ← what's next (post-REALIGN)
```

Each file is short and topic-locked, like antcode does. Top of each: one-line purpose + last-updated.

### 5.B — `nocode insight` subcommand family

Inspired by antcode's `show-policy / show-genomes / show-mutations`. Same discipline: **each subcommand answers one question**.

```
nocode insight                    # default: 7-day session/tool/cost summary
nocode insight sessions           # list recent sessions w/ duration, tool calls, model
nocode insight tools              # tool-call frequency + deny-trail breakdown
nocode insight skills             # which skills got invoked vs. discovered
nocode insight gates              # which gate denied the most? (permission/sandbox/hook/trust/plan)
nocode insight cost               # token usage by provider/model/day
```

Pure-text output, columnar. No JSON unless `--json`.

### 5.C — `--status` enhancements

Currently shows config snapshot. Add:
- Path to active sqlite volume (the file-first principle)
- Last-7-day session count
- Top 3 most-used tools
- Top 3 deny-gates (if any)

This makes `--status` itself a tiny dashboard without inventing one.

### 5.D — Closed-loop diagram in `PHILOSOPHY.md`

```text
User Prompt
    |
    v
+---------+      +---------+
| Skill   |--------> Prompt assembly  ----+
| Index   |                                |
+---------+                                v
                                      QueryEngine
                                          |
                                          v
                              +-> Schema -> Policy -> Hooks -+
                              |    (why-trail on every deny) |
                              |                              v
                              |                          ToolExecutor
                              |                              |
                              |          +------ Sub-agent (Agent tool)
                              |          |       (inherits PolicyMode + ToolRegistry + SkillIndex)
                              +-- recursive --+
```

Drop into `00_vision.md` (renamed PHILOSOPHY.md).

## Order of work

1. **5.D first** (small) — add diagram, costs nothing
2. **5.A** — `git mv` the docs, add new files (≤ 30 min)
3. **5.C** — extend `run_status` (data already in sql.rs)
4. **5.B** — `insight` family (most LOC; do last so 5.A/5.C land safely)

Each PR independent. Each shippable. No "big bang" redesign.

## What we explicitly *do not* steal

- **Strategy genome / pheromone / mutation engine.** That's antcode's value prop, not nocode's. nocode is a harness for *humans + agents writing code*, not a self-evolving meta-loop.
- **Reward engine.** Same reason.
- **Parent-child tournament / approve-attempt.** nocode already has session fork/resume; the evaluation gate is the human, not an automated reward.

We borrow antcode's **discipline of observability and file-first state**, not its self-evolution mechanics.
