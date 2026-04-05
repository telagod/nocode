use nocode_core::model_pricing::{estimate_cost, format_usd};
use std::time::Instant;

/// TUI status bar HUD — tracks model name, token usage, and turn timing.
#[derive(Debug)]
pub struct StatusHud {
    pub model_name: String,
    pub turn_input_tokens: u64,
    pub turn_output_tokens: u64,
    pub cumulative_input_tokens: u64,
    pub cumulative_output_tokens: u64,
    pub turn_start: Option<Instant>,
    pub session_id: String,
}

impl StatusHud {
    #[must_use]
    pub fn new(model_name: &str, session_id: &str) -> Self {
        Self {
            model_name: model_name.to_owned(),
            turn_input_tokens: 0,
            turn_output_tokens: 0,
            cumulative_input_tokens: 0,
            cumulative_output_tokens: 0,
            turn_start: None,
            session_id: session_id.to_owned(),
        }
    }

    /// Begin a new turn: record the start time and reset per-turn token counts.
    pub fn start_turn(&mut self) {
        self.turn_start = Some(Instant::now());
        self.turn_input_tokens = 0;
        self.turn_output_tokens = 0;
    }

    /// Accumulate token counts for the current turn and the cumulative totals.
    pub fn record_tokens(&mut self, input: u64, output: u64) {
        self.turn_input_tokens += input;
        self.turn_output_tokens += output;
        self.cumulative_input_tokens += input;
        self.cumulative_output_tokens += output;
    }

    /// Mark the current turn as finished.
    pub fn end_turn(&mut self) {
        self.turn_start = None;
    }
