//! Token budget tracking with diminishing returns logic.

/// Tracks token usage across turns and enforces budget limits.
#[derive(Debug, Clone)]
pub struct TokenBudget {
    /// Maximum total tokens (input + output) per session.
    pub max_total_tokens: u64,
    /// Maximum tokens per single turn.
    pub max_turn_tokens: u64,
    /// Accumulated input tokens.
    pub total_input: u64,
    /// Accumulated output tokens.
    pub total_output: u64,
    /// Number of turns completed.
    pub turns: u32,
    /// Compaction threshold — trigger compaction when usage exceeds this fraction.
    pub compaction_threshold: f64,
}

impl TokenBudget {
    pub fn new(max_total: u64, max_turn: u64) -> Self {
        Self {
            max_total_tokens: max_total,
            max_turn_tokens: max_turn,
            total_input: 0,
            total_output: 0,
            turns: 0,
            compaction_threshold: 0.8,
        }
    }

    /// Record token usage for a turn.
    pub fn record(&mut self, input: u64, output: u64) {
        self.total_input += input;
        self.total_output += output;
        self.turns += 1;
    }

    /// Total tokens consumed so far.
    pub fn total_tokens(&self) -> u64 {
        self.total_input + self.total_output
    }

    /// Fraction of budget consumed (0.0 to 1.0+).
    pub fn usage_fraction(&self) -> f64 {
        if self.max_total_tokens == 0 {
            return 0.0;
        }
        self.total_tokens() as f64 / self.max_total_tokens as f64
    }

    /// Whether compaction should be triggered.
    pub fn needs_compaction(&self) -> bool {
        self.usage_fraction() >= self.compaction_threshold
    }

    /// Whether the budget is exhausted.
    pub fn is_exhausted(&self) -> bool {
        self.total_tokens() >= self.max_total_tokens
    }

    /// Remaining tokens available.
    pub fn remaining(&self) -> u64 {
        self.max_total_tokens.saturating_sub(self.total_tokens())
    }

    /// Effective max_tokens for the next turn, applying diminishing returns.
    /// As budget depletes, reduce per-turn allocation to extend session life.
    pub fn effective_max_tokens(&self) -> u64 {
        let remaining_frac = 1.0 - self.usage_fraction().min(1.0);
        let diminished = (self.max_turn_tokens as f64 * remaining_frac.sqrt()) as u64;
        diminished.max(1024).min(self.max_turn_tokens)
    }

    /// Context window usage percentage (0.0 to 100.0).
    pub fn context_window_pct(&self) -> f32 {
        (self.usage_fraction() * 100.0) as f32
    }
}

impl Default for TokenBudget {
    fn default() -> Self {
        Self::new(200_000, 16_384)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_usage() {
        let mut b = TokenBudget::new(100_000, 8192);
        b.record(5000, 2000);
        assert_eq!(b.total_tokens(), 7000);
        assert_eq!(b.turns, 1);
        assert!(!b.is_exhausted());
    }

    #[test]
    fn detects_exhaustion() {
        let mut b = TokenBudget::new(10_000, 4096);
        b.record(6000, 5000);
        assert!(b.is_exhausted());
        assert_eq!(b.remaining(), 0);
    }

    #[test]
    fn compaction_threshold() {
        let mut b = TokenBudget::new(100_000, 8192);
        b.record(40_000, 41_000);
        assert!(b.needs_compaction());
    }

    #[test]
    fn diminishing_returns() {
        let b1 = TokenBudget::new(100_000, 16_384);
        assert_eq!(b1.effective_max_tokens(), 16_384);

        let mut b2 = TokenBudget::new(100_000, 16_384);
        b2.record(90_000, 0);
        assert!(b2.effective_max_tokens() < 16_384);
        assert!(b2.effective_max_tokens() >= 1024);
    }

    #[test]
    fn context_window_pct() {
        let mut b = TokenBudget::new(100_000, 8192);
        b.record(42_000, 0);
        let pct = b.context_window_pct();
        assert!((pct - 42.0).abs() < 0.1);
    }
}
