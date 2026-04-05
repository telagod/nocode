pub use crate::query_loop::TaskBudget;

use crate::budget::{BudgetCompletionEvent, TokenBudgetDecision};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetState {
    pub task_budget: Option<TaskBudget>,
    pub current_turn_budget: Option<u64>,
    pub output_tokens_at_turn_start: u64,
    pub current_turn_output_tokens: u64,
    pub last_decision: Option<TokenBudgetDecision>,
    pub last_completion_event: Option<BudgetCompletionEvent>,
}

impl BudgetState {
    pub fn new(task_budget: Option<TaskBudget>) -> Self {
        Self {
            task_budget,
            current_turn_budget: task_budget.map(|budget| u64::from(budget.total)),
            output_tokens_at_turn_start: 0,
            current_turn_output_tokens: 0,
            last_decision: None,
            last_completion_event: None,
        }
    }

    pub fn begin_turn(&mut self, total_output_tokens: u64) {
        self.output_tokens_at_turn_start = total_output_tokens;
        self.current_turn_output_tokens = 0;
        self.current_turn_budget = self.task_budget.map(|budget| u64::from(budget.total));
        self.last_decision = None;
        self.last_completion_event = None;
    }

    pub fn sync_turn_output_tokens(&mut self, total_output_tokens: u64) {
        self.current_turn_output_tokens =
            total_output_tokens.saturating_sub(self.output_tokens_at_turn_start);
    }

    pub fn record_decision(&mut self, decision: Option<TokenBudgetDecision>) {
        self.last_completion_event = match &decision {
            Some(TokenBudgetDecision::Stop { completion_event }) => completion_event.clone(),
            _ => None,
        };
        self.last_decision = decision;
    }

    pub fn continuation_count(&self) -> u32 {
        match &self.last_decision {
            Some(TokenBudgetDecision::Continue {
                continuation_count, ..
            }) => *continuation_count,
            Some(TokenBudgetDecision::Stop {
                completion_event: Some(event),
            }) => event.continuation_count,
            _ => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BudgetState, TaskBudget};
    use crate::budget::{BudgetCompletionEvent, TokenBudgetDecision};

    #[test]
    fn continue_decision_tracks_continuation_count() {
        let mut state = BudgetState::new(Some(TaskBudget { total: 10_000 }));
        state.begin_turn(100);
        state.sync_turn_output_tokens(1_250);
        state.record_decision(Some(TokenBudgetDecision::Continue {
            nudge_message: String::from("continue"),
            continuation_count: 2,
            pct: 25,
            turn_tokens: 2_500,
            budget: 10_000,
        }));

        assert_eq!(state.continuation_count(), 2);
        assert!(state.last_completion_event.is_none());
        assert_eq!(state.current_turn_budget, Some(10_000));
        assert_eq!(state.current_turn_output_tokens, 1_150);
    }

    #[test]
    fn stop_decision_keeps_completion_event() {
        let mut state = BudgetState::new(Some(TaskBudget { total: 10_000 }));
        state.begin_turn(500);
        state.sync_turn_output_tokens(9_000);
        state.record_decision(Some(TokenBudgetDecision::Stop {
            completion_event: Some(BudgetCompletionEvent {
                continuation_count: 3,
                pct: 85,
                turn_tokens: 8_500,
                budget: 10_000,
                diminishing_returns: true,
                duration_ms: 12_000,
            }),
        }));

        assert_eq!(state.continuation_count(), 3);
        assert_eq!(
            state
                .last_completion_event
                .as_ref()
                .map(|event| event.duration_ms),
            Some(12_000)
        );
        assert_eq!(state.current_turn_output_tokens, 8_500);
    }

    #[test]
    fn new_without_budget_has_no_turn_budget() {
        let state = BudgetState::new(None);
        assert!(state.task_budget.is_none());
        assert!(state.current_turn_budget.is_none());
        assert_eq!(state.output_tokens_at_turn_start, 0);
        assert_eq!(state.current_turn_output_tokens, 0);
        assert!(state.last_decision.is_none());
        assert!(state.last_completion_event.is_none());
        assert_eq!(state.continuation_count(), 0);
    }

    #[test]
    fn begin_turn_resets_per_turn_state() {
        let mut state = BudgetState::new(Some(TaskBudget { total: 5_000 }));
        // Simulate some prior state
        state.sync_turn_output_tokens(2_000);
        state.record_decision(Some(TokenBudgetDecision::Continue {
            nudge_message: String::from("go"),
            continuation_count: 1,
            pct: 40,
            turn_tokens: 2_000,
            budget: 5_000,
        }));
        assert_eq!(state.continuation_count(), 1);

        // begin_turn should reset
        state.begin_turn(3_000);
        assert_eq!(state.output_tokens_at_turn_start, 3_000);
        assert_eq!(state.current_turn_output_tokens, 0);
        assert!(state.last_decision.is_none());
        assert!(state.last_completion_event.is_none());
        assert_eq!(state.continuation_count(), 0);
        assert_eq!(state.current_turn_budget, Some(5_000));
    }

    #[test]
    fn sync_turn_output_tokens_saturates_at_zero() {
        let mut state = BudgetState::new(Some(TaskBudget { total: 1_000 }));
        state.begin_turn(500);
        // total < start => saturating_sub yields 0
        state.sync_turn_output_tokens(200);
        assert_eq!(state.current_turn_output_tokens, 0);
    }

    #[test]
    fn stop_without_completion_event_yields_zero_continuation() {
        let mut state = BudgetState::new(None);
        state.record_decision(Some(TokenBudgetDecision::Stop {
            completion_event: None,
        }));
        assert_eq!(state.continuation_count(), 0);
        assert!(state.last_completion_event.is_none());
    }
}
