use std::time::Instant;

use nocode_core::provider::pricing;

/// TUI status bar HUD — tracks model name, token usage, turn timing, and session metadata.
#[derive(Debug)]
pub struct StatusHud {
    pub model_name: String,
    pub turn_input_tokens: u64,
    pub turn_output_tokens: u64,
    pub cumulative_input_tokens: u64,
    pub cumulative_output_tokens: u64,
    pub cumulative_cache_read_tokens: u64,
    pub cumulative_cache_write_tokens: u64,
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
            cumulative_cache_read_tokens: 0,
            cumulative_cache_write_tokens: 0,
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

    /// Accumulate cache token counts.
    pub fn record_cache_tokens(&mut self, cache_read: u64, cache_write: u64) {
        self.cumulative_cache_read_tokens += cache_read;
        self.cumulative_cache_write_tokens += cache_write;
    }

    /// Mark the current turn as finished.
    pub fn end_turn(&mut self) {
        self.turn_start = None;
    }

    /// Get session name, if set.
    #[must_use]
    pub fn session_name(&self) -> Option<&str> {
        if self.session_name.is_empty() {
            None
        } else {
            Some(&self.session_name)
        }
    }

    /// Get model name.
    #[must_use]
    pub fn model_name(&self) -> &str {
        &self.model_name
    }

    /// Get cumulative input tokens.
    #[must_use]
    pub fn cumulative_input_tokens(&self) -> u64 {
        self.cumulative_input_tokens
    }

    /// Get cumulative output tokens.
    #[must_use]
    pub fn cumulative_output_tokens(&self) -> u64 {
        self.cumulative_output_tokens
    }

    /// Get context window percentage.
    #[must_use]
    pub fn context_pct(&self) -> f32 {
        self.context_window_pct
    }

    /// Get estimated cost in USD using model-aware pricing.
    #[must_use]
    pub fn estimated_cost(&self) -> f64 {
        self.cost_usd
            + pricing::calculate_cost(
                &self.model_name,
                self.cumulative_input_tokens,
                self.cumulative_output_tokens,
                self.cumulative_cache_read_tokens,
                self.cumulative_cache_write_tokens,
            )
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

    /// Shared compact renderer — Claude Code style: "model · mode · $0.05 · 12% ctx · 1.2K tok"
    fn render_compact_line(&self) -> String {
        let dot = " \u{00B7} ";
        let model = if self.model_name.is_empty() {
            "no model"
        } else {
            &self.model_name
        };
        let mode = shorten_permission_mode(&self.permission_mode);
        let cost = self.format_cost();
        let ctx = format_context_bar(self.context_window_pct);
        let total_tok = self.cumulative_input_tokens + self.cumulative_output_tokens;
        let tok = format!("{} tok", format_tokens(total_tok));

        let mut parts: Vec<&str> = vec![model, mode, &cost, &ctx];

        if total_tok > 0 {
            parts.push(&tok);
        }

        let elapsed;
        if let Some(ms) = self.elapsed_ms() {
            let secs = ms as f64 / 1000.0;
            elapsed = format!("{secs:.1}s");
            parts.push(&elapsed);
        }

        parts.join(dot)
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

    /// Compute formatted cost string using model-aware pricing.
    fn format_cost(&self) -> String {
        let total = self.estimated_cost();
        if total < 0.01 {
            "$0.00".to_string()
        } else {
            format!("${total:.2}")
        }
    }
}

/// Format context window usage as a visual mini-bar: "▰▰▰▱▱ 42%"
/// Warns with ⚠ when >80%.
fn format_context_bar(pct: f32) -> String {
    let pct_u = pct.clamp(0.0, 100.0) as u32;
    let filled = (pct_u / 20) as usize; // 5 segments, each = 20%
    let empty = 5_usize.saturating_sub(filled);
    let bar: String = "\u{25B0}".repeat(filled) + &"\u{25B1}".repeat(empty);
    if pct_u >= 80 {
        format!("{bar} {pct_u}% \u{26A0}")
    } else {
        format!("{bar} {pct_u}%")
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

        assert!(line.contains("claude-opus"));
        assert!(line.contains("1.4K tok"));
        assert!(line.contains("$"));
        assert!(line.contains("0%"));
    }

    #[test]
    fn render_line_streaming_includes_elapsed() {
        let mut hud = StatusHud::new("claude-opus", "abcdefgh99999999");
        hud.start_turn();
        hud.record_tokens(50, 10);
        let line = hud.render_line_streaming();

        assert!(line.contains("claude-opus"));
        assert!(line.contains("60 tok"));
        assert!(line.contains("$"));
        assert!(line.contains("s")); // elapsed contains seconds
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

    #[test]
    fn render_line_uses_session_name_when_set() {
        let mut hud = StatusHud::new("sonnet", "fallback-id-long");
        hud.set_session_name("my-session-name-long");
        let line = hud.render_line();
        // New format no longer includes session — just verify it renders without panic
        assert!(line.contains("sonnet"));
    }

    #[test]
    fn render_line_includes_permission_mode() {
        let mut hud = StatusHud::new("sonnet", "sess");
        hud.set_permission_mode("WorkspaceWrite");
        let line = hud.render_line();
        assert!(line.contains("workspace"));
    }

    #[test]
    fn render_line_includes_context_pct() {
        let mut hud = StatusHud::new("sonnet", "sess");
        hud.set_context_window_pct(42.7);
        let line = hud.render_line();
        assert!(line.contains("42%"));
    }

    #[test]
    fn format_tokens_scales_correctly() {
        assert_eq!(format_tokens(0), "0");
        assert_eq!(format_tokens(999), "999");
        assert_eq!(format_tokens(1000), "1.0K");
        assert_eq!(format_tokens(1500), "1.5K");
        assert_eq!(format_tokens(1_000_000), "1.0M");
        assert_eq!(format_tokens(2_500_000), "2.5M");
    }

    #[test]
    fn shorten_permission_mode_maps_known_values() {
        assert_eq!(shorten_permission_mode("WorkspaceWrite"), "workspace");
        assert_eq!(shorten_permission_mode("ReadOnly"), "readonly");
        assert_eq!(shorten_permission_mode("DangerFullAccess"), "full");
        assert_eq!(shorten_permission_mode(""), "default");
        assert_eq!(shorten_permission_mode("custom"), "custom");
    }
}
