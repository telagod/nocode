use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchFreshness {
    Fresh,
    Stale,
    Diverged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaleBranchPolicy {
    WarnOnly,
    Block,
    AutoRebase,
    AutoMergeForward,
}

#[derive(Debug, Clone)]
pub struct StaleBranchAction {
    pub freshness: BranchFreshness,
    pub policy: StaleBranchPolicy,
    pub action: StaleBranchActionKind,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaleBranchActionKind {
    Noop,
    Warn,
    Block,
    Rebase,
    MergeForward,
}

/// Check how fresh a branch is relative to its base.
/// Stub: always returns `Fresh` — real implementation needs `git rev-list`.
pub fn check_freshness(_branch: &str, _base: &str, _max_age: Duration) -> BranchFreshness {
    BranchFreshness::Fresh
}

/// Decide what action to take given a freshness reading and a policy.
pub fn apply_policy(freshness: BranchFreshness, policy: StaleBranchPolicy) -> StaleBranchAction {
    let (action, detail) = match freshness {
        BranchFreshness::Fresh => (StaleBranchActionKind::Noop, None),
        BranchFreshness::Stale => match policy {
            StaleBranchPolicy::WarnOnly => (
                StaleBranchActionKind::Warn,
                Some("Branch is stale; consider rebasing".to_string()),
            ),
            StaleBranchPolicy::Block => (
                StaleBranchActionKind::Block,
                Some("Branch is stale; merges blocked until rebased".to_string()),
            ),
            StaleBranchPolicy::AutoRebase => (
                StaleBranchActionKind::Rebase,
                Some("Branch is stale; auto-rebase triggered".to_string()),
            ),
            StaleBranchPolicy::AutoMergeForward => (
                StaleBranchActionKind::MergeForward,
                Some("Branch is stale; merge-forward triggered".to_string()),
            ),
        },
        BranchFreshness::Diverged => (
            StaleBranchActionKind::MergeForward,
            Some("Branch has diverged from base; merge-forward required".to_string()),
        ),
    };
    StaleBranchAction {
        freshness,
        policy,
        action,
        detail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_freshness_returns_fresh_stub() {
        let f = check_freshness("feature/x", "main", Duration::from_secs(3600));
        assert_eq!(f, BranchFreshness::Fresh);
    }

    #[test]
    fn fresh_branch_yields_noop() {
        let a = apply_policy(BranchFreshness::Fresh, StaleBranchPolicy::Block);
        assert_eq!(a.action, StaleBranchActionKind::Noop);
        assert!(a.detail.is_none());
    }

    #[test]
    fn stale_warn_only() {
        let a = apply_policy(BranchFreshness::Stale, StaleBranchPolicy::WarnOnly);
        assert_eq!(a.action, StaleBranchActionKind::Warn);
        assert!(a.detail.is_some());
    }

    #[test]
    fn stale_block() {
        let a = apply_policy(BranchFreshness::Stale, StaleBranchPolicy::Block);
        assert_eq!(a.action, StaleBranchActionKind::Block);
    }

    #[test]
    fn stale_auto_rebase() {
        let a = apply_policy(BranchFreshness::Stale, StaleBranchPolicy::AutoRebase);
        assert_eq!(a.action, StaleBranchActionKind::Rebase);
    }

    #[test]
    fn diverged_always_merge_forward() {
        let a = apply_policy(BranchFreshness::Diverged, StaleBranchPolicy::WarnOnly);
        assert_eq!(a.action, StaleBranchActionKind::MergeForward);
        assert!(a.detail.unwrap().contains("diverged"));
    }
}
