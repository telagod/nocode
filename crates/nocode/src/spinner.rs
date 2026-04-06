use crossterm::style::Color;

const FRAMES: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpinnerState {
    Spinning,
    Done,
    Failed,
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
}

impl Spinner {
    #[must_use]
    pub fn new(message: &str) -> Self {
        Self {
            frames: FRAMES,
            current_frame: 0,
            message: message.to_string(),
            state: SpinnerState::Spinning,
        }
    }

    pub fn tick(&mut self) -> SpinnerFrame {
        let frame_char = self.frames[self.current_frame];
        let frame = SpinnerFrame {
            display: format!("{frame_char} {}", self.message),
            color: Color::Blue,
        };
        self.current_frame = (self.current_frame + 1) % self.frames.len();
        frame
    }

    pub fn finish(&mut self, message: &str) {
        self.state = SpinnerState::Done;
        self.message = message.to_string();
    }

    pub fn fail(&mut self, message: &str) {
        self.state = SpinnerState::Failed;
        self.message = message.to_string();
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
        let expected: Vec<String> = FRAMES.iter().map(|ch| format!("{ch} loading")).collect();

        for expected_display in &expected {
            let frame = spinner.tick();
            assert_eq!(&frame.display, expected_display);
            assert_eq!(frame.color, Color::Blue);
        }

        // Wraps back to first frame
        let wrapped = spinner.tick();
        assert_eq!(wrapped.display, format!("{} loading", FRAMES[0]));
    }

    #[test]
    fn finish_sets_done_state() {
        let mut spinner = Spinner::new("working");
        assert!(!spinner.is_done());

        spinner.finish("complete");
        assert!(spinner.is_done());
        assert_eq!(spinner.state, SpinnerState::Done);
        assert_eq!(spinner.render(), "\u{2714} complete");
    }

    #[test]
    fn fail_sets_failed_state() {
        let mut spinner = Spinner::new("working");
        assert!(!spinner.is_done());

        spinner.fail("broken");
        assert!(spinner.is_done());
        assert_eq!(spinner.state, SpinnerState::Failed);
        assert_eq!(spinner.render(), "\u{2718} broken");
    }

    #[test]
    fn render_spinning_shows_current_frame() {
        let mut spinner = Spinner::new("test");
        // Before any tick, current_frame is 0
        assert_eq!(spinner.render(), format!("{} test", FRAMES[0]));

        // After one tick, current_frame advances to 1
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
}
