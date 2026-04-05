use crate::message::QueryMessage;

const COMPLETION_THRESHOLD_NUMERATOR: u64 = 9;
const COMPLETION_THRESHOLD_DENOMINATOR: u64 = 10;
const DIMINISHING_THRESHOLD: u64 = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetTracker {
    pub continuation_count: u32,
    pub last_delta_tokens: u64,
    pub last_global_turn_tokens: u64,
    pub started_at_ms: u64,
}

impl BudgetTracker {
    pub fn new(started_at_ms: u64) -> Self {
        Self {
            continuation_count: 0,
            last_delta_tokens: 0,
            last_global_turn_tokens: 0,
            started_at_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetCompletionEvent {
    pub continuation_count: u32,
    pub pct: u32,
    pub turn_tokens: u64,
    pub budget: u64,
    pub diminishing_returns: bool,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenBudgetDecision {
    Continue {
        nudge_message: String,
        continuation_count: u32,
        pct: u32,
        turn_tokens: u64,
        budget: u64,
    },
    Stop {
        completion_event: Option<BudgetCompletionEvent>,
    },
}

impl TokenBudgetDecision {
    pub const fn action_label(&self) -> &'static str {
        match self {
            Self::Continue { .. } => "continue",
            Self::Stop { .. } => "stop",
        }
    }
}

pub fn estimate_turn_tokens(messages: &[QueryMessage]) -> u64 {
    messages
        .iter()
        .map(|message| message.content.split_whitespace().count() as u64)
        .sum()
}

fn budget_continuation_message(pct: u32, turn_tokens: u64, budget: u64) -> String {
    format!("token budget continuation: {pct}% ({turn_tokens}/{budget}) - continue current turn")
}

pub fn check_token_budget(
    tracker: &mut BudgetTracker,
    agent_id: Option<&str>,
    budget: Option<u64>,
    global_turn_tokens: u64,
    now_ms: u64,
) -> TokenBudgetDecision {
    let Some(budget) = budget else {
        return TokenBudgetDecision::Stop {
            completion_event: None,
        };
    };

    if agent_id.is_some() || budget == 0 {
        return TokenBudgetDecision::Stop {
            completion_event: None,
        };
    }

    let pct = ((global_turn_tokens * 100) / budget) as u32;
    let delta_since_last_check = global_turn_tokens.saturating_sub(tracker.last_global_turn_tokens);
    let is_diminishing = tracker.continuation_count >= 3
        && delta_since_last_check < DIMINISHING_THRESHOLD
        && tracker.last_delta_tokens < DIMINISHING_THRESHOLD;
    let below_threshold = global_turn_tokens * COMPLETION_THRESHOLD_DENOMINATOR
        < budget * COMPLETION_THRESHOLD_NUMERATOR;

    if !is_diminishing && below_threshold {
        tracker.continuation_count += 1;
        tracker.last_delta_tokens = delta_since_last_check;
        tracker.last_global_turn_tokens = global_turn_tokens;
        return TokenBudgetDecision::Continue {
            nudge_message: budget_continuation_message(pct, global_turn_tokens, budget),
            continuation_count: tracker.continuation_count,
            pct,
            turn_tokens: global_turn_tokens,
            budget,
        };
    }

    if is_diminishing || tracker.continuation_count > 0 {
        return TokenBudgetDecision::Stop {
            completion_event: Some(BudgetCompletionEvent {
                continuation_count: tracker.continuation_count,
                pct,
                turn_tokens: global_turn_tokens,
                budget,
                diminishing_returns: is_diminishing,
                duration_ms: now_ms.saturating_sub(tracker.started_at_ms),
            }),
        };
    }

    TokenBudgetDecision::Stop {
        completion_event: None,
    }
}

#[cfg(test)]
mod tests {
    use super::{BudgetTracker, TokenBudgetDecision, check_token_budget, estimate_turn_tokens};
    use crate::message::QueryMessage;

    #[test]
    fn no_budget_means_stop_without_event() {
        let mut tracker = BudgetTracker::new(100);
        assert_eq!(
            check_token_budget(&mut tracker, None, None, 100, 150),
            TokenBudgetDecision::Stop {
                completion_event: None
            }
        );
    }

    #[test]
    fn below_threshold_continues_and_updates_tracker() {
        let mut tracker = BudgetTracker::new(100);
        let decision = check_token_budget(&mut tracker, None, Some(10_000), 2_000, 200);
        match decision {
            TokenBudgetDecision::Continue {
                continuation_count,
                pct,
                turn_tokens,
                budget,
                ..
            } => {
                assert_eq!(continuation_count, 1);
                assert_eq!(pct, 20);
                assert_eq!(turn_tokens, 2_000);
                assert_eq!(budget, 10_000);
            }
            TokenBudgetDecision::Stop { .. } => panic!("expected continue"),
        }
        assert_eq!(tracker.continuation_count, 1);
        assert_eq!(tracker.last_delta_tokens, 2_000);
        assert_eq!(tracker.last_global_turn_tokens, 2_000);
    }

    #[test]
    fn diminishing_returns_emit_completion_event() {
        let mut tracker = BudgetTracker {
            continuation_count: 3,
            last_delta_tokens: 300,
            last_global_turn_tokens: 4_000,
            started_at_ms: 100,
        };
        let decision = check_token_budget(&mut tracker, None, Some(10_000), 4_200, 600);
        match decision {
            TokenBudgetDecision::Stop {
                completion_event: Some(event),
            } => {
                assert!(event.diminishing_returns);
                assert_eq!(event.continuation_count, 3);
                assert_eq!(event.duration_ms, 500);
            }
            _ => panic!("expected stop with event"),
        }
    }

    #[test]
    fn agent_turns_do_not_auto_continue() {
        let mut tracker = BudgetTracker::new(100);
        assert_eq!(
            check_token_budget(&mut tracker, Some("agent-1"), Some(10_000), 100, 200),
            TokenBudgetDecision::Stop {
                completion_event: None
            }
        );
    }

    #[test]
    fn estimate_turn_tokens_counts_words() {
        let estimate = estimate_turn_tokens(&[
            QueryMessage::user("continue rewrite"),
            QueryMessage::assistant("tool message here"),
        ]);
        assert_eq!(estimate, 5);
    }
}
