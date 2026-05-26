# 05 · Fractal Sub-agents

> Last updated: 2026-05-26 · Owner: harness · Read with [00_vision](./00_vision.md), [04_policy_gates](./04_policy_gates.md)

## The invariant

A sub-agent runs the **same** harness as its parent. Not "similar", not "subset" — same. The mechanism is `tool/agent.rs::run_worker_thread`, which builds:

| Component | How it's obtained | Inherited from parent? |
|---|---|---|
| `Provider` | `build_worker_provider(resolve_worker_provider(settings))` | Yes (same `Settings::load_merged`) |
| `ToolRegistry` | `ToolRegistry::with_defaults(cwd)` | Yes (same 11 core tools, see [03_skills](./03_skills.md)) |
| `SkillRegistry` (via prompt assembly) | `assembly::assemble_system_prompt(cwd, …)` | Yes (skill index appears) |
| `PolicyEngine` config | Default `Auto` **unless** the `mode` arg is set | Explicit per call |

## Mode propagation (the load-bearing line)

```rust
// crates/nocode-core/src/tool/agent.rs
let executor = match permission_mode_override {
    Some(mode) => ToolExecutor::new(&tool_registry).with_permission_mode(mode),
    None       => ToolExecutor::new(&tool_registry),
};
```

When the parent passes `mode: "plan"` to `Agent`, the child runs `PermissionMode::ReadOnly`. Before this fix, every sub-agent silently ran `Auto` regardless of parent caution — the fractal was broken at the safety axis.

| `mode` string         | Sub-agent `PermissionMode` |
|-----------------------|----------------------------|
| `acceptEdits`         | `Auto`                     |
| `bypassPermissions`   | `Auto`                     |
| `dontAsk`             | `Auto`                     |
| `plan`                | `ReadOnly`                 |
| `default` or omitted  | inherit (workspace default)|

## What's not yet fractal

- **Trust enforcer** is not threaded through; sub-agents always start with no `PermissionEnforcer`. If a parent wants to restrict label-based trust, it must be configured in `Settings`. *Future work, tracked in [08_roadmap](./08_roadmap.md).*
- **Sandbox config** likewise not auto-propagated; sub-agent reads its own `SandboxConfig` from `settings.sandbox`. This is acceptable for now because the file-system root is shared anyway.

## Test that pins the contract

`tool::agent::tests::parse_subagent_mode_maps_strings_to_permission_modes` —
every string → `PermissionMode` mapping is asserted here. Add a row to that test before adding a new `mode` value to the schema.
