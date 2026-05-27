# nocode docs

The numbered series. Each file is short, topic-locked, and evolvable.
If a topic outgrows one file, fork it as `0Na_*.md` rather than turning
a doc into a monolith.

| # | File | One-line purpose |
|---|---|---|
| 00 | [00_vision.md](./00_vision.md) | Why nocode exists, four invariants, closed-loop diagram |
| 01 | [01_realign.md](./01_realign.md) | The repositioning PRD (May 2026) — what was retired and why |
| 02 | [02_architecture.md](./02_architecture.md) | Module map, provider / loop / storage layout |
| 03 | [03_skills.md](./03_skills.md) | First-class skill model (registry + prompt index + budget) |
| 04 | [04_policy_gates.md](./04_policy_gates.md) | Three-gate execution with explainable why-trail |
| 05 | [05_fractal_subagents.md](./05_fractal_subagents.md) | How `Agent` spawns share parent harness |
| 06 | [06_observer.md](./06_observer.md) | Observability philosophy — questions, not dashboards |
| 07 | [07_release.md](./07_release.md) | Versioning, CI, releases, NPM token, dev workflow |
| 08 | [08_roadmap.md](./08_roadmap.md) | What's next (Phase 5+ polish backlog) |
| 09 | [09_legacy_alignment.md](./09_legacy_alignment.md) | **Archived** — pre-REALIGN parity audit |
| 10 | [10_provider_config.md](./10_provider_config.md) | Named-provider tables, profiles, `nocode init` / `config` |

## Reading paths

**New user — "I just want to install and run it":** [README](../README.md) → `nocode init` → done. Come back to docs only when something surprises you.

**New contributor — "I'm going to read the code":** [00](./00_vision.md) → [04](./04_policy_gates.md) → [03](./03_skills.md) → [05](./05_fractal_subagents.md) → [02](./02_architecture.md). The first four set the mental model; the fifth is the module map you'll keep open in another tab.

**Product / PRD reviewer — "What is this for, and where's it going?":** [00](./00_vision.md) → [01](./01_realign.md) → [08](./08_roadmap.md) → [06](./06_observer.md). The first three are the *story*; the fourth is how to *measure* it.

**Release engineer — "How does the sausage get made?":** [07](./07_release.md) → [02](./02_architecture.md). The first is the runbook; the second is what's in the box.

**Migrating from 0.2.x:** [CHANGELOG](../CHANGELOG.md) → [10](./10_provider_config.md). The first has the diff; the second has the schema.

## Quick find

| Want to know… | Read |
|---|---|
| Why are there 11 tools? | [00_vision.md](./00_vision.md) §4 invariants |
| What does `Denied [policy: ...]` mean? | [04_policy_gates.md](./04_policy_gates.md) |
| Where do my skill files live? | [03_skills.md](./03_skills.md) |
| How do I migrate from `custom_*`? | [10_provider_config.md](./10_provider_config.md) §Migration · [CHANGELOG](../CHANGELOG.md) |
| What did v0.3.0 break? | [CHANGELOG](../CHANGELOG.md) → "v0.3.0 — REALIGN release" |
| How does a sub-agent inherit permissions? | [05_fractal_subagents.md](./05_fractal_subagents.md) §Mode propagation |
| What does `nocode insight` show? | [06_observer.md](./06_observer.md) |
| How do I cut a release? | [07_release.md](./07_release.md) §Cutting a release |
| Why no `--login` wizard? | [10_provider_config.md](./10_provider_config.md) §Why no wizard · [01_realign.md](./01_realign.md) |
| What's in the storage layer? | [02_architecture.md](./02_architecture.md) §Storage |

## Landing site

The public-facing landing page source is at [`landing/index.html`](./landing/index.html). It's a single self-contained HTML file, deployed to GitHub Pages by [`.github/workflows/pages.yml`](../.github/workflows/pages.yml) on every push that touches `docs/landing/**`. Live at <https://telagod.github.io/nocode/>.

If you're editing the landing, keep the contract: **zero JS framework, zero font fetch, single file**. The whole point is that one click of "view source" reveals the entire site.
