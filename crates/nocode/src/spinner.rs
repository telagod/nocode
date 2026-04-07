use std::time::Instant;

use crossterm::style::Color;

const FRAMES: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Seconds of silence before we consider the spinner stalled.
const STALL_THRESHOLD_SECS: f64 = 10.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpinnerState {
    Spinning,
    Done,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpinnerMode {
    /// No activity.
    Idle,
    /// Model is thinking.
    Thinking,
    /// Query executing.
    Loading,
    /// No output for 10+ seconds.
    Stalled,
}

#[derive(Debug, Clone)]
pub struct SpinnerFrame {
    pub display: String,
    pub color: Color,
}

#[derive(Debug, Clone)]
pub struct Spinner {
    frames: [char; 10],
    current_frame: usize,
    message: String,
    state: SpinnerState,
    /// Accumulated input tokens.
    pub input_tokens: u64,
    /// Accumulated output tokens.
    pub output_tokens: u64,
    /// When the spinner was created.
    start_time: Instant,
    /// Current operational mode.
    pub mode: SpinnerMode,
    /// Last time output was received.
    last_output_at: Instant,
}

impl Spinner {
    #[must_use]
    pub fn new(message: &str) -> Self {
        let now = Instant::now();
        Self {
            frames: FRAMES,
            current_frame: 0,
            message: message.to_string(),
            state: SpinnerState::Spinning,
            input_tokens: 0,
            output_tokens: 0,
            start_time: now,
            mode: SpinnerMode::Thinking,
            last_output_at: now,
        }
    }

    /// Accumulate token counts and mark output as received.
    pub fn record_tokens(&mut self, input: u64, output: u64) {
        self.input_tokens += input;
        self.output_tokens += output;
        if output > 0 {
            self.last_output_at = Instant::now();
            if self.mode == SpinnerMode::Stalled {
                self.mode = SpinnerMode::Thinking;
            }
        }
    }

    /// Seconds elapsed since the spinner was created.
    #[must_use]
    pub fn elapsed_secs(&self) -> f64 {
        self.start_time.elapsed().as_secs_f64()
    }

    /// If more than 10s since last output, transition to `Stalled`.
    pub fn check_stalled(&mut self) {
        if self.last_output_at.elapsed().as_secs_f64() >= STALL_THRESHOLD_SECS
            && self.mode != SpinnerMode::Stalled
            && self.mode != SpinnerMode::Idle
        {
            self.mode = SpinnerMode::Stalled;
        }
    }

    /// Format a token count for display: `0`, `999`, `1.0K`, `15.3K`, etc.
    #[must_use]
    pub fn format_tokens(n: u64) -> String {
        if n < 1000 {
            n.to_string()
        } else {
            let k = n as f64 / 1000.0;
            format!("{k:.1}K")
        }
    }

    pub fn tick(&mut self) -> SpinnerFrame {
        self.check_stalled();
        let frame_char = self.frames[self.current_frame];
        self.current_frame = (self.current_frame + 1) % self.frames.len();

        let prefix = if self.mode == SpinnerMode::Stalled {
            "\u{26a0} Stalled..."
        } else {
            &self.message
        };

        let elapsed = self.elapsed_secs();
        let elapsed_str = format!("{elapsed:.1}s");
        let in_str = Self::format_tokens(self.input_tokens);
        let out_str = Self::format_tokens(self.output_tokens);

        let display =
            format!("{frame_char} {prefix} {elapsed_str} \u{2502} {in_str} in / {out_str} out");

        let color = if self.mode == SpinnerMode::Stalled {
            Color::Yellow
        } else {
            Color::Blue
        };

        SpinnerFrame { display, color }
    }

    pub fn finish(&mut self, message: &str) {
        self.state = SpinnerState::Done;
        self.message = message.to_string();
        self.mode = SpinnerMode::Idle;
    }

    pub fn fail(&mut self, message: &str) {
        self.state = SpinnerState::Failed;
        self.message = message.to_string();
        self.mode = SpinnerMode::Idle;
    }

    #[must_use]
    pub fn render(&self) -> String {
        match self.state {
            SpinnerState::Spinning => {
                let frame_char = self.frames[self.current_frame];
                format!("{frame_char} {}", self.message)
            }
            SpinnerState::Done => format!("\u{2714} {}", self.message),
            SpinnerState::Failed => format!("\u{2718} {}", self.message),
        }
    }

    #[must_use]
    pub fn is_done(&self) -> bool {
        matches!(self.state, SpinnerState::Done | SpinnerState::Failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_cycles_through_all_frames() {
        let mut spinner = Spinner::new("loading");

        for (i, frame_char) in FRAMES.iter().enumerate() {
            let frame = spinner.tick();
            // tick now includes elapsed + tokens, so check prefix
            assert!(
                frame.display.starts_with(&format!("{frame_char} loading")),
                "frame {i}: got {}",
                frame.display
            );
            assert_eq!(frame.color, Color::Blue);
        }

        // Wraps back to first frame
        let wrapped = spinner.tick();
        assert!(
            wrapped
                .display
                .starts_with(&format!("{} loading", FRAMES[0]))
        );
    }

    #[test]
    fn finish_sets_done_state() {
        let mut spinner = Spinner::new("working");
        assert!(!spinner.is_done());

        spinner.finish("complete");
        assert!(spinner.is_done());
        assert_eq!(spinner.state, SpinnerState::Done);
        assert_eq!(spinner.mode, SpinnerMode::Idle);
        assert_eq!(spinner.render(), "\u{2714} complete");
    }

    #[test]
    fn fail_sets_failed_state() {
        let mut spinner = Spinner::new("working");
        assert!(!spinner.is_done());

        spinner.fail("broken");
        assert!(spinner.is_done());
        assert_eq!(spinner.state, SpinnerState::Failed);
        assert_eq!(spinner.mode, SpinnerMode::Idle);
        assert_eq!(spinner.render(), "\u{2718} broken");
    }

    #[test]
    fn render_spinning_shows_current_frame() {
        let mut spinner = Spinner::new("test");
        assert_eq!(spinner.render(), format!("{} test", FRAMES[0]));

        let _ = spinner.tick();
        assert_eq!(spinner.render(), format!("{} test", FRAMES[1]));
    }

    #[test]
    fn render_done_shows_checkmark() {
        let mut spinner = Spinner::new("task");
        spinner.finish("done");
        assert!(spinner.render().starts_with('\u{2714}'));
    }

    #[test]
    fn render_failed_shows_cross() {
        let mut spinner = Spinner::new("task");
        spinner.fail("error");
        assert!(spinner.render().starts_with('\u{2718}'));
    }

    // --- New tests ---

    #[test]
    fn format_tokens_zero() {
        assert_eq!(Spinner::format_tokens(0), "0");
    }

    #[test]
    fn format_tokens_below_thousand() {
        assert_eq!(Spinner::format_tokens(999), "999");
    }

    #[test]
    fn format_tokens_exactly_thousand() {
        assert_eq!(Spinner::format_tokens(1000), "1.0K");
    }

    #[test]
    fn format_tokens_fifteen_hundred() {
        assert_eq!(Spinner::format_tokens(1500), "1.5K");
    }

    #[test]
    fn format_tokens_ten_thousand() {
        assert_eq!(Spinner::format_tokens(10000), "10.0K");
    }

    #[test]
    fn record_tokens_accumulates() {
        let mut spinner = Spinner::new("test");
        spinner.record_tokens(100, 50);
        assert_eq!(spinner.input_tokens, 100);
        assert_eq!(spinner.output_tokens, 50);

        spinner.record_tokens(200, 30);
        assert_eq!(spinner.input_tokens, 300);
        assert_eq!(spinner.output_tokens, 80);
    }

    #[test]
    fn elapsed_secs_is_non_negative() {
        let spinner = Spinner::new("test");
        assert!(spinner.elapsed_secs() >= 0.0);
    }

    #[test]
    fn tick_display_includes_token_counts() {
        let mut spinner = Spinner::new("Thinking");
        spinner.record_tokens(1500, 300);
        let frame = spinner.tick();
        assert!(
            frame.display.contains("1.5K in"),
            "expected token display, got: {}",
            frame.display
        );
        assert!(
            frame.display.contains("300 out"),
            "expected token display, got: {}",
            frame.display
        );
    }

    #[test]
    fn tick_display_includes_elapsed() {
        let mut spinner = Spinner::new("Thinking");
        let frame = spinner.tick();
        // Should contain something like "0.0s"
        assert!(
            frame.display.contains('s'),
            "expected elapsed time, got: {}",
            frame.display
        );
    }

    #[test]
    fn stalled_detection_via_manual_time() {
        let mut spinner = Spinner::new("Thinking");
        // Force last_output_at to 11 seconds ago
        spinner.last_output_at = Instant::now() - std::time::Duration::from_secs(11);
        spinner.check_stalled();
        assert_eq!(spinner.mode, SpinnerMode::Stalled);
    }

    #[test]
    fn stalled_mode_shows_warning_prefix() {
        let mut spinner = Spinner::new("Thinking");
        spinner.last_output_at = Instant::now() - std::time::Duration::from_secs(11);
        let frame = spinner.tick();
        assert!(
            frame.display.contains("Stalled"),
            "expected stalled prefix, got: {}",
            frame.display
        );
        assert_eq!(frame.color, Color::Yellow);
    }

    #[test]
    fn stalled_recovers_on_output() {
        let mut spinner = Spinner::new("Thinking");
        spinner.last_output_at = Instant::now() - std::time::Duration::from_secs(11);
        spinner.check_stalled();
        assert_eq!(spinner.mode, SpinnerMode::Stalled);

        // Receiving output should recover
        spinner.record_tokens(0, 10);
        assert_eq!(spinner.mode, SpinnerMode::Thinking);
    }

    #[test]
    fn mode_transitions() {
        let mut spinner = Spinner::new("test");
        assert_eq!(spinner.mode, SpinnerMode::Thinking);

        spinner.finish("done");
        assert_eq!(spinner.mode, SpinnerMode::Idle);

        let mut spinner2 = Spinner::new("test2");
        spinner2.fail("err");
        assert_eq!(spinner2.mode, SpinnerMode::Idle);
    }

    #[test]
    fn new_preserves_backward_compat() {
        let spinner = Spinner::new("hello");
        assert_eq!(spinner.input_tokens, 0);
        assert_eq!(spinner.output_tokens, 0);
        assert_eq!(spinner.mode, SpinnerMode::Thinking);
        assert!(!spinner.is_done());
    }
}
