use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustDecision {
    Allow,
    Deny,
    PromptUser,
}

/// Context passed to trust resolvers for policy evaluation.
#[derive(Debug, Clone)]
pub struct TrustContext {
    pub worker_id: String,
    pub origin: String,
    pub labels: Vec<String>,
}

impl TrustContext {
    pub fn new(worker_id: &str, origin: &str) -> Self {
        Self {
            worker_id: worker_id.to_string(),
            origin: origin.to_string(),
            labels: Vec::new(),
        }
    }

    pub fn with_label(mut self, label: &str) -> Self {
        self.labels.push(label.to_string());
        self
    }
}

/// Trait for trust decision engines.
pub trait TrustResolver: Send + Sync {
    fn resolve(&self, ctx: &TrustContext) -> TrustDecision;
}

/// Always allows — for local/CTF/test environments.
#[derive(Debug, Clone)]
pub struct AllowAllPolicy;

impl TrustResolver for AllowAllPolicy {
    fn resolve(&self, _ctx: &TrustContext) -> TrustDecision {
        TrustDecision::Allow
    }
}

/// Always requires user prompt — for untrusted origins.
#[derive(Debug, Clone)]
pub struct PromptRequiredPolicy;

impl TrustResolver for PromptRequiredPolicy {
    fn resolve(&self, _ctx: &TrustContext) -> TrustDecision {
        TrustDecision::PromptUser
    }
}

/// Rule-based policy: labels matching allow list → Allow, deny list → Deny, else → PromptUser.
#[derive(Debug, Clone)]
pub struct RuleBasedPolicy {
    pub allow_labels: Vec<String>,
    pub deny_labels: Vec<String>,
}

impl RuleBasedPolicy {
    pub fn new(allow_labels: Vec<String>, deny_labels: Vec<String>) -> Self {
        Self {
            allow_labels,
            deny_labels,
        }
    }
}

impl TrustResolver for RuleBasedPolicy {
    fn resolve(&self, ctx: &TrustContext) -> TrustDecision {
        for label in &ctx.labels {
            if self.deny_labels.iter().any(|d| d == label) {
                return TrustDecision::Deny;
            }
        }
        for label in &ctx.labels {
            if self.allow_labels.iter().any(|a| a == label) {
                return TrustDecision::Allow;
            }
        }
        TrustDecision::PromptUser
    }
}

/// Selects which policy to use.
#[derive(Debug, Clone)]
pub enum TrustPolicy {
    AllowAll,
    PromptRequired,
    RuleBased {
        allow_labels: Vec<String>,
        deny_labels: Vec<String>,
    },
}

impl TrustPolicy {
    pub fn into_resolver(self) -> Box<dyn TrustResolver> {
        match self {
            Self::AllowAll => Box::new(AllowAllPolicy),
            Self::PromptRequired => Box::new(PromptRequiredPolicy),
            Self::RuleBased {
                allow_labels,
                deny_labels,
            } => Box::new(RuleBasedPolicy::new(allow_labels, deny_labels)),
        }
    }
}

/// Chain of trust resolvers evaluated in order.
/// First definitive result (Allow/Deny) wins. If all return PromptUser,
/// the chain returns PromptUser. Empty chain returns Allow.
pub struct TrustChain {
    resolvers: Vec<Box<dyn TrustResolver>>,
}

impl TrustChain {
    pub fn new() -> Self {
        Self {
            resolvers: Vec::new(),
        }
    }

    pub fn with(mut self, resolver: impl TrustResolver + 'static) -> Self {
        self.resolvers.push(Box::new(resolver));
        self
    }

    pub fn push(&mut self, resolver: impl TrustResolver + 'static) {
        self.resolvers.push(Box::new(resolver));
    }

    pub fn len(&self) -> usize {
        self.resolvers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.resolvers.is_empty()
    }
}

impl Default for TrustChain {
    fn default() -> Self {
        Self::new()
    }
}

impl TrustResolver for TrustChain {
    fn resolve(&self, ctx: &TrustContext) -> TrustDecision {
        for resolver in &self.resolvers {
            match resolver.resolve(ctx) {
                TrustDecision::Allow => return TrustDecision::Allow,
                TrustDecision::Deny => return TrustDecision::Deny,
                TrustDecision::PromptUser => continue,
            }
        }
        if self.resolvers.is_empty() {
            TrustDecision::Allow
        } else {
            TrustDecision::PromptUser
        }
    }
}

