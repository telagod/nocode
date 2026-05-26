# 03 · Skills (first-class)

> Last updated: 2026-05-26 · Owner: harness · Read with [00_vision](./00_vision.md), [02_architecture](./02_architecture.md)

## What a skill is

A skill is a markdown file under one of:

```
{cwd}/.nocode/skills/{name}.md       # project
{cwd}/.claude/skills/{name}.md       # project, compat
~/.nocode/skills/{name}.md           # user-global
~/.claude/skills/{name}.md           # user-global, compat
```

Sub-directories give one level of namespacing: `git/commit.md` becomes the skill named `git:commit`.

## Frontmatter (all optional)

```yaml
---
name: commit-and-push                # default: file stem (or ns:stem)
description: Stage, commit, push.    # default: first non-heading line, capped at 200 chars
triggers:                            # for future routing — currently informational
  - commit
  - git push
---
```

Body is the prompt material. `$ARGUMENTS` / `${ARGUMENTS}` are substituted on invocation.

## Why first-class

A wrapper-only `Skill` tool (the old design) made the model invoke skills by *guessing* the name. The current design loads the **index** (name + description, no bodies) into the system prompt next to `CLAUDE.md`, so the model picks from a visible menu. Only the chosen skill's body is materialized via the `Skill` tool — token cost stays linear in *use*, not in *inventory*.

## Adaptive budget

The index is rendered by `SkillRegistry::prompt_index_with_budget(Some(max_chars))`. Entries are sorted by description length ascending (densest first); overflow is trimmed with a `_...and N more_` footer. Default budget: `TruncationBudget::max_skill_index_chars = 4_000`.

## Where it lives

| File | Role |
|---|---|
| `crates/nocode-core/src/skill/mod.rs` | Module root, re-exports |
| `crates/nocode-core/src/skill/registry.rs` | `SkillDef`, `SkillRegistry`, frontmatter parser, adaptive trim |
| `crates/nocode-core/src/tool/skill.rs` | `SkillTool` — thin wrapper that calls `SkillRegistry::load(cwd)` and renders `def.body` |
| `crates/nocode-core/src/prompt/assembly.rs` | Injects `prompt_index_with_budget` as the 4th system block |

## Tests that pin the contract

- `skill::registry::tests::prompt_index_adaptive_trims_to_budget`
- `skill::registry::tests::namespaced_subdir_becomes_ns_colon_name`
- `prompt::assembly::tests::skill_index_appears_when_skills_exist`
- `prompt::assembly::tests::skill_index_absent_when_no_skills`
