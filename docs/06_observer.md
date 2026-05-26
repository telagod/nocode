# 06 · Observer (questions, not dashboards)

> Last updated: 2026-05-26 · Owner: product · Read with [08_roadmap](./08_roadmap.md)
>
> Inspired by [antcode](https://github.com/telagod/antcode)'s observer discipline.

## The principle

> A good observer doesn't try to show you everything. It tries to answer a specific question.

antcode codified this in its CLI: `show-policy`, `show-genomes`, `show-mutations`, `show-health`. Each command answers one question. There is no monolithic "dashboard view". nocode follows the same discipline.

## Questions worth answering

For a code agent, the high-value questions are:

1. **"What did I run lately?"** — recent sessions, with duration and tool count
2. **"Where am I hitting walls?"** — which gate denies most? what tool errors most?
3. **"Are my skills being used?"** — invoked-vs-discovered ratio per skill
4. **"What's it costing me?"** — tokens by provider/model/day
5. **"Is the harness behaving?"** — config snapshot, active sqlite volume

## Commands (target shape)

```
nocode --status                # quick health check (already exists; planned: enriched)
nocode insight                 # default summary of the last 7 days
nocode insight sessions        # answer question 1
nocode insight gates           # answer question 2
nocode insight skills          # answer question 3
nocode insight cost            # answer question 4
nocode insight where           # show active state dir + sqlite volume path (file-first)
```

Output is **plain columnar text**. `--json` flag flips to structured for scripting. No HTML, no TUI charts, no auto-refresh. If you want a dashboard, pipe to anything.

## Why no dashboard

Same reason antcode resisted the Observer Web View in its v0.3.2 positioning: UIs are sinks for engineering effort that should go into the agent itself. A tight CLI command is a one-day delivery; a dashboard is a one-quarter quagmire.

## Status

| Command           | Status | Source                          |
|-------------------|--------|---------------------------------|
| `--status`        | Live   | `crates/nocode/src/main.rs:498` |
| `insight`         | TODO   | will live in `crates/nocode/src/insight.rs` |
| `insight sessions`| TODO   | uses `storage::sql::list_sessions` |
| `insight gates`   | TODO   | needs gate-decision telemetry (see [08_roadmap](./08_roadmap.md)) |
| `insight skills`  | TODO   | needs skill-invocation telemetry |
| `insight cost`    | TODO   | uses `storage::sql::list_telemetry` |
| `insight where`   | TODO   | trivial, do first                |
