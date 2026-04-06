//! Wave B integration tests — TrustResolver, PermissionPrompter, SessionControl, MCP Health.

// ---------------------------------------------------------------------------
// Trust system integration
// ---------------------------------------------------------------------------

use nocode_core::worker_boot::{
    AllowAllPolicy, PromptRequiredPolicy, RuleBasedPolicy, TrustChain, TrustContext, TrustDecision,
    TrustPolicy, TrustResolver, Worker, WorkerEventKind, WorkerFailureKind, WorkerRegistry,
    WorkerStatus,
};

#[test]
fn trust_chain_integration_allow_all_skips_prompt() {
    let chain = TrustChain::new()
        .with(PromptRequiredPolicy)
        .with(AllowAllPolicy);
    let mut w = Worker::new("int-1");
    w.require_trust();
    let ctx = TrustContext::new("int-1", "local");
    let d = w.resolve_trust_with(&chain, &ctx);
    assert_eq!(d, TrustDecision::Allow);
    assert_eq!(w.status, WorkerStatus::ReadyForPrompt);
}

#[test]
fn trust_chain_integration_deny_blocks_worker() {
    let deny_rule = RuleBasedPolicy::new(vec![], vec!["untrusted".into()]);
    let chain = TrustChain::new().with(deny_rule);
    let mut w = Worker::new("int-2");
    w.require_trust();
    let ctx = TrustContext::new("int-2", "remote").with_label("untrusted");
    let d = w.resolve_trust_with(&chain, &ctx);
    assert_eq!(d, TrustDecision::Deny);
    assert_eq!(w.status, WorkerStatus::Failed);
    assert_eq!(
        w.failure.as_ref().unwrap().kind,
        WorkerFailureKind::TrustGate
    );
}

#[test]
fn trust_policy_enum_roundtrip() {
    let policies = vec![
        TrustPolicy::AllowAll,
        TrustPolicy::PromptRequired,
        TrustPolicy::RuleBased {
            allow_labels: vec!["safe".into()],
            deny_labels: vec!["evil".into()],
        },
    ];
    let ctx_safe = TrustContext::new("w", "t").with_label("safe");
    let ctx_evil = TrustContext::new("w", "t").with_label("evil");
    let ctx_unknown = TrustContext::new("w", "t").with_label("other");

    let resolvers: Vec<Box<dyn nocode_core::worker_boot::TrustResolver>> =
        policies.into_iter().map(|p| p.into_resolver()).collect();

    assert_eq!(resolvers[0].resolve(&ctx_safe), TrustDecision::Allow);
    assert_eq!(resolvers[1].resolve(&ctx_safe), TrustDecision::PromptUser);
    assert_eq!(resolvers[2].resolve(&ctx_safe), TrustDecision::Allow);
    assert_eq!(resolvers[2].resolve(&ctx_evil), TrustDecision::Deny);
    assert_eq!(
        resolvers[2].resolve(&ctx_unknown),
        TrustDecision::PromptUser
    );
}

#[test]
fn worker_full_lifecycle_with_trust_chain() {
    let chain = TrustChain::new().with(AllowAllPolicy);
    let mut w = Worker::new("lifecycle-1");
    assert_eq!(w.status, WorkerStatus::Spawning);

    w.require_trust();
    assert_eq!(w.status, WorkerStatus::TrustRequired);

    let ctx = TrustContext::new("lifecycle-1", "local");
    w.resolve_trust_with(&chain, &ctx);
    assert_eq!(w.status, WorkerStatus::ReadyForPrompt);

    w.start_running();
    assert_eq!(w.status, WorkerStatus::Running);

    w.finish();
    assert_eq!(w.status, WorkerStatus::Finished);

    // Verify event trail
    let kinds: Vec<_> = w.events.iter().map(|e| e.kind.clone()).collect();
    assert_eq!(
        kinds,
        vec![
            WorkerEventKind::Spawning,
            WorkerEventKind::TrustRequired,
            WorkerEventKind::TrustResolved,
            WorkerEventKind::Running,
            WorkerEventKind::Finished,
        ]
    );
}

