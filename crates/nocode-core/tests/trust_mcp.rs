//! Integration test: trust system + MCP health checks.

use nocode_core::mcp::manager::McpManager;
use nocode_core::tool::trust::{
    AllowAllPolicy, ChainedResolver, PermissionEnforcer, RuleBasedPolicy, TrustContext,
    TrustDecision, TrustResolver,
};

// ---------------------------------------------------------------------------
// Trust system integration
// ---------------------------------------------------------------------------

#[test]
fn chained_resolver_deny_overrides_allow() {
    let chain = ChainedResolver::new()
        .push_resolver(Box::new(RuleBasedPolicy::new(
            vec!["safe".to_string()],
            vec!["blocked".to_string()],
        )))
        .push_resolver(Box::new(AllowAllPolicy));

    // "blocked" label → Deny (first resolver wins)
    let ctx = TrustContext::new("Bash", "model").with_label("blocked");
    assert_eq!(chain.resolve(&ctx), TrustDecision::Deny);

    // "safe" label → Allow (first resolver wins)
    let ctx = TrustContext::new("Read", "model").with_label("safe");
    assert_eq!(chain.resolve(&ctx), TrustDecision::Allow);

    // No labels → PromptUser from first, then Allow from second
    let ctx = TrustContext::new("Write", "model");
    assert_eq!(chain.resolve(&ctx), TrustDecision::Allow);
}

#[test]
fn permission_enforcer_with_labels() {
    let policy = RuleBasedPolicy::new(
        vec!["read-only".to_string()],
        vec!["destructive".to_string()],
    );
    let enforcer = PermissionEnforcer::new(Box::new(policy));

    assert_eq!(
        enforcer.check_with_labels("Read", "model", &["read-only"]),
        TrustDecision::Allow
    );
    assert_eq!(
        enforcer.check_with_labels("Bash", "model", &["destructive"]),
        TrustDecision::Deny
    );
    assert_eq!(
        enforcer.check_with_labels("Write", "model", &[]),
        TrustDecision::PromptUser
    );
}

#[test]
fn enforcer_allow_all_permits_everything() {
    let enforcer = PermissionEnforcer::allow_all();
    assert_eq!(enforcer.check("Bash", "model"), TrustDecision::Allow);
    assert_eq!(enforcer.check("Write", "model"), TrustDecision::Allow);
    assert_eq!(enforcer.check("rm_rf", "model"), TrustDecision::Allow);
}

#[test]
fn enforcer_prompt_required_always_prompts() {
    let enforcer = PermissionEnforcer::prompt_required();
    assert_eq!(enforcer.check("Read", "model"), TrustDecision::PromptUser);
    assert_eq!(enforcer.check("Bash", "model"), TrustDecision::PromptUser);
}

#[test]
fn rule_based_deny_takes_priority_over_allow() {
    // If a label matches both allow and deny, deny wins (checked first)
    let policy = RuleBasedPolicy::new(vec!["dual".to_string()], vec!["dual".to_string()]);
    let ctx = TrustContext::new("Tool", "model").with_label("dual");
    assert_eq!(policy.resolve(&ctx), TrustDecision::Deny);
}

#[test]
fn trust_context_multiple_labels() {
    let ctx = TrustContext::new("Bash", "model")
        .with_label("safe")
        .with_label("local");
    assert_eq!(ctx.labels.len(), 2);
    assert_eq!(ctx.tool_name, "Bash");
    assert_eq!(ctx.origin, "model");
}

// ---------------------------------------------------------------------------
// MCP Manager integration
// ---------------------------------------------------------------------------

#[test]
fn mcp_manager_register_and_list() {
    let mut mgr = McpManager::new();
    mgr.register_server(
        "github",
        "npx",
        vec!["-y".to_string(), "mcp-github".to_string()],
    );
    mgr.register_server(
        "slack",
        "npx",
        vec!["-y".to_string(), "mcp-slack".to_string()],
    );
    let servers = mgr.list_servers();
    assert_eq!(servers.len(), 2);
}

#[test]
fn mcp_manager_empty_has_no_tools() {
    let mgr = McpManager::new();
    assert!(mgr.all_tools().is_empty());
    assert!(mgr.list_servers().is_empty());
}

// ---------------------------------------------------------------------------
// Worker lifecycle states
// ---------------------------------------------------------------------------

#[test]
fn worker_lifecycle_states() {
    use nocode_core::agent::worker::{Worker, WorkerState};

    let mut w = Worker::new("w-1", "explorer", "find files");
    assert_eq!(w.state, WorkerState::Spawning);

    w.state = WorkerState::TrustRequired;
    assert_eq!(w.state, WorkerState::TrustRequired);

    w.state = WorkerState::ReadyForPrompt;
    assert_eq!(w.state, WorkerState::ReadyForPrompt);

    w.state = WorkerState::Running;
    assert_eq!(w.state, WorkerState::Running);

    w.state = WorkerState::Finished;
    assert_eq!(w.state, WorkerState::Finished);
}

#[test]
fn worker_registry_lifecycle() {
    use nocode_core::agent::worker::{WorkerRegistry, WorkerState};

    let mut reg = WorkerRegistry::new();
    let id = reg.register("test-worker", "do stuff");
    assert!(!id.is_empty());

    reg.set_state(&id, WorkerState::Running);
    assert_eq!(reg.get(&id).unwrap().state, WorkerState::Running);

    reg.set_result(&id, "done".to_string());
    assert_eq!(reg.get(&id).unwrap().state, WorkerState::Finished);

    let workers = reg.list();
    assert_eq!(workers.len(), 1);

    reg.remove(&id);
    assert!(reg.get(&id).is_none());
}
