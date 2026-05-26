# nocode docs — index

| # | File | Purpose |
|---|---|---|
| 00 | [00_vision.md](./00_vision.md) | Why nocode exists, four invariants, closed-loop diagram |
| 01 | [01_realign.md](./01_realign.md) | The repositioning PRD (May 2026) — what was retired and why |
| 02 | [02_architecture.md](./02_architecture.md) | Module map, provider/loop/storage layout |
| 03 | [03_skills.md](./03_skills.md) | First-class skill model (registry + prompt index + budget) |
| 04 | [04_policy_gates.md](./04_policy_gates.md) | Three-gate execution with explainable why-trail |
| 05 | [05_fractal_subagents.md](./05_fractal_subagents.md) | How `Agent` spawns share parent harness |
| 06 | [06_observer.md](./06_observer.md) | Observability philosophy — questions, not dashboards |
| 07 | [07_release.md](./07_release.md) | Release process and CI workflow |
| 08 | [08_roadmap.md](./08_roadmap.md) | What's next (Phase 5+ polish backlog) |
| 09 | [09_legacy_alignment.md](./09_legacy_alignment.md) | Archived comparison work; kept for context only |
| 10 | [10_provider_config.md](./10_provider_config.md) | Named-provider table, profiles, `nocode init` / `config` CLI |

## Reading order

For a **new contributor**: 00 → 04 → 03 → 05 → 02 → 06.
For a **product/PRD reviewer**: 00 → 01 → 08 → 06.
For **release engineering**: 07 → 02.

Each numbered file is short and topic-locked. If a topic outgrows one file, fork it (e.g. `04a_*.md`) rather than turning a doc into a monolith.
