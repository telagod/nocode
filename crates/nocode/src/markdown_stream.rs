/// Streaming markdown renderer that buffers text deltas and emits safe-to-render
/// fragments at paragraph or closed-fence boundaries.
///
/// Design constraints:
/// - Inside a fenced code block we never split on blank lines — the entire block
///   is held until the closing fence arrives.
/// - Outside a code block, a double newline (`\n\n`) marks a paragraph boundary
///   and triggers a flush of everything up to (and including) that boundary.

#[derive(Debug, Default)]
pub struct MarkdownStreamState {
    buffer: String,
    in_fence: bool,
}

impl MarkdownStreamState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Accumulate a text delta. Returns a safe-to-render fragment when a
    /// complete boundary is detected, or `None` if more data is needed.
    pub fn push(&mut self, delta: &str) -> Option<String> {
        self.buffer.push_str(delta);
        self.try_emit()
    }

    /// Force-flush whatever remains in the buffer.
    pub fn flush(&mut self) -> Option<String> {
        if self.buffer.is_empty() {
            return None;
        }
        self.in_fence = false;
        Some(std::mem::take(&mut self.buffer))
    }

    // --- internals ---

    /// Scan the buffer for the next renderable boundary and return it.
    fn try_emit(&mut self) -> Option<String> {
        let bytes = self.buffer.as_bytes();
        let len = bytes.len();
        let mut i = 0;

        // If we're already inside a fence from a previous push, the opening
        // fence marker is still at the start of the buffer. Skip past it so
        // we don't re-toggle.
        if self.in_fence {
            if self.is_fence_at(0) {
                if let Some(end) = self.scan_fence_marker(0) {
                    i = end;
                }
            }
        }

        while i < len {
            if self.is_fence_at(i)
                && let Some(end) = self.scan_fence_marker(i)
            {
                if self.in_fence {
                    // Closing fence found — drain everything up to `end`.
                    self.in_fence = false;
                    return Some(self.drain_up_to(end));
                }
                // Opening fence — mark and skip past it.
                self.in_fence = true;
                i = end;
                continue;
            }

            // Paragraph break (only outside fences).
            if !self.in_fence && bytes[i] == b'\n' && i + 1 < len && bytes[i + 1] == b'\n' {
                let cut = i + 2;
                return Some(self.drain_up_to(cut));
            }

            i += 1;
        }

        None
    }

    /// Returns `true` when position `i` is at the start of a line (or the
    /// buffer) — the only place a fence marker is valid.
    fn is_fence_at(&self, i: usize) -> bool {
        let bytes = self.buffer.as_bytes();
        if i >= bytes.len() || bytes[i] != b'`' {
            return false;
        }
        i == 0 || bytes[i - 1] == b'\n'
    }

    /// Starting at `i` (which must point to a `` ` ``), count consecutive
    /// backticks. If there are at least 3, skip to the end of the line and
    /// return the index *after* the newline (or end-of-buffer).
    fn scan_fence_marker(&self, start: usize) -> Option<usize> {
        let bytes = self.buffer.as_bytes();
        let len = bytes.len();
        let mut i = start;
        while i < len && bytes[i] == b'`' {
            i += 1;
        }
        let backtick_count = i - start;
        if backtick_count < 3 {
            return None;
        }
        // Skip to end of line (info-string for opening fences, empty for closing).
        while i < len && bytes[i] != b'\n' {
            i += 1;
        }
        // Include the newline if present.
        if i < len && bytes[i] == b'\n' {
            i += 1;
        }
        Some(i)
    }

    /// Drain the first `end` bytes from the buffer and return them.
    fn drain_up_to(&mut self, end: usize) -> String {
        let fragment = self.buffer[..end].to_string();
        self.buffer.drain(..end);
        fragment
    }
}

#[cfg(test)]
mod tests {
    use super::MarkdownStreamState;

    // ---------------------------------------------------------------
    // Paragraph splitting
    // ---------------------------------------------------------------

    #[test]
    fn single_paragraph_no_emit() {
        let mut state = MarkdownStreamState::new();
        assert!(state.push("hello world").is_none());
    }