impl std::fmt::Debug for TrustChain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrustChain")
            .field("len", &self.resolvers.len())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerStatus {
    Spawning,
    TrustRequired,
    ReadyForPrompt,
    Running,
    Finished,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerFailureKind {
    TrustGate,
    PromptDelivery,
    Protocol,
    Provider,
}

#[derive(Debug, Clone)]
pub struct WorkerEvent {
    pub seq: u64,
    pub kind: WorkerEventKind,
    pub status: WorkerStatus,
    pub detail: Option<String>,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerEventKind {
    Spawning,
    TrustRequired,
    TrustResolved,
    ReadyForPrompt,
    PromptMisdelivery,
    Running,
    Finished,
    Failed { kind: WorkerFailureKind },
}

#[derive(Debug, Clone)]
pub struct WorkerFailure {
    pub kind: WorkerFailureKind,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct Worker {
    pub id: String,
    pub status: WorkerStatus,
    pub events: Vec<WorkerEvent>,
    pub failure: Option<WorkerFailure>,
    next_seq: u64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

impl Worker {
    pub fn new(id: &str) -> Self {
        let mut w = Self {
            id: id.to_string(),
            status: WorkerStatus::Spawning,
            events: Vec::new(),
            failure: None,
            next_seq: 0,
        };
        w.emit_event(WorkerEventKind::Spawning);
        w
    }

    pub fn require_trust(&mut self) {
        self.status = WorkerStatus::TrustRequired;
        self.emit_event(WorkerEventKind::TrustRequired);
    }

    pub fn resolve_trust(&mut self) {
        self.status = WorkerStatus::ReadyForPrompt;
        self.emit_event(WorkerEventKind::TrustResolved);
    }

    /// Resolve trust using a policy-driven resolver.
    /// Returns the decision. On Allow, transitions to ReadyForPrompt.
    /// On Deny, transitions to Failed. On PromptUser, stays at TrustRequired.
    pub fn resolve_trust_with(
        &mut self,
        resolver: &dyn TrustResolver,
        ctx: &TrustContext,
    ) -> TrustDecision {
        let decision = resolver.resolve(ctx);
        match decision {
            TrustDecision::Allow => {
                self.status = WorkerStatus::ReadyForPrompt;
                self.emit_event(WorkerEventKind::TrustResolved);
            }
            TrustDecision::Deny => {
                self.fail(WorkerFailureKind::TrustGate, "trust denied by policy");
            }
            TrustDecision::PromptUser => {
                // Stay in TrustRequired — caller must prompt user and call resolve_trust() or fail()
            }
        }
        decision
    }

    pub fn mark_ready(&mut self) {
        self.status = WorkerStatus::ReadyForPrompt;
        self.emit_event(WorkerEventKind::ReadyForPrompt);
    }

    pub fn start_running(&mut self) {
        self.status = WorkerStatus::Running;
        self.emit_event(WorkerEventKind::Running);
    }

    pub fn finish(&mut self) {
        self.status = WorkerStatus::Finished;
        self.emit_event(WorkerEventKind::Finished);
    }

    pub fn fail(&mut self, kind: WorkerFailureKind, message: &str) {
        self.status = WorkerStatus::Failed;
        self.failure = Some(WorkerFailure {
            kind,
            message: message.to_string(),
        });
        self.emit_event(WorkerEventKind::Failed { kind });
    }

    pub fn emit_event(&mut self, kind: WorkerEventKind) {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.events.push(WorkerEvent {
            seq,
            kind,
            status: self.status,
            detail: None,
            timestamp_ms: now_ms(),
        });
    }
}

pub struct WorkerRegistry {
    workers: HashMap<String, Worker>,
}

impl WorkerRegistry {
    pub fn new() -> Self {
        Self {
            workers: HashMap::new(),
        }
    }

    pub fn create(&mut self, id: &str) -> &Worker {
        self.workers
            .entry(id.to_string())
            .or_insert_with(|| Worker::new(id))
    }

    pub fn get(&self, id: &str) -> Option<&Worker> {
        self.workers.get(id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Worker> {
        self.workers.get_mut(id)
    }

    pub fn list(&self) -> Vec<&Worker> {
        self.workers.values().collect()
    }

    pub fn remove(&mut self, id: &str) -> Option<Worker> {
        self.workers.remove(id)
    }
}

impl Default for WorkerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

static WORKER_REGISTRY: OnceLock<Arc<Mutex<WorkerRegistry>>> = OnceLock::new();

pub fn global_worker_registry() -> Arc<Mutex<WorkerRegistry>> {
    WORKER_REGISTRY
        .get_or_init(|| Arc::new(Mutex::new(WorkerRegistry::new())))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_lifecycle_spawning_to_finished() {
        let mut w = Worker::new("w1");
        assert_eq!(w.status, WorkerStatus::Spawning);

        w.mark_ready();
        assert_eq!(w.status, WorkerStatus::ReadyForPrompt);

        w.start_running();
        assert_eq!(w.status, WorkerStatus::Running);

        w.finish();
        assert_eq!(w.status, WorkerStatus::Finished);
        assert!(w.failure.is_none());
    }

    #[test]
    fn worker_failure_records_event() {
        let mut w = Worker::new("w2");
        w.fail(WorkerFailureKind::Provider, "timeout");

        assert_eq!(w.status, WorkerStatus::Failed);
        let f = w.failure.as_ref().unwrap();
        assert_eq!(f.kind, WorkerFailureKind::Provider);
        assert_eq!(f.message, "timeout");

        let last = w.events.last().unwrap();
        assert_eq!(
            last.kind,
            WorkerEventKind::Failed {
                kind: WorkerFailureKind::Provider
            }
        );
        assert_eq!(last.status, WorkerStatus::Failed);
    }

    #[test]
    fn trust_gate_flow() {
        let mut w = Worker::new("w3");
        w.require_trust();
        assert_eq!(w.status, WorkerStatus::TrustRequired);

        w.resolve_trust();
        assert_eq!(w.status, WorkerStatus::ReadyForPrompt);

        let kinds: Vec<_> = w.events.iter().map(|e| e.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![
                WorkerEventKind::Spawning,
                WorkerEventKind::TrustRequired,
                WorkerEventKind::TrustResolved,
            ]
        );
    }

    #[test]
    fn registry_create_and_get() {
        let mut reg = WorkerRegistry::new();
        reg.create("a");
        reg.create("b");

        assert!(reg.get("a").is_some());
        assert!(reg.get("b").is_some());
        assert!(reg.get("c").is_none());
        assert_eq!(reg.list().len(), 2);
    }

    #[test]
    fn registry_remove() {
        let mut reg = WorkerRegistry::new();
        reg.create("x");
        assert!(reg.get("x").is_some());

        let removed = reg.remove("x");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().id, "x");
        assert!(reg.get("x").is_none());
    }

    #[test]
    fn events_have_sequential_ids() {
        let mut w = Worker::new("seq");
        w.mark_ready();
        w.start_running();
        w.finish();

        let seqs: Vec<u64> = w.events.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, vec![0, 1, 2, 3]);
    }

    #[test]
    fn allow_all_policy_allows() {
        let resolver = AllowAllPolicy;
        let ctx = TrustContext::new("w1", "local");
        assert_eq!(resolver.resolve(&ctx), TrustDecision::Allow);
    }

    #[test]
    fn prompt_required_policy_prompts() {
        let resolver = PromptRequiredPolicy;
        let ctx = TrustContext::new("w1", "remote");
        assert_eq!(resolver.resolve(&ctx), TrustDecision::PromptUser);
    }

    #[test]
    fn rule_based_policy_deny_takes_precedence() {
        let resolver =
            RuleBasedPolicy::new(vec!["trusted".to_string()], vec!["blocked".to_string()]);
        let ctx = TrustContext::new("w1", "test")
            .with_label("trusted")
            .with_label("blocked");
        assert_eq!(resolver.resolve(&ctx), TrustDecision::Deny);
    }

    #[test]
    fn rule_based_policy_allow_on_match() {
        let resolver =
            RuleBasedPolicy::new(vec!["internal".to_string()], vec!["blocked".to_string()]);
        let ctx = TrustContext::new("w1", "test").with_label("internal");
        assert_eq!(resolver.resolve(&ctx), TrustDecision::Allow);
    }

    #[test]
    fn rule_based_policy_prompt_on_no_match() {
        let resolver =
            RuleBasedPolicy::new(vec!["internal".to_string()], vec!["blocked".to_string()]);
        let ctx = TrustContext::new("w1", "test").with_label("unknown");
        assert_eq!(resolver.resolve(&ctx), TrustDecision::PromptUser);
    }

    #[test]
    fn trust_policy_into_resolver() {
        let policy = TrustPolicy::AllowAll;
        let resolver = policy.into_resolver();
        let ctx = TrustContext::new("w1", "local");
        assert_eq!(resolver.resolve(&ctx), TrustDecision::Allow);

        let policy = TrustPolicy::PromptRequired;
        let resolver = policy.into_resolver();
        assert_eq!(resolver.resolve(&ctx), TrustDecision::PromptUser);

        let policy = TrustPolicy::RuleBased {
            allow_labels: vec!["ok".to_string()],
            deny_labels: vec![],
        };
        let resolver = policy.into_resolver();
        let ctx2 = TrustContext::new("w1", "test").with_label("ok");
        assert_eq!(resolver.resolve(&ctx2), TrustDecision::Allow);
    }

    #[test]
    fn resolve_trust_with_allow_transitions_to_ready() {
        let mut w = Worker::new("tw1");
        w.require_trust();
        let resolver = AllowAllPolicy;
        let ctx = TrustContext::new("tw1", "local");
        let decision = w.resolve_trust_with(&resolver, &ctx);
        assert_eq!(decision, TrustDecision::Allow);
        assert_eq!(w.status, WorkerStatus::ReadyForPrompt);
        assert!(
            w.events
                .iter()
                .any(|e| e.kind == WorkerEventKind::TrustResolved)
        );
    }

    #[test]
    fn resolve_trust_with_deny_transitions_to_failed() {
        let mut w = Worker::new("tw2");
        w.require_trust();
        let resolver = RuleBasedPolicy::new(vec![], vec!["evil".to_string()]);
        let ctx = TrustContext::new("tw2", "remote").with_label("evil");
        let decision = w.resolve_trust_with(&resolver, &ctx);
        assert_eq!(decision, TrustDecision::Deny);
        assert_eq!(w.status, WorkerStatus::Failed);
        let f = w.failure.as_ref().unwrap();
        assert_eq!(f.kind, WorkerFailureKind::TrustGate);
    }

    #[test]
    fn resolve_trust_with_prompt_stays_trust_required() {
        let mut w = Worker::new("tw3");
        w.require_trust();
        let resolver = PromptRequiredPolicy;
        let ctx = TrustContext::new("tw3", "unknown");
        let decision = w.resolve_trust_with(&resolver, &ctx);
        assert_eq!(decision, TrustDecision::PromptUser);
        assert_eq!(w.status, WorkerStatus::TrustRequired);
    }

    #[test]
    fn trust_context_builder() {
        let ctx = TrustContext::new("w1", "origin1")
            .with_label("a")
            .with_label("b");
        assert_eq!(ctx.worker_id, "w1");
        assert_eq!(ctx.origin, "origin1");
        assert_eq!(ctx.labels, vec!["a", "b"]);
    }

    #[test]
    fn trust_chain_empty_allows() {
        let chain = TrustChain::new();
        assert!(chain.is_empty());
        let ctx = TrustContext::new("w1", "local");
        assert_eq!(chain.resolve(&ctx), TrustDecision::Allow);
    }

    #[test]
    fn trust_chain_first_definitive_wins() {
        // PromptRequired → AllowAll → chain should return Allow (first definitive)
        let chain = TrustChain::new()
            .with(PromptRequiredPolicy)
            .with(AllowAllPolicy);
        let ctx = TrustContext::new("w1", "test");
        assert_eq!(chain.resolve(&ctx), TrustDecision::Allow);
    }

    #[test]
    fn trust_chain_deny_short_circuits() {
        let deny_rule = RuleBasedPolicy::new(vec![], vec!["evil".to_string()]);
        let chain = TrustChain::new().with(deny_rule).with(AllowAllPolicy);
        let ctx = TrustContext::new("w1", "test").with_label("evil");
        assert_eq!(chain.resolve(&ctx), TrustDecision::Deny);
    }

    #[test]
    fn trust_chain_all_prompt_returns_prompt() {
        let chain = TrustChain::new()
            .with(PromptRequiredPolicy)
            .with(PromptRequiredPolicy);
        assert_eq!(chain.len(), 2);
        let ctx = TrustContext::new("w1", "test");
        assert_eq!(chain.resolve(&ctx), TrustDecision::PromptUser);
    }

    #[test]
    fn trust_chain_push_works() {
        let mut chain = TrustChain::new();
        chain.push(AllowAllPolicy);
        assert_eq!(chain.len(), 1);
        assert!(!chain.is_empty());
        let ctx = TrustContext::new("w1", "test");
        assert_eq!(chain.resolve(&ctx), TrustDecision::Allow);
    }
}