#[test]
fn worker_registry_lifecycle() {
    let mut reg = WorkerRegistry::new();
    reg.create("r1");
    reg.create("r2");
    reg.create("r3");
    assert_eq!(reg.list().len(), 3);

    if let Some(w) = reg.get_mut("r2") {
        w.mark_ready();
        w.start_running();
        w.finish();
    }
    assert_eq!(reg.get("r2").unwrap().status, WorkerStatus::Finished);

    let removed = reg.remove("r3").unwrap();
    assert_eq!(removed.id, "r3");
    assert_eq!(reg.list().len(), 2);
    assert!(reg.get("r3").is_none());
}

// ---------------------------------------------------------------------------
// PermissionPrompter integration
// ---------------------------------------------------------------------------

use nocode_core::tool_execution::{
    AutoApprovePrompter, AutoDenyPrompter, InteractivePrompter, PermissionAuditLog,
    PermissionPrompter, ToolPermissionDecision,
};

#[test]
fn permission_prompter_auto_approve_all_tools() {
    let p = AutoApprovePrompter;
    let tools = [
        "Read", "Write", "Edit", "Bash", "Glob", "Grep", "Agent", "WebFetch",
    ];
    for tool in &tools {
        let d = p.check(tool, "");
        assert!(
            matches!(d, ToolPermissionDecision::Allow { .. }),
            "auto-approve should allow {tool}"
        );
    }
}

#[test]
fn permission_prompter_auto_deny_all_tools() {
    let p = AutoDenyPrompter::new();
    let tools = ["Read", "Write", "Edit", "Bash"];
    for tool in &tools {
        let d = p.check(tool, "");
        assert!(
            matches!(d, ToolPermissionDecision::Deny { .. }),
            "auto-deny should deny {tool}"
        );
    }
}

#[test]
fn permission_prompter_interactive_classification() {
    let p = InteractivePrompter::default_dangerous();
    // Dangerous tools → Prompt
    let dangerous = [
        "Bash",
        "Write",
        "Edit",
        "WebFetch",
        "WebSearch",
        "Agent",
        "TeamCreate",
        "TeamDelete",
        "CronCreate",
        "CronDelete",
    ];
    for tool in &dangerous {
        let d = p.check(tool, "");
        assert!(
            matches!(d, ToolPermissionDecision::Prompt { .. }),
            "{tool} should require prompt"
        );
    }
    // Safe tools → Allow
    let safe = ["Read", "Glob", "Grep", "TaskGet", "TaskList", "MemoryList"];
    for tool in &safe {
        let d = p.check(tool, "");
        assert!(
            matches!(d, ToolPermissionDecision::Allow { .. }),
            "{tool} should be allowed"
        );
    }
}

#[test]
fn permission_audit_log_integration() {
    let prompters: Vec<Box<dyn PermissionPrompter>> = vec![
        Box::new(AutoApprovePrompter),
        Box::new(InteractivePrompter::default_dangerous()),
    ];
    let mut log = PermissionAuditLog::default();

    for p in &prompters {
        let d = p.check("Bash", "command=ls");
        let label = match &d {
            ToolPermissionDecision::Allow { .. } => "allow",
            ToolPermissionDecision::Deny { .. } => "deny",
            ToolPermissionDecision::Prompt { .. } => "prompt",
        };
        log.record("Bash", label, p.name());
    }

    assert_eq!(log.len(), 2);
    assert_eq!(log.entries[0].decision, "allow");
    assert_eq!(log.entries[0].prompter, "auto-approve");
    assert_eq!(log.entries[1].decision, "prompt");
    assert_eq!(log.entries[1].prompter, "interactive");
}

// ---------------------------------------------------------------------------
// SessionControl integration
// ---------------------------------------------------------------------------

use nocode_core::session_control::{SessionControl, SessionStatus};
use nocode_core::session_persistence::SessionIdentity;

fn make_id(id: &str) -> SessionIdentity {
    SessionIdentity::new(id, "/tmp/wave-b-test")
}

