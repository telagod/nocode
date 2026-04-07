//! Multi-line text input widget for the nocode TUI.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, StatefulWidget, Widget, Wrap};

// ---------------------------------------------------------------------------
// TuiTextInput — state
// ---------------------------------------------------------------------------

/// Multi-line input state with history and cursor tracking.
#[derive(Debug, Clone)]
pub(crate) struct TuiTextInput {
    pub lines: Vec<String>,
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub scroll_offset: usize,
    pub history: Vec<String>,
    pub history_pos: Option<usize>,
}

impl Default for TuiTextInput {
    fn default() -> Self {
        Self::new()
    }
}

impl TuiTextInput {
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cursor_row: 0,
            cursor_col: 0,
            scroll_offset: 0,
            history: Vec::new(),
            history_pos: None,
        }
    }

    /// Insert a character at the current cursor position.
    pub fn insert_char(&mut self, c: char) {
        let line = &mut self.lines[self.cursor_row];
        let byte_idx = char_to_byte_index(line, self.cursor_col);
        line.insert(byte_idx, c);
        self.cursor_col += 1;
    }

    /// Backspace: delete the character before the cursor.
    pub fn delete_char(&mut self) {
        if self.cursor_col > 0 {
            let line = &mut self.lines[self.cursor_row];
            let byte_idx = char_to_byte_index(line, self.cursor_col - 1);
            line.remove(byte_idx);
            self.cursor_col -= 1;
        } else if self.cursor_row > 0 {
            // Merge current line into previous line.
            let current = self.lines.remove(self.cursor_row);
            self.cursor_row -= 1;
            self.cursor_col = char_len(&self.lines[self.cursor_row]);
            self.lines[self.cursor_row].push_str(&current);
        }
    }

    /// Delete key: delete the character at the cursor.
    pub fn delete_forward(&mut self) {
        let line_char_len = char_len(&self.lines[self.cursor_row]);
        if self.cursor_col < line_char_len {
            let line = &mut self.lines[self.cursor_row];
            let byte_idx = char_to_byte_index(line, self.cursor_col);
            line.remove(byte_idx);
        } else if self.cursor_row + 1 < self.lines.len() {
            // Merge next line into current line.
            let next = self.lines.remove(self.cursor_row + 1);
            self.lines[self.cursor_row].push_str(&next);
        }
    }

    /// Move cursor left one character.
    pub fn move_left(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        } else if self.cursor_row > 0 {
            self.cursor_row -= 1;
            self.cursor_col = char_len(&self.lines[self.cursor_row]);
        }
    }

    /// Move cursor right one character.
    pub fn move_right(&mut self) {
        let line_char_len = char_len(&self.lines[self.cursor_row]);
        if self.cursor_col < line_char_len {
            self.cursor_col += 1;
        } else if self.cursor_row + 1 < self.lines.len() {
            self.cursor_row += 1;
            self.cursor_col = 0;
        }
    }

    /// Move cursor up one line.
    pub fn move_up(&mut self) {
        if self.cursor_row > 0 {
            self.cursor_row -= 1;
            let line_char_len = char_len(&self.lines[self.cursor_row]);
            self.cursor_col = self.cursor_col.min(line_char_len);
        }
    }

    /// Move cursor down one line.
    pub fn move_down(&mut self) {
        if self.cursor_row + 1 < self.lines.len() {
            self.cursor_row += 1;
            let line_char_len = char_len(&self.lines[self.cursor_row]);
            self.cursor_col = self.cursor_col.min(line_char_len);
        }
    }

    /// Move cursor to the start of the current line.
    pub fn home(&mut self) {
        self.cursor_col = 0;
    }

    /// Move cursor to the end of the current line.
    pub fn end(&mut self) {
        self.cursor_col = char_len(&self.lines[self.cursor_row]);
    }

    /// Clear all content and reset cursor.
    pub fn clear(&mut self) {
        self.lines = vec![String::new()];
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.scroll_offset = 0;
        self.history_pos = None;
    }

    /// Join all lines into a single string.
    pub fn content(&self) -> String {
        self.lines.join("\n")
    }

    /// Returns true if the input is empty.
    pub fn is_empty(&self) -> bool {
        self.lines.len() == 1 && self.lines[0].is_empty()
    }

    /// Take content, add to history, clear input. Returns the submitted text.
    pub fn submit(&mut self) -> String {
        let text = self.content();
        if !text.is_empty() {
            self.history.push(text.clone());
        }
        self.clear();
        text
    }

    /// Navigate to the previous history entry.
    pub fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let new_pos = match self.history_pos {
            Some(pos) => {
                if pos > 0 {
                    pos - 1
                } else {
                    return;
                }
            }
            None => self.history.len() - 1,
        };
        self.history_pos = Some(new_pos);
        self.load_from_history(new_pos);
    }

    /// Navigate to the next history entry.
    pub fn history_next(&mut self) {
        let Some(pos) = self.history_pos else {
            return;
        };
        if pos + 1 < self.history.len() {
            let new_pos = pos + 1;
            self.history_pos = Some(new_pos);
            self.load_from_history(new_pos);
        } else {
            // Past the end of history — clear to empty.
            self.history_pos = None;
            self.clear();
        }
    }

    /// Handle enter key. Currently always submits.
    pub fn handle_enter(&mut self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        Some(self.submit())
    }

    /// Insert a newline at the cursor position (for multi-line editing).
    pub fn insert_newline(&mut self) {
        let line = &mut self.lines[self.cursor_row];
        let byte_idx = char_to_byte_index(line, self.cursor_col);
        let remainder = line[byte_idx..].to_string();
        line.truncate(byte_idx);
        self.cursor_row += 1;
        self.lines.insert(self.cursor_row, remainder);
        self.cursor_col = 0;
    }

    /// Cursor position as (col, row) relative to visible area, accounting for prompt width.
    pub fn cursor_position(&self, prompt_len: u16) -> (u16, u16) {
        let col = prompt_len + self.cursor_col as u16;
        let row = self.cursor_row.saturating_sub(self.scroll_offset) as u16;
        (col, row)
    }

    /// Adjust scroll offset so the cursor row is visible within `visible_height` lines.
    pub fn ensure_cursor_visible(&mut self, visible_height: usize) {
        if visible_height == 0 {
            return;
        }
        if self.cursor_row < self.scroll_offset {
            self.scroll_offset = self.cursor_row;
        } else if self.cursor_row >= self.scroll_offset + visible_height {
            self.scroll_offset = self.cursor_row - visible_height + 1;
        }
    }

    fn load_from_history(&mut self, pos: usize) {
        let entry = &self.history[pos];
        self.lines = entry.lines().map(String::from).collect();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.cursor_row = self.lines.len() - 1;
        self.cursor_col = char_len(&self.lines[self.cursor_row]);
        self.scroll_offset = 0;
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Number of chars in a string.
fn char_len(s: &str) -> usize {
    s.chars().count()
}

/// Convert a char index to a byte index.
fn char_to_byte_index(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map_or(s.len(), |(byte_idx, _)| byte_idx)
}

// ---------------------------------------------------------------------------
// TuiTextInputWidget — StatefulWidget renderer
// ---------------------------------------------------------------------------

/// Renderer for `TuiTextInput`. Use with `StatefulWidget::render`.
pub(crate) struct TuiTextInputWidget<'a> {
    pub prompt: &'a str,
}

