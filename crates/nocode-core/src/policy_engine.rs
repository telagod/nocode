use std::time::Duration;

pub type GreenLevel = u8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaneBlocker {
    None,
    Startup,
    External,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewStatus {
    Pending,
    Approved,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffScope {
    Full,
    Scoped,
}

#[derive(Debug, Clone)]
pub struct LaneContext {
    pub lane_id: String,
    pub green_level: GreenLevel,
    pub branch_freshness: Duration,
    pub blocker: LaneBlocker,
    pub review_status: ReviewStatus,
    pub diff_scope: DiffScope,
    pub completed: bool,
    pub reconciled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyCondition {
    GreenAt { level: GreenLevel },
    StaleBranch,
    StartupBlocked,
    LaneCompleted,
    LaneReconciled,
    ReviewPassed,
    ScopedDiff,
    TimedOut { duration: Duration },
    And(Vec<PolicyCondition>),
    Or(Vec<PolicyCondition>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconcileReason {
    AlreadyMerged,
    Superseded,
    EmptyDiff,
    ManualClose,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyAction {
    MergeToDev,
    MergeForward,
    RecoverOnce,
    Escalate,
    CloseoutLane,
    CleanupSession,
    Reconcile { reason: ReconcileReason },
    Notify { channel: String },
    Block { reason: String },
    Chain(Vec<PolicyAction>),
}

#[derive(Debug, Clone)]
pub struct PolicyRule {
    pub name: String,
    pub condition: PolicyCondition,
    pub action: PolicyAction,
    pub priority: u32,
}

pub struct PolicyEngine {
    rules: Vec<PolicyRule>,
}

const STALE_BRANCH_THRESHOLD: Duration = Duration::from_secs(3600);

impl PolicyCondition {
    pub fn matches(&self, ctx: &LaneContext) -> bool {
        match self {
            PolicyCondition::GreenAt { level } => ctx.green_level >= *level,
            PolicyCondition::StaleBranch => ctx.branch_freshness >= STALE_BRANCH_THRESHOLD,
            PolicyCondition::StartupBlocked => ctx.blocker == LaneBlocker::Startup,
            PolicyCondition::LaneCompleted => ctx.completed,
            PolicyCondition::LaneReconciled => ctx.reconciled,
            PolicyCondition::ReviewPassed => ctx.review_status == ReviewStatus::Approved,
            PolicyCondition::ScopedDiff => ctx.diff_scope == DiffScope::Scoped,
            PolicyCondition::TimedOut { duration } => ctx.branch_freshness >= *duration,
            PolicyCondition::And(conditions) => conditions.iter().all(|c| c.matches(ctx)),
            PolicyCondition::Or(conditions) => conditions.iter().any(|c| c.matches(ctx)),
        }
    }
}

impl PolicyAction {
    pub fn flatten(&self) -> Vec<&PolicyAction> {
        match self {
            PolicyAction::Chain(actions) => actions.iter().flat_map(|a| a.flatten()).collect(),
            other => vec![other],
        }
    }
}

impl PolicyEngine {
    pub fn new(mut rules: Vec<PolicyRule>) -> Self {
        rules.sort_by_key(|r| r.priority);
        Self { rules }
    }

    pub fn evaluate(&self, ctx: &LaneContext) -> Vec<&PolicyAction> {
        self.rules
            .iter()
            .filter(|rule| rule.condition.matches(ctx))
            .flat_map(|rule| rule.action.flatten())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_ctx() -> LaneContext {
        LaneContext {
            lane_id: "test-lane".to_string(),
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
    fn green_at_matches_when_level_sufficient() {
        let cond = PolicyCondition::GreenAt { level: 3 };
        let mut ctx = base_ctx();
        ctx.green_level = 2;
        assert!(!cond.matches(&ctx));
        ctx.green_level = 3;
        assert!(cond.matches(&ctx));
        ctx.green_level = 5;
        assert!(cond.matches(&ctx));
    }

    #[test]
    fn stale_branch_matches_after_one_hour() {
        let cond = PolicyCondition::StaleBranch;
        let mut ctx = base_ctx();
        ctx.branch_freshness = Duration::from_secs(3599);
        assert!(!cond.matches(&ctx));
        ctx.branch_freshness = Duration::from_secs(3600);
        assert!(cond.matches(&ctx));
        ctx.branch_freshness = Duration::from_secs(7200);
        assert!(cond.matches(&ctx));
    }

    #[test]
    fn and_requires_all_conditions() {
        let cond = PolicyCondition::And(vec![
            PolicyCondition::GreenAt { level: 2 },
            PolicyCondition::ReviewPassed,
        ]);
        let mut ctx = base_ctx();
        ctx.green_level = 3;
        ctx.review_status = ReviewStatus::Pending;
        assert!(!cond.matches(&ctx));
        ctx.review_status = ReviewStatus::Approved;
        assert!(cond.matches(&ctx));
    }

    #[test]
    fn or_requires_any_condition() {
        let cond = PolicyCondition::Or(vec![
            PolicyCondition::LaneCompleted,
            PolicyCondition::LaneReconciled,
        ]);
        let mut ctx = base_ctx();
        assert!(!cond.matches(&ctx));
        ctx.completed = true;
        assert!(cond.matches(&ctx));
        ctx.completed = false;
        ctx.reconciled = true;
        assert!(cond.matches(&ctx));
    }

    #[test]
    fn chain_flattens_nested_actions() {
        let action = PolicyAction::Chain(vec![
            PolicyAction::MergeToDev,
            PolicyAction::Chain(vec![PolicyAction::MergeForward, PolicyAction::Escalate]),
            PolicyAction::CleanupSession,
        ]);
        let flat = action.flatten();
        assert_eq!(flat.len(), 4);
        assert_eq!(*flat[0], PolicyAction::MergeToDev);
        assert_eq!(*flat[1], PolicyAction::MergeForward);
        assert_eq!(*flat[2], PolicyAction::Escalate);
        assert_eq!(*flat[3], PolicyAction::CleanupSession);
    }

    #[test]
    fn engine_evaluates_by_priority() {
        let rules = vec![
            PolicyRule {
                name: "low-priority".to_string(),
                condition: PolicyCondition::GreenAt { level: 1 },
                action: PolicyAction::MergeForward,
                priority: 10,
            },
            PolicyRule {
                name: "high-priority".to_string(),
                condition: PolicyCondition::GreenAt { level: 1 },
                action: PolicyAction::MergeToDev,
                priority: 1,
            },
        ];
        let engine = PolicyEngine::new(rules);
        let mut ctx = base_ctx();
        ctx.green_level = 2;
        let actions = engine.evaluate(&ctx);
        assert_eq!(actions.len(), 2);
        assert_eq!(*actions[0], PolicyAction::MergeToDev);
        assert_eq!(*actions[1], PolicyAction::MergeForward);
    }

    #[test]
    fn no_matching_rules_returns_empty() {
        let rules = vec![PolicyRule {
            name: "needs-green-5".to_string(),
            condition: PolicyCondition::GreenAt { level: 5 },
            action: PolicyAction::Escalate,
            priority: 1,
        }];
        let engine = PolicyEngine::new(rules);
        let ctx = base_ctx();
        let actions = engine.evaluate(&ctx);
        assert!(actions.is_empty());
    }
}
