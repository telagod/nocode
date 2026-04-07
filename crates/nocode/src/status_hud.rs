use nocode_core::model_pricing::{estimate_cost, format_usd};
use std::time::Instant;

/// TUI status bar HUD — tracks model name, token usage, turn timing, and session metadata.
#[derive(Debug)]
pub struct StatusHud {
    pub model_name: String,
    pub turn_input_tokens: u64,
    pub turn_output_tokens: u64,
    pub cumulative_input_tokens: u64,
    pub cumulative_output_tokens: u64,
    pub turn_start: Option<Instant>,
    pub session_id: String,
    pub permission_mode: String,
    pub session_name: String,
    pub context_window_pct: f32,
    pub cost_usd: f64,
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
            permission_mode: String::new(),
            session_name: String::new(),
            context_window_pct: 0.0,
            cost_usd: 0.0,
        }
    }

    /// Set the current permission mode label.
    pub fn set_permission_mode(&mut self, mode: &str) {
        self.permission_mode = mode.to_owned();
    }

    /// Set the session name / identifier.
    pub fn set_session_name(&mut self, name: &str) {
        self.session_name = name.to_owned();
    }

    /// Set the context window usage percentage (0.0–100.0).
    pub fn set_context_window_pct(&mut self, pct: f32) {
        self.context_window_pct = pct.clamp(0.0, 100.0);
    }

    /// Add to the accumulated cost in USD.
    pub fn add_cost(&mut self, usd: f64) {
        self.cost_usd += usd;
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

    /// Render a compact single-line status bar suitable for a TUI footer.
    ///
    /// Format: `model │ tokens: 1.2K in / 0.3K out │ cost: $0.05 │ ctx: 12% │ mode: workspace │ 1.2s`
    #[must_use]
    pub fn render_line(&self) -> String {
        self.render_compact_line()
    }

    /// Render a single-line status bar for streaming (same compact format).
    #[must_use]
    pub fn render_line_streaming(&self) -> String {
        self.render_compact_line()
    }

    /// Shared compact renderer used by both `render_line` and `render_line_streaming`.
    fn render_compact_line(&self) -> String {
        let sep = " \u{2502} ";
        let in_tok = format_tokens(self.cumulative_input_tokens);
        let out_tok = format_tokens(self.cumulative_output_tokens);
        let cost = self.format_cost();
        let ctx = format!("{:.0}%", self.context_window_pct);
        let mode = shorten_permission_mode(&self.permission_mode);
        let elapsed = self.format_elapsed();

        let session = if self.session_name.is_empty() {
            truncate_session_id(&self.session_id)
        } else {
            truncate_session_id(&self.session_name)
        };

        format!(
            "model: {}{sep}tokens: {in_tok} in / {out_tok} out{sep}cost: {cost}{sep}ctx: {ctx}{sep}mode: {mode}{sep}{elapsed}{sep}session: {session}",
            self.model_name,
        )
    }

    /// Format elapsed time as human-friendly string (seconds with 1 decimal).
    fn format_elapsed(&self) -> String {
        self.elapsed_ms().map_or_else(
            || String::from("--"),
            |ms| {
                let secs = ms as f64 / 1000.0;
                format!("{secs:.1}s")
            },
        )
    }

    /// Compute formatted cost string from cumulative tokens and model name.
    fn format_cost(&self) -> String {
        if self.model_name.is_empty() {
            return "$0.00".to_string();
        }
        let est = estimate_cost(
            &self.model_name,
            self.cumulative_input_tokens,
            self.cumulative_output_tokens,
            0,
            0,
        );
        format_usd(est.total)
    }
}

/// Format a token count as a compact string (e.g. 1200 → "1.2K", 500 → "500").
fn format_tokens(count: u64) -> String {
    if count >= 1_000_000 {
        let m = count as f64 / 1_000_000.0;
        format!("{m:.1}M")
    } else if count >= 1000 {
        let k = count as f64 / 1000.0;
        format!("{k:.1}K")
    } else {
        count.to_string()
    }
}

/// Shorten a permission mode string for compact display.
fn shorten_permission_mode(mode: &str) -> &str {
    match mode {
        "workspace_write" | "WorkspaceWrite" => "workspace",
        "read_only" | "ReadOnly" => "readonly",
        "danger_full_access" | "DangerFullAccess" => "full",
        "" => "default",
        other => other,
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
        assert_eq!(hud.permission_mode, "");
        assert_eq!(hud.session_name, "");
        assert!((hud.context_window_pct - 0.0).abs() < f32::EPSILON);
        assert!((hud.cost_usd - 0.0).abs() < f64::EPSILON);
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
        assert!(line.contains("tokens: 1.0K in / 400 out"));
        assert!(line.contains("cost: $"));
        assert!(line.contains("--"));
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
        assert!(line.contains("tokens: 50 in / 10 out"));
        assert!(line.contains("cost: $"));
        assert!(line.contains("s")); // elapsed contains seconds
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