impl<'a> TuiTextInputWidget<'a> {
    pub fn new(prompt: &'a str) -> Self {
        Self { prompt }
    }
}

impl StatefulWidget for TuiTextInputWidget<'_> {
    type State = TuiTextInput;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let block = Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(" input ")
            .title_style(Style::default().fg(Color::DarkGray));
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let visible_height = inner.height as usize;
        state.ensure_cursor_visible(visible_height);

        let prompt_style = Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM);
        let text_style = Style::default().fg(Color::White);

        let visible_lines = state
            .lines
            .iter()
            .skip(state.scroll_offset)
            .take(visible_height);

        let ratatui_lines: Vec<Line<'_>> = visible_lines
            .enumerate()
            .map(|(i, line_text)| {
                let is_first = i == 0 && state.scroll_offset == 0;
                let prefix = if is_first { self.prompt } else { "" };
                Line::from(vec![
                    Span::styled(prefix, prompt_style),
                    Span::styled(line_text.as_str(), text_style),
                ])
            })
            .collect();

        let paragraph = Paragraph::new(ratatui_lines).wrap(Wrap { trim: false });
        paragraph.render(inner, buf);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_char_and_content() {
        let mut input = TuiTextInput::new();
        input.insert_char('h');
        input.insert_char('i');
        assert_eq!(input.content(), "hi");
        assert_eq!(input.cursor_col, 2);
    }

    #[test]
    fn delete_char_middle_of_line() {
        let mut input = TuiTextInput::new();
        for c in "abc".chars() {
            input.insert_char(c);
        }
        // cursor at 3, delete back → "ab"
        input.delete_char();
        assert_eq!(input.content(), "ab");
        assert_eq!(input.cursor_col, 2);
    }

    #[test]
    fn delete_char_at_start_does_nothing() {
        let mut input = TuiTextInput::new();
        input.insert_char('x');
        input.cursor_col = 0;
        input.delete_char();
        // Single line, col 0 — nothing to delete.
        assert_eq!(input.content(), "x");
    }

    #[test]
    fn delete_char_merges_lines() {
        let mut input = TuiTextInput::new();
        for c in "ab".chars() {
            input.insert_char(c);
        }
        input.insert_newline();
        input.insert_char('c');
        // lines: ["ab", "c"], cursor at row 1 col 1
        input.cursor_col = 0;
        input.delete_char();
        // Should merge: ["abc"], cursor at row 0 col 2
        assert_eq!(input.content(), "abc");
        assert_eq!(input.cursor_row, 0);
        assert_eq!(input.cursor_col, 2);
    }

    #[test]
    fn delete_forward_removes_char_at_cursor() {
        let mut input = TuiTextInput::new();
        for c in "abc".chars() {
            input.insert_char(c);
        }
        input.cursor_col = 1;
        input.delete_forward();
        assert_eq!(input.content(), "ac");
    }

    #[test]
    fn delete_forward_merges_next_line() {
        let mut input = TuiTextInput::new();
        for c in "ab".chars() {
            input.insert_char(c);
        }
        input.insert_newline();
        input.insert_char('c');
        // lines: ["ab", "c"], go back to end of first line
        input.cursor_row = 0;
        input.cursor_col = 2;
        input.delete_forward();
        assert_eq!(input.content(), "abc");
        assert_eq!(input.lines.len(), 1);
    }

    #[test]
    fn move_left_wraps_to_previous_line() {
        let mut input = TuiTextInput::new();
        input.insert_char('a');
        input.insert_newline();
        input.insert_char('b');
        // cursor at row 1, col 1
        input.cursor_col = 0;
        input.move_left();
        assert_eq!(input.cursor_row, 0);
        assert_eq!(input.cursor_col, 1);
    }

    #[test]
    fn move_left_at_origin_stays() {
        let mut input = TuiTextInput::new();
        input.insert_char('x');
        input.cursor_col = 0;
        input.move_left();
        assert_eq!(input.cursor_row, 0);
        assert_eq!(input.cursor_col, 0);
    }

    #[test]
    fn move_right_wraps_to_next_line() {
        let mut input = TuiTextInput::new();
        input.insert_char('a');
        input.insert_newline();
        input.insert_char('b');
        input.cursor_row = 0;
        input.cursor_col = 1;
        input.move_right();
        assert_eq!(input.cursor_row, 1);
        assert_eq!(input.cursor_col, 0);
    }

    #[test]
    fn move_right_at_end_stays() {
        let mut input = TuiTextInput::new();
        input.insert_char('z');
        // cursor at col 1, only line — should stay
        input.move_right();
        assert_eq!(input.cursor_row, 0);
        assert_eq!(input.cursor_col, 1);
    }

    #[test]
    fn move_up_clamps_col() {
        let mut input = TuiTextInput::new();
        for c in "abcdef".chars() {
            input.insert_char(c);
        }
        input.insert_newline();
        input.insert_char('x');
        // row 1, col 1 — move up to row 0 which has 6 chars, col stays 1
        input.move_up();
        assert_eq!(input.cursor_row, 0);
        assert_eq!(input.cursor_col, 1);
    }

    #[test]
    fn move_down_clamps_col() {
        let mut input = TuiTextInput::new();
        for c in "abcdef".chars() {
            input.insert_char(c);
        }
        input.insert_newline();
        input.insert_char('x');
        // go to row 0, col 6
        input.cursor_row = 0;
        input.cursor_col = 6;
        input.move_down();
        assert_eq!(input.cursor_row, 1);
        // row 1 has 1 char, col clamped to 1
        assert_eq!(input.cursor_col, 1);
    }

    #[test]
    fn home_and_end() {
        let mut input = TuiTextInput::new();
        for c in "hello".chars() {
            input.insert_char(c);
        }
        input.home();
        assert_eq!(input.cursor_col, 0);
        input.end();
        assert_eq!(input.cursor_col, 5);
    }

    #[test]
    fn submit_clears_and_adds_to_history() {
        let mut input = TuiTextInput::new();
        for c in "cmd1".chars() {
            input.insert_char(c);
        }
        let submitted = input.submit();
        assert_eq!(submitted, "cmd1");
        assert!(input.is_empty());
        assert_eq!(input.history.len(), 1);
        assert_eq!(input.history[0], "cmd1");
    }

    #[test]
    fn submit_empty_returns_none() {
        let mut input = TuiTextInput::new();
        assert!(input.handle_enter().is_none());
        assert!(input.history.is_empty());
    }

    #[test]
    fn history_prev_cycles_backward() {
        let mut input = TuiTextInput::new();
        // Build history
        for c in "first".chars() {
            input.insert_char(c);
        }
        input.submit();
        for c in "second".chars() {
            input.insert_char(c);
        }
        input.submit();

        // Navigate back
        input.history_prev();
        assert_eq!(input.content(), "second");
        assert_eq!(input.history_pos, Some(1));

        input.history_prev();
        assert_eq!(input.content(), "first");
        assert_eq!(input.history_pos, Some(0));

        // At the beginning — stays
        input.history_prev();
        assert_eq!(input.content(), "first");
        assert_eq!(input.history_pos, Some(0));
    }

    #[test]
    fn history_next_cycles_forward() {
        let mut input = TuiTextInput::new();
        for c in "aaa".chars() {
            input.insert_char(c);
        }
        input.submit();
        for c in "bbb".chars() {
            input.insert_char(c);
        }
        input.submit();

        // Go to oldest
        input.history_prev();
        input.history_prev();
        assert_eq!(input.content(), "aaa");

        // Forward
        input.history_next();
        assert_eq!(input.content(), "bbb");

        // Past end — clears
        input.history_next();
        assert!(input.is_empty());
        assert!(input.history_pos.is_none());
    }

    #[test]
    fn history_next_without_prev_does_nothing() {
        let mut input = TuiTextInput::new();
        for c in "x".chars() {
            input.insert_char(c);
        }
        input.submit();
        input.history_next();
        assert!(input.is_empty());
    }

    #[test]
    fn insert_newline_splits_line() {
        let mut input = TuiTextInput::new();
        for c in "abcd".chars() {
            input.insert_char(c);
        }
        input.cursor_col = 2;
        input.insert_newline();
        assert_eq!(input.lines, vec!["ab", "cd"]);
        assert_eq!(input.cursor_row, 1);
        assert_eq!(input.cursor_col, 0);
    }

    #[test]
    fn content_joins_with_newlines() {
        let mut input = TuiTextInput::new();
        for c in "line1".chars() {
            input.insert_char(c);
        }
        input.insert_newline();
        for c in "line2".chars() {
            input.insert_char(c);
        }
        assert_eq!(input.content(), "line1\nline2");
    }

    #[test]
    fn ensure_cursor_visible_scrolls_down() {
        let mut input = TuiTextInput::new();
        for _ in 0..10 {
            input.insert_newline();
        }
        // cursor at row 10, visible_height 3
        input.ensure_cursor_visible(3);
        assert_eq!(input.scroll_offset, 8);
    }

    #[test]
    fn ensure_cursor_visible_scrolls_up() {
        let mut input = TuiTextInput::new();
        for _ in 0..10 {
            input.insert_newline();
        }
        input.scroll_offset = 8;
        input.cursor_row = 2;
        input.ensure_cursor_visible(3);
        assert_eq!(input.scroll_offset, 2);
    }
}