#[test]
fn session_control_full_lifecycle() {
    let mut sc = SessionControl::new();
    sc.register(&make_id("main"));
    sc.update_message_count("main", 10);

    // Checkpoint at message 5
    sc.checkpoint("main", 5, Some("mid-point")).unwrap();

    // Fork from checkpoint
    let fork_id = sc.fork("main", 5, Some("experiment")).unwrap();
    assert_eq!(sc.get("main").unwrap().status, SessionStatus::Forked);
    assert_eq!(sc.get(&fork_id).unwrap().status, SessionStatus::Active);
    assert_eq!(sc.get(&fork_id).unwrap().message_count, 5);

    // Work on fork
    sc.update_message_count(&fork_id, 8);
    assert_eq!(sc.get(&fork_id).unwrap().message_count, 8);

    // Complete fork
    sc.complete(&fork_id).unwrap();
    assert_eq!(sc.get(&fork_id).unwrap().status, SessionStatus::Completed);

    // Resume parent
    sc.resume("main").unwrap();
    assert_eq!(sc.get("main").unwrap().status, SessionStatus::Active);
}

#[test]
fn session_control_multiple_forks() {
    let mut sc = SessionControl::new();
    sc.register(&make_id("root"));
    sc.checkpoint("root", 2, None).unwrap();
    sc.checkpoint("root", 5, None).unwrap();

    let f1 = sc.fork("root", 2, Some("branch-a")).unwrap();
    sc.resume("root").unwrap();
    let f2 = sc.fork("root", 5, Some("branch-b")).unwrap();

    let branches = sc.list_branches("root");
    assert_eq!(branches.len(), 2);

    assert_ne!(f1, f2);
    assert_eq!(
        sc.get(&f1).unwrap().branch_name.as_deref(),
        Some("branch-a")
    );
    assert_eq!(
        sc.get(&f2).unwrap().branch_name.as_deref(),
        Some("branch-b")
    );
}

#[test]
fn session_control_suspend_resume_cycle() {
    let mut sc = SessionControl::new();
    sc.register(&make_id("s1"));

    sc.suspend("s1").unwrap();
    assert_eq!(sc.get("s1").unwrap().status, SessionStatus::Suspended);

    sc.resume("s1").unwrap();
    assert_eq!(sc.get("s1").unwrap().status, SessionStatus::Active);

    sc.suspend("s1").unwrap();
    sc.resume("s1").unwrap();
    assert_eq!(sc.get("s1").unwrap().status, SessionStatus::Active);
}

#[test]
fn session_control_list_all() {
    let mut sc = SessionControl::new();
    sc.register(&make_id("a"));
    sc.register(&make_id("b"));
    sc.register(&make_id("c"));
    assert_eq!(sc.list_all().len(), 3);
}

#[test]
fn session_control_error_paths() {
    let mut sc = SessionControl::new();

    // Operations on nonexistent sessions
    assert!(sc.suspend("ghost").is_err());
    assert!(sc.resume("ghost").is_err());
    assert!(sc.complete("ghost").is_err());
    assert!(sc.checkpoint("ghost", 0, None).is_err());
    assert!(sc.fork("ghost", 0, None).is_err());
    assert!(sc.get("ghost").is_none());
}

// ---------------------------------------------------------------------------
// MCP Health integration
// ---------------------------------------------------------------------------

use nocode_core::mcp_manager::{McpManager, McpServerStatus};

#[test]
fn mcp_health_check_lifecycle() {
    let mut mgr = McpManager::new();
    mgr.register_server("test-srv", "fake-binary", vec![]);

    // Set to Connected (no real client) to test health check path
    // Access internal state via the public API
    assert_eq!(
        mgr.get_status("test-srv"),
        Some(McpServerStatus::Disconnected)
    );

    // Health check on disconnected should error
    assert!(mgr.health_check("test-srv").is_err());

    // Health stats should be zero
    let h = mgr.get_health("test-srv").unwrap();
    assert_eq!(h.checks_total, 0);
    assert_eq!(h.consecutive_failures, 0);
}

#[test]
fn mcp_reconnect_disconnected_tries_connect() {
    let mut mgr = McpManager::new();
    mgr.register_server("srv", "__nonexistent_binary_wave_b__", vec![]);

    // Reconnect from Disconnected should attempt connect
    let err = mgr.reconnect("srv").unwrap_err();
    assert!(err.contains("failed to spawn"));
    assert_eq!(mgr.get_status("srv"), Some(McpServerStatus::Failed));
}

#[test]
fn mcp_reconnect_unregistered_fails() {
    let mut mgr = McpManager::new();
    let err = mgr.reconnect("ghost").unwrap_err();
    assert!(err.contains("not registered"));
}

#[test]
fn mcp_health_check_all_empty() {
    let mut mgr = McpManager::new();
    mgr.register_server("a", "cmd-a", vec![]);
    // All disconnected — health_check_all should return empty
    let results = mgr.health_check_all();
    assert!(results.is_empty());
}

