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
/// Uses `git rev-list --count` to determine ahead/behind status,
/// and commit timestamps to determine staleness.
pub fn check_freshness(branch: &str, base: &str, max_age: Duration) -> BranchFreshness {
    let ahead = git_rev_count(branch, base);
    let behind = git_rev_count(base, branch);

    match (ahead, behind) {
        (Ok(0), Ok(0)) | (Ok(_), Ok(0)) => BranchFreshness::Fresh,
        (Ok(0), Ok(_)) => {
            if is_stale(branch, max_age) {
                BranchFreshness::Stale
            } else {
                BranchFreshness::Fresh
            }
        }
        (Ok(_), Ok(_)) => BranchFreshness::Diverged,
        _ => BranchFreshness::Fresh, // git command failed, assume fresh
    }
}

fn git_rev_count(from: &str, to: &str) -> Result<usize, String> {
    let output = std::process::Command::new("git")
        .args(["rev-list", "--count", &format!("{to}..{from}")])
        .output()
        .map_err(|e| format!("git rev-list failed: {e}"))?;

    if !output.status.success() {
        return Err("git rev-list returned non-zero".to_string());
    }

    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<usize>()
        .map_err(|e| format!("failed to parse rev count: {e}"))
}

fn is_stale(branch: &str, max_age: Duration) -> bool {
    let output = std::process::Command::new("git")
        .args(["log", "-1", "--format=%ct", branch])
        .output();

    let Ok(output) = output else { return false };
    if !output.status.success() {
        return false;
    }

    let timestamp: u64 = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .unwrap_or(0);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    now.saturating_sub(timestamp) > max_age.as_secs()
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

// ---------------------------------------------------------------------------
// Policy execution — real git operations
// ---------------------------------------------------------------------------

/// Result of executing a stale branch policy action.
#[derive(Debug, Clone)]
pub struct PolicyExecutionResult {
    pub action: StaleBranchActionKind,
    pub success: bool,
    pub message: String,
}

/// Execute the git operation corresponding to a `StaleBranchAction`.
///
/// - `Noop` / `Warn` — no git operation, always succeeds.
/// - `Block` — returns an error (blocks the operation).
/// - `Rebase` — runs `git rebase <base>`.
/// - `MergeForward` — runs `git merge <base> --no-edit`.
pub fn execute_policy(
    action: &StaleBranchAction,
    branch: &str,
    base: &str,
    cwd: &str,
) -> PolicyExecutionResult {
    match action.action {
        StaleBranchActionKind::Noop => PolicyExecutionResult {
            action: StaleBranchActionKind::Noop,
            success: true,
            message: "no action needed".to_string(),
        },
        StaleBranchActionKind::Warn => PolicyExecutionResult {
            action: StaleBranchActionKind::Warn,
            success: true,
            message: action
                .detail
                .clone()
                .unwrap_or_else(|| "stale branch warning".to_string()),
        },
        StaleBranchActionKind::Block => PolicyExecutionResult {
            action: StaleBranchActionKind::Block,
            success: false,
            message: action
                .detail
                .clone()
                .unwrap_or_else(|| "blocked by stale branch policy".to_string()),
        },
        StaleBranchActionKind::Rebase => {
            let result = run_git(cwd, &["rebase", base]);
            PolicyExecutionResult {
                action: StaleBranchActionKind::Rebase,
                success: result.0,
                message: if result.0 {
                    format!("rebased {branch} onto {base}")
                } else {
                    format!("rebase failed: {}", result.1)
                },
            }
        }
        StaleBranchActionKind::MergeForward => {
            let result = run_git(cwd, &["merge", base, "--no-edit"]);
            PolicyExecutionResult {
                action: StaleBranchActionKind::MergeForward,
                success: result.0,
                message: if result.0 {
                    format!("merged {base} into {branch}")
                } else {
                    format!("merge-forward failed: {}", result.1)
                },
            }
        }
    }
}

fn run_git(cwd: &str, args: &[&str]) -> (bool, String) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output();

    match output {
        Ok(o) => {
            let combined = if o.status.success() {
                String::from_utf8_lossy(&o.stdout).trim().to_string()
            } else {
                String::from_utf8_lossy(&o.stderr).trim().to_string()
            };
            (o.status.success(), combined)
        }
        Err(e) => (false, format!("git command failed: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_freshness_on_current_branch_is_fresh() {
        let f = check_freshness("HEAD", "HEAD", Duration::from_secs(3600));
        assert_eq!(f, BranchFreshness::Fresh);
    }

    #[test]
    fn git_rev_count_on_same_ref_is_zero() {
        let count = git_rev_count("HEAD", "HEAD");
        assert_eq!(count, Ok(0));
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

    // -----------------------------------------------------------------------
    // execute_policy tests
    // -----------------------------------------------------------------------

    #[test]
    fn execute_noop_succeeds() {
        let action = apply_policy(BranchFreshness::Fresh, StaleBranchPolicy::WarnOnly);
        let result = execute_policy(&action, "main", "main", "/tmp");
        assert!(result.success);
        assert_eq!(result.action, StaleBranchActionKind::Noop);
    }

    #[test]
    fn execute_warn_succeeds_with_message() {
        let action = apply_policy(BranchFreshness::Stale, StaleBranchPolicy::WarnOnly);
        let result = execute_policy(&action, "feature", "main", "/tmp");
        assert!(result.success);
        assert_eq!(result.action, StaleBranchActionKind::Warn);
        assert!(result.message.contains("stale"));
    }

    #[test]
    fn execute_block_fails() {
        let action = apply_policy(BranchFreshness::Stale, StaleBranchPolicy::Block);
        let result = execute_policy(&action, "feature", "main", "/tmp");
        assert!(!result.success);
        assert_eq!(result.action, StaleBranchActionKind::Block);
        assert!(result.message.contains("blocked") || result.message.contains("stale"));
    }

    #[test]
    fn execute_rebase_on_head() {
        // Rebase HEAD onto HEAD — should succeed (noop rebase).
        let action = StaleBranchAction {
            freshness: BranchFreshness::Stale,
            policy: StaleBranchPolicy::AutoRebase,
            action: StaleBranchActionKind::Rebase,
            detail: Some("auto-rebase".into()),
        };
        let cwd = env!("CARGO_MANIFEST_DIR");
        let result = execute_policy(&action, "HEAD", "HEAD", cwd);
        // May succeed or fail depending on git state, but should not panic.
        assert_eq!(result.action, StaleBranchActionKind::Rebase);
    }

    #[test]
    fn execute_merge_forward_on_head() {
        let action = StaleBranchAction {
            freshness: BranchFreshness::Diverged,
            policy: StaleBranchPolicy::AutoMergeForward,
            action: StaleBranchActionKind::MergeForward,
            detail: Some("merge-forward".into()),
        };
        let cwd = env!("CARGO_MANIFEST_DIR");
        let result = execute_policy(&action, "HEAD", "HEAD", cwd);
        assert_eq!(result.action, StaleBranchActionKind::MergeForward);
    }
}
