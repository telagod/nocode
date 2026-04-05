use std::collections::HashMap;

/// Failure scenarios that the recovery system can handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FailureScenario {
    TrustPromptUnresolved,
    PromptMisdelivery,
    StaleBranch,
    CompileRedCrossCrate,
    McpHandshakeFailure,
    PartialPluginStartup,
    ProviderFailure,
}

/// Individual recovery actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryStep {
    AcceptTrustPrompt,
    RedirectPromptToAgent,
    RebaseBranch,
    CleanBuild,
    RetryMcpHandshake { timeout_ms: u64 },
    RestartPlugin,
    RestartWorker,
    EscalateToHuman,
}

/// What to do when recovery exhausts all attempts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscalationPolicy {
    AlertHuman,
    LogAndContinue,
    Abort,
}

/// A recipe describing how to recover from a specific failure scenario.
#[derive(Debug, Clone)]
pub struct RecoveryRecipe {
    pub scenario: FailureScenario,
    pub steps: Vec<RecoveryStep>,
    pub max_attempts: u32,
    pub escalation: EscalationPolicy,
}

/// Outcome of a recovery attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryResult {
    Recovered,
    PartialRecovery,
    EscalationRequired { policy: EscalationPolicy },
}

/// Events emitted during recovery for observability.
#[derive(Debug, Clone)]
pub enum RecoveryEvent {
    RecoveryAttempted {
        scenario: FailureScenario,
        step: RecoveryStep,
    },
    RecoverySucceeded {
        scenario: FailureScenario,
    },
    RecoveryFailed {
        scenario: FailureScenario,
        reason: String,
    },
    Escalated {
        scenario: FailureScenario,
        policy: EscalationPolicy,
    },
}

/// Tracks recovery attempts and events across scenarios.
pub struct RecoveryContext {
    attempts: HashMap<FailureScenario, u32>,
    events: Vec<RecoveryEvent>,
}

/// Returns the hardcoded recovery recipe for a given failure scenario.
#[must_use]
pub fn recipe_for(scenario: FailureScenario) -> RecoveryRecipe {
    match scenario {
        FailureScenario::TrustPromptUnresolved => RecoveryRecipe {
            scenario,
            steps: vec![RecoveryStep::AcceptTrustPrompt],
            max_attempts: 1,
            escalation: EscalationPolicy::AlertHuman,
        },
        FailureScenario::PromptMisdelivery => RecoveryRecipe {
            scenario,
            steps: vec![RecoveryStep::RedirectPromptToAgent],
            max_attempts: 2,
            escalation: EscalationPolicy::AlertHuman,
        },
        FailureScenario::StaleBranch => RecoveryRecipe {
            scenario,
            steps: vec![RecoveryStep::RebaseBranch],
            max_attempts: 2,
            escalation: EscalationPolicy::Abort,
        },
        FailureScenario::CompileRedCrossCrate => RecoveryRecipe {
            scenario,
            steps: vec![RecoveryStep::CleanBuild],
            max_attempts: 2,
            escalation: EscalationPolicy::Abort,
        },
        FailureScenario::McpHandshakeFailure => RecoveryRecipe {
            scenario,
            steps: vec![RecoveryStep::RetryMcpHandshake { timeout_ms: 5000 }],
            max_attempts: 3,
            escalation: EscalationPolicy::AlertHuman,
        },
        FailureScenario::PartialPluginStartup => RecoveryRecipe {
            scenario,
            steps: vec![RecoveryStep::RestartPlugin],
            max_attempts: 2,
            escalation: EscalationPolicy::LogAndContinue,
        },
        FailureScenario::ProviderFailure => RecoveryRecipe {
            scenario,
            steps: vec![RecoveryStep::RestartWorker],
            max_attempts: 3,
            escalation: EscalationPolicy::Abort,
        },
    }
}