#[test]
fn mcp_manager_register_disconnect_cycle() {
    let mut mgr = McpManager::new();
    mgr.register_server("s1", "cmd", vec!["--arg".into()]);
    assert_eq!(mgr.get_status("s1"), Some(McpServerStatus::Disconnected));
    assert_eq!(mgr.list_servers().len(), 1);

    mgr.disconnect("s1");
    assert_eq!(mgr.get_status("s1"), Some(McpServerStatus::Disconnected));

    // Disconnect nonexistent — no panic
    mgr.disconnect("nonexistent");
}

#[test]
fn mcp_find_tool_requires_connected() {
    let mut mgr = McpManager::new();
    mgr.register_server("s1", "cmd", vec![]);
    // No tools registered, no connected servers
    assert!(mgr.find_tool("anything").is_none());
}

// ---------------------------------------------------------------------------
// Recovery system integration
// ---------------------------------------------------------------------------

use nocode_core::recovery::{
    EscalationPolicy, FailureScenario, RecoveryContext, RecoveryResult, recipe_for,
};

#[test]
fn recovery_all_scenarios_have_recipes() {
    let scenarios = [
        FailureScenario::TrustPromptUnresolved,
        FailureScenario::PromptMisdelivery,
        FailureScenario::StaleBranch,
        FailureScenario::CompileRedCrossCrate,
        FailureScenario::McpHandshakeFailure,
        FailureScenario::PartialPluginStartup,
        FailureScenario::ProviderFailure,
    ];
    for scenario in &scenarios {
        let recipe = recipe_for(*scenario);
        assert!(
            !recipe.steps.is_empty(),
            "recipe for {scenario:?} should have steps"
        );
        assert!(recipe.max_attempts > 0);
    }
}

#[test]
fn recovery_context_tracks_attempts() {
    let mut ctx = RecoveryContext::new();
    // TrustPromptUnresolved has max_attempts = 1
    let r1 = ctx.attempt_recovery(FailureScenario::TrustPromptUnresolved);
    assert_eq!(r1, RecoveryResult::Recovered);

    let r2 = ctx.attempt_recovery(FailureScenario::TrustPromptUnresolved);
    assert_eq!(
        r2,
        RecoveryResult::EscalationRequired {
            policy: EscalationPolicy::AlertHuman,
        }
    );
}

#[test]
fn recovery_independent_scenarios() {
    let mut ctx = RecoveryContext::new();

    let r1 = ctx.attempt_recovery(FailureScenario::ProviderFailure);
    assert_eq!(r1, RecoveryResult::Recovered);

    // Different scenario — fresh attempt
    let r2 = ctx.attempt_recovery(FailureScenario::McpHandshakeFailure);
    assert_eq!(r2, RecoveryResult::Recovered);
}

#[test]
fn recovery_events_recorded() {
    let mut ctx = RecoveryContext::new();
    assert!(ctx.events().is_empty());
    ctx.attempt_recovery(FailureScenario::StaleBranch);
    assert!(ctx.events().len() >= 2);
}

#[test]
fn recovery_provider_failure_allows_multiple_attempts() {
    let mut ctx = RecoveryContext::new();
    // ProviderFailure has max_attempts = 3
    for _ in 0..3 {
        let r = ctx.attempt_recovery(FailureScenario::ProviderFailure);
        assert_eq!(r, RecoveryResult::Recovered);
    }
    // 4th attempt should escalate
    let r = ctx.attempt_recovery(FailureScenario::ProviderFailure);
    assert_eq!(
        r,
        RecoveryResult::EscalationRequired {
            policy: EscalationPolicy::Abort,
        }
    );
}

// ---------------------------------------------------------------------------
// Policy engine integration
// ---------------------------------------------------------------------------

use nocode_core::policy_engine::{
    DiffScope, LaneBlocker, LaneContext, PolicyAction, PolicyCondition, PolicyEngine, PolicyRule,
    ReviewStatus,
};
use std::time::Duration;

fn base_lane_ctx() -> LaneContext {
    LaneContext {
        lane_id: "test-lane".into(),
        green_level: 0,
        branch_freshness: Duration::from_secs(0),
        blocker: LaneBlocker::None,
        review_status: ReviewStatus::Pending,
        diff_scope: DiffScope::Full,
        completed: false,
        reconciled: false,
    }
}

