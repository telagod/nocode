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

    /// Milliseconds elapsed since the current turn started, if any.
    #[must_use]
    pub fn elapsed_ms(&self) -> Option<u64> {
        self.turn_start
            .map(|start| start.elapsed().as_millis() as u64)
    }

    /// Render a single-line status bar suitable for a TUI footer.
    #[must_use]
    pub fn render_line(&self) -> String {
        let id_short = truncate_session_id(&self.session_id);
        let elapsed = self
            .elapsed_ms()
            .map_or_else(|| String::from("--"), |ms| format!("{ms}"));
        format!(
            "model: {} | turn: {}\u{2193} {}\u{2191} | total: {}\u{2193} {}\u{2191} | {}ms | session: {}",
            self.model_name,
            self.turn_input_tokens,
            self.turn_output_tokens,
            self.cumulative_input_tokens,
            self.cumulative_output_tokens,
            elapsed,
            id_short,
        )
    }

    /// Render a single-line status bar for streaming (always includes elapsed).
    #[must_use]
    pub fn render_line_streaming(&self) -> String {
        let id_short = truncate_session_id(&self.session_id);
        let elapsed = self
            .elapsed_ms()
            .map_or_else(|| String::from("--"), |ms| format!("{ms}"));
        format!(
            "model: {} | turn: {}\u{2193} {}\u{2191} | total: {}\u{2193} {}\u{2191} | {}ms | session: {}",
            self.model_name,
            self.turn_input_tokens,
            self.turn_output_tokens,
            self.cumulative_input_tokens,
            self.cumulative_output_tokens,
            elapsed,
            id_short,
        )
    }
}

fn truncate_session_id(id: &str) -> &str {
    if id.len() > 8 { &id[..8] } else { id }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_initialises_zeroed_tokens() {
        let hud = StatusHud::new("gpt-4", "abcdef1234567890");
        assert_eq!(hud.model_name, "gpt-4");
        assert_eq!(hud.turn_input_tokens, 0);
        assert_eq!(hud.turn_output_tokens, 0);
        assert_eq!(hud.cumulative_input_tokens, 0);
        assert_eq!(hud.cumulative_output_tokens, 0);
        assert!(hud.turn_start.is_none());
        assert_eq!(hud.session_id, "abcdef1234567890");
    }

    #[test]
    fn record_tokens_accumulates_turn_and_cumulative() {
        let mut hud = StatusHud::new("sonnet", "sess-001");
        hud.record_tokens(100, 50);
        assert_eq!(hud.turn_input_tokens, 100);
        assert_eq!(hud.turn_output_tokens, 50);
        assert_eq!(hud.cumulative_input_tokens, 100);
        assert_eq!(hud.cumulative_output_tokens, 50);

        hud.record_tokens(200, 80);
        assert_eq!(hud.turn_input_tokens, 300);
        assert_eq!(hud.turn_output_tokens, 130);
        assert_eq!(hud.cumulative_input_tokens, 300);
        assert_eq!(hud.cumulative_output_tokens, 130);
    }

    #[test]
    fn start_turn_resets_turn_tokens_and_sets_instant() {
        let mut hud = StatusHud::new("sonnet", "sess-002");
        hud.record_tokens(500, 200);
        hud.start_turn();

        assert_eq!(hud.turn_input_tokens, 0);
        assert_eq!(hud.turn_output_tokens, 0);
        assert!(hud.turn_start.is_some());
        // Cumulative must survive across turns.
        assert_eq!(hud.cumulative_input_tokens, 500);
        assert_eq!(hud.cumulative_output_tokens, 200);
    }

    #[test]
    fn end_turn_clears_turn_start() {
        let mut hud = StatusHud::new("sonnet", "sess-003");
        hud.start_turn();
        assert!(hud.turn_start.is_some());
        hud.end_turn();
        assert!(hud.turn_start.is_none());
    }

    #[test]
    fn elapsed_ms_none_when_no_turn() {
        let hud = StatusHud::new("sonnet", "sess-004");
        assert!(hud.elapsed_ms().is_none());
    }

    #[test]
    fn elapsed_ms_some_during_turn() {
        let mut hud = StatusHud::new("sonnet", "sess-005");
        hud.start_turn();
        // Elapsed should be Some and non-negative (could be 0 on fast machines).
        assert!(hud.elapsed_ms().is_some());
    }

    #[test]
    fn render_line_format_without_active_turn() {
        let mut hud = StatusHud::new("claude-opus", "abcdefgh99999999");
        hud.record_tokens(1000, 400);
        let line = hud.render_line();

        assert!(line.contains("model: claude-opus"));
        assert!(line.contains("turn: 1000\u{2193} 400\u{2191}"));
        assert!(line.contains("total: 1000\u{2193} 400\u{2191}"));
        assert!(line.contains("--ms"));
        assert!(line.contains("session: abcdefgh"));
        // Must NOT contain the full session id.
        assert!(!line.contains("abcdefgh99999999"));
    }

    #[test]
    fn render_line_streaming_includes_elapsed() {
        let mut hud = StatusHud::new("claude-opus", "abcdefgh99999999");
        hud.start_turn();
        hud.record_tokens(50, 10);
        let line = hud.render_line_streaming();

        assert!(line.contains("model: claude-opus"));
        assert!(line.contains("turn: 50\u{2193} 10\u{2191}"));
        assert!(line.contains("ms"));
        assert!(line.contains("session: abcdefgh"));
    }

    #[test]
    fn full_turn_lifecycle() {
        let mut hud = StatusHud::new("haiku", "lifecycle-session-id");

        // Turn 1
        hud.start_turn();
        hud.record_tokens(100, 50);
        hud.record_tokens(100, 50);
        hud.end_turn();

        assert_eq!(hud.turn_input_tokens, 200);
        assert_eq!(hud.turn_output_tokens, 100);
        assert_eq!(hud.cumulative_input_tokens, 200);
        assert_eq!(hud.cumulative_output_tokens, 100);

        // Turn 2 — turn tokens reset, cumulative keeps growing.
        hud.start_turn();
        assert_eq!(hud.turn_input_tokens, 0);
        assert_eq!(hud.turn_output_tokens, 0);
        hud.record_tokens(300, 150);
        hud.end_turn();

        assert_eq!(hud.turn_input_tokens, 300);
        assert_eq!(hud.turn_output_tokens, 150);
        assert_eq!(hud.cumulative_input_tokens, 500);
        assert_eq!(hud.cumulative_output_tokens, 250);
    }

    #[test]
    fn truncate_session_id_short_input() {
        assert_eq!(truncate_session_id("abc"), "abc");
        assert_eq!(truncate_session_id("12345678"), "12345678");
        assert_eq!(truncate_session_id("123456789"), "12345678");
    }
}