impl RecoveryContext {
    /// Creates a new empty recovery context.
    #[must_use]
    pub fn new() -> Self {
        Self {
            attempts: HashMap::new(),
            events: Vec::new(),
        }
    }

    /// Attempts recovery for the given scenario.
    ///
    /// Returns `Recovered` on the first attempt if under `max_attempts`,
    /// otherwise escalates.
    pub fn attempt_recovery(&mut self, scenario: FailureScenario) -> RecoveryResult {
        let recipe = recipe_for(scenario);
        let count = self.attempts.entry(scenario).or_insert(0);
        *count += 1;

        if *count > recipe.max_attempts {
            self.events.push(RecoveryEvent::RecoveryFailed {
                scenario,
                reason: format!("exceeded max attempts ({0})", recipe.max_attempts),
            });
            self.events.push(RecoveryEvent::Escalated {
                scenario,
                policy: recipe.escalation,
            });
            return RecoveryResult::EscalationRequired {
                policy: recipe.escalation,
            };
        }

        // Simulate executing each step in the recipe.
        for step in &recipe.steps {
            self.events.push(RecoveryEvent::RecoveryAttempted {
                scenario,
                step: step.clone(),
            });
        }

        self.events.push(RecoveryEvent::RecoverySucceeded { scenario });
        RecoveryResult::Recovered
    }

    /// Returns all events recorded so far.
    #[must_use]
    pub fn events(&self) -> &[RecoveryEvent] {
        &self.events
    }
}

impl Default for RecoveryContext {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_SCENARIOS: [FailureScenario; 7] = [
        FailureScenario::TrustPromptUnresolved,
        FailureScenario::PromptMisdelivery,
        FailureScenario::StaleBranch,
        FailureScenario::CompileRedCrossCrate,
        FailureScenario::McpHandshakeFailure,
        FailureScenario::PartialPluginStartup,
        FailureScenario::ProviderFailure,
    ];

    #[test]
    fn recipe_for_all_scenarios() {
        for scenario in ALL_SCENARIOS {
            let recipe = recipe_for(scenario);
            assert_eq!(recipe.scenario, scenario);
            assert!(!recipe.steps.is_empty());
            assert!(recipe.max_attempts >= 1);
        }
    }

    #[test]
    fn first_attempt_succeeds() {
        let mut ctx = RecoveryContext::new();
        let result = ctx.attempt_recovery(FailureScenario::ProviderFailure);
        assert_eq!(result, RecoveryResult::Recovered);
    }

    #[test]
    fn second_attempt_escalates() {
        // TrustPromptUnresolved has max_attempts = 1
        let mut ctx = RecoveryContext::new();
        let first = ctx.attempt_recovery(FailureScenario::TrustPromptUnresolved);
        assert_eq!(first, RecoveryResult::Recovered);

        let second = ctx.attempt_recovery(FailureScenario::TrustPromptUnresolved);
        assert_eq!(
            second,
            RecoveryResult::EscalationRequired {
                policy: EscalationPolicy::AlertHuman,
            }
        );
    }

    #[test]
    fn events_are_recorded() {
        let mut ctx = RecoveryContext::new();
        assert!(ctx.events().is_empty());

        ctx.attempt_recovery(FailureScenario::StaleBranch);
        // Should have at least an Attempted + Succeeded event.
        assert!(ctx.events().len() >= 2);
    }

    #[test]
    fn different_scenarios_track_independently() {
        let mut ctx = RecoveryContext::new();

        // Exhaust TrustPromptUnresolved (max 1).
        let _ = ctx.attempt_recovery(FailureScenario::TrustPromptUnresolved);
        let escalated = ctx.attempt_recovery(FailureScenario::TrustPromptUnresolved);
        assert!(matches!(
            escalated,
            RecoveryResult::EscalationRequired { .. }
        ));

        // ProviderFailure should still succeed (independent counter).
        let result = ctx.attempt_recovery(FailureScenario::ProviderFailure);
        assert_eq!(result, RecoveryResult::Recovered);
    }
}