#[test]
fn policy_engine_priority_ordering() {
    let rules = vec![
        PolicyRule {
            name: "low-priority".into(),
            priority: 10,
            condition: PolicyCondition::GreenAt { level: 1 },
            action: PolicyAction::MergeToDev,
        },
        PolicyRule {
            name: "high-priority".into(),
            priority: 1,
            condition: PolicyCondition::GreenAt { level: 1 },
            action: PolicyAction::Escalate,
        },
    ];
    let engine = PolicyEngine::new(rules);

    let mut ctx = base_lane_ctx();
    ctx.green_level = 2;
    let actions = engine.evaluate(&ctx);
    assert!(!actions.is_empty());
    assert_eq!(*actions[0], PolicyAction::Escalate);
}

#[test]
fn policy_engine_no_match_returns_empty() {
    let rules = vec![PolicyRule {
        name: "needs-high-green".into(),
        priority: 1,
        condition: PolicyCondition::GreenAt { level: 10 },
        action: PolicyAction::MergeToDev,
    }];
    let engine = PolicyEngine::new(rules);

    let ctx = base_lane_ctx();
    let actions = engine.evaluate(&ctx);
    assert!(actions.is_empty());
}

#[test]
fn policy_engine_and_condition() {
    let rules = vec![PolicyRule {
        name: "both".into(),
        priority: 1,
        condition: PolicyCondition::And(vec![
            PolicyCondition::GreenAt { level: 1 },
            PolicyCondition::StaleBranch,
        ]),
        action: PolicyAction::MergeForward,
    }];
    let engine = PolicyEngine::new(rules);

    // Green but not stale → no match
    let mut ctx = base_lane_ctx();
    ctx.green_level = 5;
    ctx.branch_freshness = Duration::from_secs(100);
    assert!(engine.evaluate(&ctx).is_empty());

    // Green and stale → match
    ctx.branch_freshness = Duration::from_secs(7200);
    let actions = engine.evaluate(&ctx);
    assert_eq!(actions.len(), 1);
    assert_eq!(*actions[0], PolicyAction::MergeForward);
}

#[test]
fn policy_engine_or_condition() {
    let rules = vec![PolicyRule {
        name: "either".into(),
        priority: 1,
        condition: PolicyCondition::Or(vec![
            PolicyCondition::LaneCompleted,
            PolicyCondition::StaleBranch,
        ]),
        action: PolicyAction::RecoverOnce,
    }];
    let engine = PolicyEngine::new(rules);

    // Not completed, not stale → no match
    let mut ctx = base_lane_ctx();
    assert!(engine.evaluate(&ctx).is_empty());

    // Stale → match via Or
    ctx.branch_freshness = Duration::from_secs(7200);
    let actions = engine.evaluate(&ctx);
    assert_eq!(actions.len(), 1);
}

#[test]
fn policy_engine_review_passed() {
    let rules = vec![PolicyRule {
        name: "review-gate".into(),
        priority: 1,
        condition: PolicyCondition::ReviewPassed,
        action: PolicyAction::MergeToDev,
    }];
    let engine = PolicyEngine::new(rules);

    let mut ctx = base_lane_ctx();
    assert!(engine.evaluate(&ctx).is_empty());

    ctx.review_status = ReviewStatus::Approved;
    assert_eq!(engine.evaluate(&ctx).len(), 1);
}

#[test]
fn policy_engine_chain_action() {
    let rules = vec![PolicyRule {
        name: "chain".into(),
        priority: 1,
        condition: PolicyCondition::LaneCompleted,
        action: PolicyAction::Chain(vec![PolicyAction::MergeToDev, PolicyAction::CleanupSession]),
    }];
    let engine = PolicyEngine::new(rules);

    let mut ctx = base_lane_ctx();
    ctx.completed = true;
    let actions = engine.evaluate(&ctx);
    assert_eq!(actions.len(), 2);
    assert_eq!(*actions[0], PolicyAction::MergeToDev);
    assert_eq!(*actions[1], PolicyAction::CleanupSession);
}

// ---------------------------------------------------------------------------
// Permission enforcer integration
// ---------------------------------------------------------------------------

