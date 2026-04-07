//! Recovery system — maps failure scenarios to recovery recipes.

/// Failure scenario classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureScenario {
    /// Network timeout or transient error.
    TransientNetwork,
    /// Rate limited (429).
    RateLimited,
    /// Authentication failure (401/403).
    AuthFailure,
    /// Model overloaded (529).
    ModelOverloaded,
    /// Context window exceeded.
    ContextOverflow,
    /// Tool execution failure.
    ToolFailure,
    /// Unrecoverable error.
    Fatal,
}

/// Recovery action to take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryAction {
    Retry { delay_ms: u64 },
    RetryWithBackoff { base_ms: u64, max_attempts: u32 },
    CompactAndRetry,
    SwitchModel { fallback: String },
    SkipTool,
    Escalate { reason: String },
    Abort { reason: String },
}

/// A recovery recipe for a failure scenario.
#[derive(Debug, Clone)]
pub struct RecoveryRecipe {
    pub scenario: FailureScenario,
    pub actions: Vec<RecoveryAction>,
    pub max_attempts: u32,
}

impl RecoveryRecipe {
    /// Get the recovery recipe for a failure scenario.
    pub fn for_scenario(scenario: FailureScenario) -> Self {
        match scenario {
            FailureScenario::TransientNetwork => Self {
                scenario,
                actions: vec![RecoveryAction::RetryWithBackoff {
                    base_ms: 500,
                    max_attempts: 3,
                }],
                max_attempts: 3,
            },
            FailureScenario::RateLimited => Self {
                scenario,
                actions: vec![RecoveryAction::Retry { delay_ms: 60_000 }],
                max_attempts: 3,
            },
            FailureScenario::AuthFailure => Self {
                scenario,
                actions: vec![RecoveryAction::Abort {
                    reason: "Authentication failed — check API key".to_string(),
                }],
                max_attempts: 1,
            },
            FailureScenario::ModelOverloaded => Self {
                scenario,
                actions: vec![
                    RecoveryAction::RetryWithBackoff {
                        base_ms: 2000,
                        max_attempts: 2,
                    },
                    RecoveryAction::SwitchModel {
                        fallback: "claude-haiku-4-5-20251001".to_string(),
                    },
                ],
                max_attempts: 3,
            },
            FailureScenario::ContextOverflow => Self {
                scenario,
                actions: vec![RecoveryAction::CompactAndRetry],
                max_attempts: 2,
            },
            FailureScenario::ToolFailure => Self {
                scenario,
                actions: vec![
                    RecoveryAction::SkipTool,
                    RecoveryAction::Escalate {
                        reason: "Tool failed after retry".to_string(),
                    },
                ],
                max_attempts: 2,
            },
            FailureScenario::Fatal => Self {
                scenario,
                actions: vec![RecoveryAction::Abort {
                    reason: "Unrecoverable error".to_string(),
                }],
                max_attempts: 1,
            },
        }
    }
}

/// Classify an error into a failure scenario.
pub fn classify_error(status_code: Option<u16>, message: &str) -> FailureScenario {
    match status_code {
        Some(401 | 403) => FailureScenario::AuthFailure,
        Some(429) => FailureScenario::RateLimited,
        Some(529) => FailureScenario::ModelOverloaded,
        Some(500..=599) => FailureScenario::TransientNetwork,
        _ => {
            if message.contains("timeout") || message.contains("connection") {
                FailureScenario::TransientNetwork
            } else if message.contains("context") || message.contains("token limit") {
                FailureScenario::ContextOverflow
            } else if message.contains("tool") {
                FailureScenario::ToolFailure
            } else {
                FailureScenario::Fatal
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_auth_error() {
        assert_eq!(
            classify_error(Some(401), "unauthorized"),
            FailureScenario::AuthFailure
        );
        assert_eq!(
            classify_error(Some(403), "forbidden"),
            FailureScenario::AuthFailure
        );
    }

    #[test]
    fn classify_rate_limit() {
        assert_eq!(
            classify_error(Some(429), "too many requests"),
            FailureScenario::RateLimited
        );
    }

    #[test]
    fn classify_overloaded() {
        assert_eq!(
            classify_error(Some(529), "overloaded"),
            FailureScenario::ModelOverloaded
        );
    }

    #[test]
    fn classify_transient() {
        assert_eq!(
            classify_error(Some(502), "bad gateway"),
            FailureScenario::TransientNetwork
        );
        assert_eq!(
            classify_error(None, "connection timeout"),
            FailureScenario::TransientNetwork
        );
    }

    #[test]
    fn classify_context_overflow() {
        assert_eq!(
            classify_error(None, "context window exceeded"),
            FailureScenario::ContextOverflow
        );
    }

    #[test]
    fn recipe_auth_aborts() {
        let recipe = RecoveryRecipe::for_scenario(FailureScenario::AuthFailure);
        assert_eq!(recipe.max_attempts, 1);
        assert!(matches!(recipe.actions[0], RecoveryAction::Abort { .. }));
    }

    #[test]
    fn recipe_transient_retries() {
        let recipe = RecoveryRecipe::for_scenario(FailureScenario::TransientNetwork);
        assert_eq!(recipe.max_attempts, 3);
        assert!(matches!(
            recipe.actions[0],
            RecoveryAction::RetryWithBackoff { .. }
        ));
    }

    #[test]
    fn recipe_context_compacts() {
        let recipe = RecoveryRecipe::for_scenario(FailureScenario::ContextOverflow);
        assert!(matches!(recipe.actions[0], RecoveryAction::CompactAndRetry));
    }

    #[test]
    fn recipe_overloaded_has_fallback() {
        let recipe = RecoveryRecipe::for_scenario(FailureScenario::ModelOverloaded);
        assert!(
            recipe
                .actions
                .iter()
                .any(|a| matches!(a, RecoveryAction::SwitchModel { .. }))
        );
    }
}