    #[test]
    fn two_paragraphs_emits_first() {
        let mut state = MarkdownStreamState::new();
        let out = state.push("first paragraph\n\nsecond paragraph");
        assert_eq!(out.as_deref(), Some("first paragraph\n\n"));
        // The remainder stays buffered.
        assert_eq!(state.flush().as_deref(), Some("second paragraph"));
    }

    #[test]
    fn paragraph_boundary_across_deltas() {
        let mut state = MarkdownStreamState::new();
        assert!(state.push("aaa\n").is_none());
        let out = state.push("\nbbb");
        assert_eq!(out.as_deref(), Some("aaa\n\n"));
        assert_eq!(state.flush().as_deref(), Some("bbb"));
    }

    #[test]
    fn multiple_paragraphs_emit_one_at_a_time() {
        let mut state = MarkdownStreamState::new();
        let first = state.push("p1\n\np2\n\np3");
        assert_eq!(first.as_deref(), Some("p1\n\n"));
        let second = state.push("");
        assert_eq!(second.as_deref(), Some("p2\n\n"));
        assert_eq!(state.flush().as_deref(), Some("p3"));
    }

    // ---------------------------------------------------------------
    // Code fence handling
    // ---------------------------------------------------------------

    #[test]
    fn code_block_suppresses_paragraph_split() {
        let mut state = MarkdownStreamState::new();
        // The double newline inside the fence must NOT trigger a split.
        let out = state.push("```\nline1\n\nline2\n```\n");
        assert_eq!(out.as_deref(), Some("```\nline1\n\nline2\n```\n"));
    }

    #[test]
    fn code_block_across_multiple_deltas() {
        let mut state = MarkdownStreamState::new();
        assert!(state.push("```rust\n").is_none());
        assert!(state.push("fn main() {\n\n").is_none());
        assert!(state.push("}\n").is_none());
        let out = state.push("```\n");
        assert_eq!(out.as_deref(), Some("```rust\nfn main() {\n\n}\n```\n"));
    }

    #[test]
    fn text_after_closed_fence_splits_normally() {
        let mut state = MarkdownStreamState::new();
        let out = state.push("```\ncode\n```\n\n\nnext paragraph");
        // First emit: the closed code block.
        assert_eq!(out.as_deref(), Some("```\ncode\n```\n"));
        // The remaining "\n\nnext paragraph" should split on the paragraph break.
        let second = state.push("");
        assert_eq!(second.as_deref(), Some("\n\n"));
        assert_eq!(state.flush().as_deref(), Some("next paragraph"));
    }

    // ---------------------------------------------------------------
    // Flush behaviour
    // ---------------------------------------------------------------

    #[test]
    fn flush_empty_returns_none() {
        let mut state = MarkdownStreamState::new();
        assert!(state.flush().is_none());
    }

    #[test]
    fn flush_returns_remaining_buffer() {
        let mut state = MarkdownStreamState::new();
        state.push("partial");
        assert_eq!(state.flush().as_deref(), Some("partial"));
        assert!(state.flush().is_none());
    }

    #[test]
    fn flush_resets_fence_state() {
        let mut state = MarkdownStreamState::new();
        state.push("```\nunclosed code");
        let out = state.flush();
        assert_eq!(out.as_deref(), Some("```\nunclosed code"));
        // After flush, fence state is reset — a new paragraph split should work.
        let out = state.push("a\n\nb");
        assert_eq!(out.as_deref(), Some("a\n\n"));
    }

    // ---------------------------------------------------------------
    // Edge cases
    // ---------------------------------------------------------------

    #[test]
    fn backticks_fewer_than_three_are_not_fences() {
        let mut state = MarkdownStreamState::new();
        let out = state.push("``not a fence``\n\nnext");
        assert_eq!(out.as_deref(), Some("``not a fence``\n\n"));
    }

    #[test]
    fn fence_with_info_string() {
        let mut state = MarkdownStreamState::new();
        assert!(state.push("```python\nprint('hi')\n").is_none());
        let out = state.push("```\n");
        assert_eq!(out.as_deref(), Some("```python\nprint('hi')\n```\n"));
    }

    #[test]
    fn empty_delta_is_harmless() {
        let mut state = MarkdownStreamState::new();
        assert!(state.push("").is_none());
        assert!(state.flush().is_none());
    }
}