use nocode_core::permission_enforcer::{
    PermissionCheckResult, check_tool_permission, check_workspace_write,
};
use nocode_core::tool_registry::PermissionMode;

#[test]
fn permission_enforcer_readonly_tools() {
    let readonly_tools = [
        "Read",
        "Glob",
        "Grep",
        "WebFetch",
        "TaskGet",
        "TaskList",
        "CronList",
        "ToolSearch",
        "Lsp",
        "MemoryList",
        "MemorySearch",
    ];
    for tool in &readonly_tools {
        let r = check_tool_permission(tool, PermissionMode::ReadOnly);
        assert_eq!(
            r,
            PermissionCheckResult::Allowed,
            "{tool} should be allowed in ReadOnly"
        );
    }
}

#[test]
fn permission_enforcer_write_tools_denied_in_readonly() {
    let write_tools = [
        "Edit",
        "Write",
        "Bash",
        "Agent",
        "TaskUpdate",
        "TeamCreate",
        "CronCreate",
        "MemorySave",
        "MemoryDelete",
    ];
    for tool in &write_tools {
        let r = check_tool_permission(tool, PermissionMode::ReadOnly);
        assert!(
            matches!(r, PermissionCheckResult::Denied { .. }),
            "{tool} should be denied in ReadOnly"
        );
    }
}

#[test]
fn permission_enforcer_workspace_write_allows_all_base_tools() {
    let all_tools = [
        "Read",
        "Edit",
        "Write",
        "Bash",
        "Glob",
        "Grep",
        "Agent",
        "WebFetch",
        "WebSearch",
        "TaskGet",
        "TaskList",
        "TaskUpdate",
        "TaskStop",
        "TaskOutput",
        "TeamCreate",
        "TeamDelete",
        "CronCreate",
        "CronDelete",
        "CronList",
        "ToolSearch",
        "Lsp",
        "MemorySave",
        "MemoryList",
        "MemorySearch",
        "MemoryDelete",
    ];
    for tool in &all_tools {
        let r = check_tool_permission(tool, PermissionMode::WorkspaceWrite);
        assert_eq!(
            r,
            PermissionCheckResult::Allowed,
            "{tool} should be allowed in WorkspaceWrite"
        );
    }
}

#[test]
fn permission_enforcer_mcp_tools() {
    let r = check_tool_permission("mcp:server:read_file", PermissionMode::WorkspaceWrite);
    assert_eq!(r, PermissionCheckResult::Allowed);

    let r = check_tool_permission("mcp:server:read_file", PermissionMode::ReadOnly);
    assert!(matches!(r, PermissionCheckResult::Denied { .. }));
}

#[test]
fn permission_enforcer_workspace_write_boundary() {
    let r = check_workspace_write("src/main.rs", "/tmp");
    assert_eq!(r, PermissionCheckResult::Allowed);

    let r = check_workspace_write("/etc/shadow", "/tmp");
    assert!(matches!(r, PermissionCheckResult::Denied { .. }));
}

// ---------------------------------------------------------------------------
// Plugin system integration
// ---------------------------------------------------------------------------

use nocode_core::plugin_system::{
    HookEvent, Plugin, PluginKind, PluginMetadata, PluginRegistry, PluginState,
};

fn make_plugin(id: &str) -> Plugin {
    Plugin {
        metadata: PluginMetadata {
            id: id.into(),
            name: format!("Plugin {id}"),
            version: "1.0.0".into(),
            description: format!("Test plugin {id}"),
            kind: PluginKind::External,
            default_enabled: true,
        },
        state: PluginState::Unconfigured,
        tools: Vec::new(),
        hooks: Vec::new(),
    }
}

#[test]
fn plugin_registry_lifecycle() {
    let mut reg = PluginRegistry::new();
    reg.register(make_plugin("test-plugin")).unwrap();

    let p = reg.get("test-plugin").unwrap();
    assert_eq!(p.state, PluginState::Validated); // register auto-validates

    reg.start("test-plugin").unwrap();
    assert_eq!(reg.get("test-plugin").unwrap().state, PluginState::Healthy);
    assert_eq!(reg.list_healthy().len(), 1);
}

#[test]
fn plugin_registry_duplicate_rejected() {
    let mut reg = PluginRegistry::new();
    reg.register(make_plugin("dup")).unwrap();
    assert!(reg.register(make_plugin("dup")).is_err());
}

