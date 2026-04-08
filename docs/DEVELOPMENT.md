# nocode Development Guide

## Build & Test

```bash
cargo build                                     # debug build
cargo build --release                           # release build
cargo test                                      # all tests (~476)
cargo test -p nocode-core                       # core library only
cargo test -p nocode                            # CLI binary only
cargo test <test_name>                          # single test by name
cargo clippy --all-targets -- -D warnings       # lint
cargo fmt --check                               # format check
cargo fmt                                       # auto-format
```

## Workspace Layout

Two-crate Cargo workspace (edition 2024, clippy all+pedantic+nursery, unsafe forbidden):

```
crates/nocode-core/   — library (~30K LOC, 51 modules), all core logic
crates/nocode/        — binary (~8K LOC), CLI/REPL/TUI shell
```

Dependencies: serde, serde_json, reqwest, jsonschema, rusqlite (bundled), chrono, pulldown-cmark, syntect, crossterm.

## Architecture

### Provider Layer
- `provider.rs` — ModelProvider enum (Mock|Claude|OpenAi|Gemini|Custom), ApiFormat routing
- `provider_transport.rs` — HTTP client, SSE parsing, retry/backoff, per-provider auth

### Query Engine
- `query_engine.rs` — conversation lifecycle, tool schema gen, 9-state loop
- `query_loop.rs` — QueryLoopRunner state machine, budget, stop hooks
- `query_deps.rs` — DI with trait objects (CallModel, Compactor, ToolRunner)
- `query_config.rs` — query configuration with tool definitions

### Tool System (25 tools)
- `tool_execution/executor.rs` — dispatch with hook/sandbox/validation
- `tool_registry.rs` — PermissionMode (ReadOnly/WorkspaceWrite/DangerFullAccess)
- `tool_validation.rs` — JSON Schema validation for all tool inputs
- `bash_validation.rs` — 6 validation submodules
- `file_safety.rs` — symlink escape prevention, binary detection, 10MB limit

### Storage
- `sql_store.rs` — rusqlite with date-based volume partitioning
- `memory_store.rs` — MemoryEntry with YAML frontmatter, file-system CRUD
- `session_persistence.rs` — JSONL session/transcript/history persistence

### TUI
- `tui.rs` — 4-pane fullscreen with RGB rendering, overlay system
- `markdown_render.rs` — pulldown-cmark + syntect integration
- `status_hud.rs` — token/cost/elapsed/model/session HUD

## Key Conventions

- Workspace edition 2024. Clippy all+pedantic+nursery. Unsafe forbidden.
- Global registries use `OnceLock<Arc<Mutex<T>>>` singleton pattern.
- State machines over inferred state: worker, MCP, plugin lifecycles use explicit enum states.
- One recovery attempt before escalation — never silently retry indefinitely.
- Memory entries use Markdown with YAML frontmatter (name/description/type).

## Release

Tag-triggered via GitHub Actions:

```bash
# bump version in Cargo.toml, commit, then:
git tag v0.x.x
git push origin main --tags
```

CI builds 3 platform binaries, creates GitHub Release, and publishes to npm via Trusted Publishers.
