use std::process::Command;

use crate::policy_engine::GreenLevel;

pub const TARGETED_TESTS: GreenLevel = 1;
pub const PACKAGE: GreenLevel = 2;
pub const WORKSPACE: GreenLevel = 3;
pub const MERGE_READY: GreenLevel = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractOutcome {
    Met,
    NotMet {
        required: GreenLevel,
        observed: GreenLevel,
    },
}

#[derive(Debug, Clone)]
pub struct GreenContract {
    pub name: String,
    pub required_level: GreenLevel,
}

impl GreenContract {
    pub fn evaluate(&self, observed: GreenLevel) -> ContractOutcome {
        if observed >= self.required_level {
            ContractOutcome::Met
        } else {
            ContractOutcome::NotMet {
                required: self.required_level,
                observed,
            }
        }
    }
}

pub fn level_name(level: GreenLevel) -> &'static str {
    match level {
        1 => "targeted-tests",
        2 => "package",
        3 => "workspace",
        4 => "merge-ready",
        _ => "unknown",
    }
}

/// Configuration for green level checks.
#[derive(Debug, Clone)]
pub struct GreenCheckConfig {
    /// Working directory for cargo commands.
    pub cwd: String,
    /// Target test name for TargetedTests level (e.g. a specific test name).
    pub target_test: Option<String>,
    /// Package name for Package level (e.g. "nocode-core").
    pub package: Option<String>,
}

/// Result of a green level check.
#[derive(Debug, Clone)]
pub struct GreenCheckResult {
    /// The highest level that passed.
    pub achieved_level: GreenLevel,
    /// Per-level pass/fail details.
    pub details: Vec<GreenCheckDetail>,
}

#[derive(Debug, Clone)]
pub struct GreenCheckDetail {
    pub level: GreenLevel,
    pub passed: bool,
    pub command: String,
    pub output: String,
}

/// Trait for executing shell commands, enabling test mocking.
pub trait CommandRunner: Send + Sync {
    fn run(&self, cwd: &str, command: &str) -> CommandRunResult;
}