#[test]
fn plugin_registry_empty_id_rejected() {
    let mut reg = PluginRegistry::new();
    let mut p = make_plugin("");
    p.metadata.id = String::new();
    assert!(reg.register(p).is_err());
}

#[test]
fn plugin_hook_dispatch() {
    let mut reg = PluginRegistry::new();
    let mut p = make_plugin("hook-test");
    p.hooks = vec![HookEvent::PreToolUse];
    reg.register(p).unwrap();
    reg.start("hook-test").unwrap();

    let results = reg.run_hook(HookEvent::PreToolUse, "Read");
    // Hook runs but no actual handler — should return results without panic
    let _ = results;
}

#[test]
fn plugin_stop_and_restart() {
    let mut reg = PluginRegistry::new();
    reg.register(make_plugin("restart-test")).unwrap();
    reg.start("restart-test").unwrap();
    assert_eq!(reg.get("restart-test").unwrap().state, PluginState::Healthy);

    reg.stop("restart-test").unwrap();
    assert_eq!(reg.get("restart-test").unwrap().state, PluginState::Stopped);
    assert_eq!(reg.list_healthy().len(), 0);

    // Restart
    reg.start("restart-test").unwrap();
    assert_eq!(reg.get("restart-test").unwrap().state, PluginState::Healthy);
}

// ---------------------------------------------------------------------------
// Global registry integration
// ---------------------------------------------------------------------------

use nocode_core::global_registry::{GlobalToolRegistry, RegisteredTool, ToolSource};

fn make_reg_tool(name: &str, source: ToolSource, desc: &str) -> RegisteredTool {
    RegisteredTool {
        name: name.into(),
        source,
        description: desc.into(),
        schema: serde_json::json!({}),
    }
}

#[test]
fn global_registry_register_and_search() {
    let mut reg = GlobalToolRegistry::new();
    reg.register(make_reg_tool(
        "Read",
        ToolSource::Base,
        "Read files from disk",
    ));
    reg.register(make_reg_tool(
        "Edit",
        ToolSource::Base,
        "Edit files on disk",
    ));
    reg.register(make_reg_tool(
        "mcp:fs:read",
        ToolSource::Mcp,
        "MCP file read",
    ));

    let results = reg.search("read");
    assert!(results.len() >= 2);

    let base_tools = reg.list_by_source(ToolSource::Base);
    assert!(base_tools.len() >= 2);

    let mcp_tools = reg.list_by_source(ToolSource::Mcp);
    assert_eq!(mcp_tools.len(), 1);
}

#[test]
fn global_registry_unregister() {
    let mut reg = GlobalToolRegistry::new();
    reg.register(make_reg_tool("TempTool", ToolSource::Runtime, "Temporary"));
    assert_eq!(reg.search("TempTool").len(), 1);

    reg.unregister("TempTool");
    assert!(reg.search("TempTool").is_empty());
}

#[test]
fn global_registry_empty_search() {
    let reg = GlobalToolRegistry::new();
    assert!(reg.search("nonexistent").is_empty());
    assert!(reg.list_by_source(ToolSource::Base).is_empty());
}

// ---------------------------------------------------------------------------
// Session compaction integration
// ---------------------------------------------------------------------------

use nocode_core::message::QueryMessage;
use nocode_core::query_deps::Compactor;
use nocode_core::session_compaction::{
    CompactionConfig, RichCompactor, estimate_message_tokens, should_compact,
};

#[test]
fn compaction_under_threshold_passes_through() {
    let compactor = RichCompactor::new(CompactionConfig::default());
    let messages = vec![
        QueryMessage::system("system prompt"),
        QueryMessage::user("hello"),
        QueryMessage::assistant("hi there"),
    ];
    let result = compactor.compact(&messages);
    // Under threshold — should pass through unchanged
    assert!(result.len() >= 3);
}

#[test]
fn estimate_tokens_basic() {
    let messages = vec![
        QueryMessage::user("hello world"),
        QueryMessage::assistant("hi"),
    ];
    let tokens = estimate_message_tokens(&messages);
    assert!(tokens > 0);
}

#[test]
fn should_compact_under_threshold() {
    let messages = vec![QueryMessage::user("short")];
    let config = CompactionConfig::default();
    assert!(!should_compact(&messages, &config));
}

