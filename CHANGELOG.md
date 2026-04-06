# Changelog

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