#[derive(Debug, Clone)]
pub struct CommandRunResult {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Real command runner that executes via `sh -c`.
pub struct ShellCommandRunner;

impl CommandRunner for ShellCommandRunner {
    fn run(&self, cwd: &str, command: &str) -> CommandRunResult {
        let output = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(cwd)
            .output();
        match output {
            Ok(o) => CommandRunResult {
                success: o.status.success(),
                stdout: String::from_utf8_lossy(&o.stdout).to_string(),
                stderr: String::from_utf8_lossy(&o.stderr).to_string(),
            },
            Err(e) => CommandRunResult {
                success: false,
                stdout: String::new(),
                stderr: format!("failed to execute: {e}"),
            },
        }
    }
}

/// Check the green level by executing cargo commands at each tier.
///
/// Levels are checked in order: TargetedTests → Package → Workspace → MergeReady.
/// Stops at the first failure and returns the highest achieved level (0 if none pass).
pub fn check_green_level(
    config: &GreenCheckConfig,
    runner: &dyn CommandRunner,
) -> GreenCheckResult {
    let mut details = Vec::new();
    let mut achieved: GreenLevel = 0;

    // Level 1: TargetedTests — cargo test <name> or cargo test (single)
    let cmd1 = if let Some(ref test_name) = config.target_test {
        format!("cargo test {test_name}")
    } else {
        "cargo test".to_string()
    };
    let r1 = runner.run(&config.cwd, &cmd1);
    details.push(GreenCheckDetail {
        level: TARGETED_TESTS,
        passed: r1.success,
        command: cmd1,
        output: if r1.success {
            r1.stdout.clone()
        } else {
            r1.stderr.clone()
        },
    });
    if !r1.success {
        return GreenCheckResult {
            achieved_level: achieved,
            details,
        };
    }
    achieved = TARGETED_TESTS;

    // Level 2: Package — cargo test -p <package>
    let cmd2 = if let Some(ref pkg) = config.package {
        format!("cargo test -p {pkg}")
    } else {
        "cargo test".to_string()
    };
    let r2 = runner.run(&config.cwd, &cmd2);
    details.push(GreenCheckDetail {
        level: PACKAGE,
        passed: r2.success,
        command: cmd2,
        output: if r2.success {
            r2.stdout.clone()
        } else {
            r2.stderr.clone()
        },
    });
    if !r2.success {
        return GreenCheckResult {
            achieved_level: achieved,
            details,
        };
    }
    achieved = PACKAGE;

    // Level 3: Workspace — cargo test --workspace
    let cmd3 = "cargo test --workspace";
    let r3 = runner.run(&config.cwd, cmd3);
    details.push(GreenCheckDetail {
        level: WORKSPACE,
        passed: r3.success,
        command: cmd3.to_string(),
        output: if r3.success {
            r3.stdout.clone()
        } else {
            r3.stderr.clone()
        },
    });
    if !r3.success {
        return GreenCheckResult {
            achieved_level: achieved,
            details,
        };
    }
    achieved = WORKSPACE;

    // Level 4: MergeReady — cargo clippy + cargo test --workspace
    let cmd4a = "cargo clippy --all-targets -- -D warnings";
    let r4a = runner.run(&config.cwd, cmd4a);
    if !r4a.success {
        details.push(GreenCheckDetail {
            level: MERGE_READY,
            passed: false,
            command: cmd4a.to_string(),
            output: r4a.stderr.clone(),
        });
        return GreenCheckResult {
            achieved_level: achieved,
            details,
        };
    }
    // Workspace tests already passed at level 3, clippy passed — merge ready
    details.push(GreenCheckDetail {
        level: MERGE_READY,
        passed: true,
        command: format!("{cmd4a} && cargo test --workspace"),
        output: r4a.stdout.clone(),
    });
    achieved = MERGE_READY;

    GreenCheckResult {
        achieved_level: achieved,
        details,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_met_when_observed_equals_required() {
        let c = GreenContract {
            name: "ci".into(),
            required_level: PACKAGE,
        };
        assert_eq!(c.evaluate(PACKAGE), ContractOutcome::Met);
    }

    #[test]
    fn contract_met_when_observed_exceeds_required() {
        let c = GreenContract {
            name: "ci".into(),
            required_level: TARGETED_TESTS,
        };
        assert_eq!(c.evaluate(WORKSPACE), ContractOutcome::Met);
    }

    #[test]
    fn contract_not_met_when_observed_below_required() {
        let c = GreenContract {
            name: "merge".into(),
            required_level: MERGE_READY,
        };
        assert_eq!(
            c.evaluate(PACKAGE),
            ContractOutcome::NotMet {
                required: MERGE_READY,
                observed: PACKAGE
            },
        );
    }

    #[test]
    fn level_name_maps_correctly() {
        assert_eq!(level_name(1), "targeted-tests");
        assert_eq!(level_name(2), "package");
        assert_eq!(level_name(3), "workspace");
        assert_eq!(level_name(4), "merge-ready");
        assert_eq!(level_name(0), "unknown");
        assert_eq!(level_name(255), "unknown");
    }

    /// Mock runner that returns success/failure based on a predicate on the command string.
    struct MockRunner {
        /// Commands containing any of these substrings will fail.
        fail_on: Vec<String>,
    }

    impl MockRunner {
        fn all_pass() -> Self {
            Self { fail_on: vec![] }
        }

        fn failing_on(patterns: &[&str]) -> Self {
            Self {
                fail_on: patterns.iter().map(|s| s.to_string()).collect(),
            }
        }
    }

    impl CommandRunner for MockRunner {
        fn run(&self, _cwd: &str, command: &str) -> CommandRunResult {
            let should_fail = self.fail_on.iter().any(|p| command.contains(p.as_str()));
            if should_fail {
                CommandRunResult {
                    success: false,
                    stdout: String::new(),
                    stderr: format!("MOCK FAIL: {command}"),
                }
            } else {
                CommandRunResult {
                    success: true,
                    stdout: format!("MOCK OK: {command}"),
                    stderr: String::new(),
                }
            }
        }
    }

    fn default_config() -> GreenCheckConfig {
        GreenCheckConfig {
            cwd: "/tmp".to_string(),
            target_test: Some("my_test".to_string()),
            package: Some("nocode-core".to_string()),
        }
    }

    #[test]
    fn check_all_levels_pass() {
        let runner = MockRunner::all_pass();
        let result = check_green_level(&default_config(), &runner);
        assert_eq!(result.achieved_level, MERGE_READY);
        assert_eq!(result.details.len(), 4);
        assert!(result.details.iter().all(|d| d.passed));
    }

    #[test]
    fn check_fails_at_targeted_tests() {
        let runner = MockRunner::failing_on(&["cargo test"]);
        let result = check_green_level(&default_config(), &runner);
        assert_eq!(result.achieved_level, 0);
        assert_eq!(result.details.len(), 1);
        assert!(!result.details[0].passed);
        assert_eq!(result.details[0].level, TARGETED_TESTS);
    }

    #[test]
    fn check_fails_at_package_level() {
        let runner = MockRunner::failing_on(&["cargo test -p"]);
        let result = check_green_level(&default_config(), &runner);
        assert_eq!(result.achieved_level, TARGETED_TESTS);
        assert_eq!(result.details.len(), 2);
        assert!(result.details[0].passed);
        assert!(!result.details[1].passed);
        assert_eq!(result.details[1].level, PACKAGE);
    }

    #[test]
    fn check_fails_at_workspace_level() {
        let runner = MockRunner::failing_on(&["--workspace"]);
        let result = check_green_level(&default_config(), &runner);
        assert_eq!(result.achieved_level, PACKAGE);
        assert_eq!(result.details.len(), 3);
        assert!(result.details[0].passed);
        assert!(result.details[1].passed);
        assert!(!result.details[2].passed);
        assert_eq!(result.details[2].level, WORKSPACE);
    }

    #[test]
    fn check_fails_at_merge_ready_clippy() {
        let runner = MockRunner::failing_on(&["clippy"]);
        let result = check_green_level(&default_config(), &runner);
        assert_eq!(result.achieved_level, WORKSPACE);
        assert_eq!(result.details.len(), 4);
        assert!(result.details[0].passed);
        assert!(result.details[1].passed);
        assert!(result.details[2].passed);
        assert!(!result.details[3].passed);
        assert_eq!(result.details[3].level, MERGE_READY);
    }

    #[test]
    fn check_without_target_test_or_package() {
        let config = GreenCheckConfig {
            cwd: "/tmp".to_string(),
            target_test: None,
            package: None,
        };
        let runner = MockRunner::all_pass();
        let result = check_green_level(&config, &runner);
        assert_eq!(result.achieved_level, MERGE_READY);
        // Level 1 and 2 both use "cargo test" when no target/package specified
        assert!(result.details[0].command.contains("cargo test"));
        assert!(result.details[1].command.contains("cargo test"));
    }

    #[test]
    fn check_result_integrates_with_contract() {
        let runner = MockRunner::failing_on(&["--workspace"]);
        let result = check_green_level(&default_config(), &runner);

        let contract = GreenContract {
            name: "merge-gate".into(),
            required_level: MERGE_READY,
        };
        assert_eq!(
            contract.evaluate(result.achieved_level),
            ContractOutcome::NotMet {
                required: MERGE_READY,
                observed: PACKAGE,
            }
        );

        let contract2 = GreenContract {
            name: "basic-gate".into(),
            required_level: PACKAGE,
        };
        assert_eq!(
            contract2.evaluate(result.achieved_level),
            ContractOutcome::Met
        );
    }
}