#[test]
fn should_compact_over_threshold() {
    // Create enough messages to exceed default threshold (10_000 tokens ≈ 40_000 chars)
    let mut messages = vec![QueryMessage::system("system")];
    let padding = "x".repeat(200);
    for i in 0..200 {
        messages.push(QueryMessage::user(format!("message {i} {padding}")));
        messages.push(QueryMessage::assistant(format!("response {i} {padding}")));
    }
    let config = CompactionConfig::default();
    assert!(should_compact(&messages, &config));
}

// ---------------------------------------------------------------------------
// Additional coverage — cross-module integration
// ---------------------------------------------------------------------------

#[test]
fn trust_chain_with_rule_based_and_prompt_fallback() {
    let rule = RuleBasedPolicy::new(vec!["internal".into()], vec!["banned".into()]);
    let chain = TrustChain::new().with(rule).with(PromptRequiredPolicy);

    // internal label → Allow via rule
    let ctx = TrustContext::new("w1", "test").with_label("internal");
    assert_eq!(chain.resolve(&ctx), TrustDecision::Allow);

    // banned label → Deny via rule
    let ctx = TrustContext::new("w2", "test").with_label("banned");
    assert_eq!(chain.resolve(&ctx), TrustDecision::Deny);

    // unknown label → PromptUser from rule, then PromptUser from fallback
    let ctx = TrustContext::new("w3", "test").with_label("unknown");
    assert_eq!(chain.resolve(&ctx), TrustDecision::PromptUser);
}

#[test]
fn compaction_preserves_system_messages() {
    let compactor = RichCompactor::new(CompactionConfig {
        preserve_recent_messages: 2,
        max_estimated_tokens: 10_000,
    });
    let messages = vec![
        QueryMessage::system("important system prompt"),
        QueryMessage::user("hello"),
        QueryMessage::assistant("hi"),
    ];
    let result = compactor.compact(&messages);
    // Under threshold — all preserved
    assert!(
        result
            .iter()
            .any(|m| m.content.contains("important system prompt"))
    );
}

#[test]
fn estimate_tokens_empty() {
    assert_eq!(estimate_message_tokens(&[]), 0);
}

#[test]
fn session_control_update_message_count() {
    let mut sc = SessionControl::new();
    sc.register(&make_id("mc1"));
    assert_eq!(sc.get("mc1").unwrap().message_count, 0);

    sc.update_message_count("mc1", 42);
    assert_eq!(sc.get("mc1").unwrap().message_count, 42);

    // Update nonexistent — no panic
    sc.update_message_count("ghost", 99);
}

#[test]
fn global_registry_overwrite_same_name() {
    let mut reg = GlobalToolRegistry::new();
    reg.register(make_reg_tool("Overwrite", ToolSource::Base, "v1"));
    reg.register(make_reg_tool("Overwrite", ToolSource::Plugin, "v2"));
    assert_eq!(reg.count(), 1);
    let t = reg.get("Overwrite").unwrap();
    assert_eq!(t.source, ToolSource::Plugin);
    assert_eq!(t.description, "v2");
}

#[test]
fn policy_engine_timed_out_condition() {
    let rules = vec![PolicyRule {
        name: "timeout".into(),
        priority: 1,
        condition: PolicyCondition::TimedOut {
            duration: Duration::from_secs(600),
        },
        action: PolicyAction::Escalate,
    }];
    let engine = PolicyEngine::new(rules);

    let mut ctx = base_lane_ctx();
    ctx.branch_freshness = Duration::from_secs(300);
    assert!(engine.evaluate(&ctx).is_empty());

    ctx.branch_freshness = Duration::from_secs(700);
    assert_eq!(engine.evaluate(&ctx).len(), 1);
}

#[test]
fn policy_condition_scoped_diff() {
    let cond = PolicyCondition::ScopedDiff;
    let mut ctx = base_lane_ctx();
    assert!(!cond.matches(&ctx));
    ctx.diff_scope = DiffScope::Scoped;
    assert!(cond.matches(&ctx));
}

#[test]
fn policy_condition_startup_blocked() {
    let cond = PolicyCondition::StartupBlocked;
    let mut ctx = base_lane_ctx();
    assert!(!cond.matches(&ctx));
    ctx.blocker = LaneBlocker::Startup;
    assert!(cond.matches(&ctx));
}
