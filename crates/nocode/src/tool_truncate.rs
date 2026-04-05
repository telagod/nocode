/// Tool output truncation for display.
///
/// Provides configurable truncation of tool output strings, applying character
/// and line limits to keep terminal display manageable.

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum lines shown when displaying file-read output.
pub const READ_DISPLAY_MAX_LINES: usize = 80;
/// Maximum characters shown when displaying file-read output.
pub const READ_DISPLAY_MAX_CHARS: usize = 6_000;
/// Maximum lines shown for general tool output.
pub const TOOL_OUTPUT_DISPLAY_MAX_LINES: usize = 60;
/// Maximum characters shown for general tool output.
pub const TOOL_OUTPUT_DISPLAY_MAX_CHARS: usize = 4_000;

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// Truncation limits for a single output string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TruncateConfig {
    /// Maximum number of lines to keep.
    pub max_lines: usize,
    /// Maximum number of characters (bytes in the UTF-8 sense of `chars()`) to keep.
    pub max_chars: usize,
}

impl TruncateConfig {
    /// Preset for file-read tool output.
    #[must_use]
    pub fn for_read_file() -> Self {
        Self {
            max_lines: READ_DISPLAY_MAX_LINES,
            max_chars: READ_DISPLAY_MAX_CHARS,
        }
    }

    /// Preset for general tool output.
    #[must_use]
    pub fn for_tool_output() -> Self {
        Self {
            max_lines: TOOL_OUTPUT_DISPLAY_MAX_LINES,
            max_chars: TOOL_OUTPUT_DISPLAY_MAX_CHARS,
        }
    }
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

/// Result of a truncation operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TruncatedOutput {
    /// The (possibly truncated) content.
    pub content: String,
    /// Whether any truncation was applied.
    pub was_truncated: bool,
    /// Line count of the original input.
    pub original_lines: usize,
    /// Character count of the original input.
    pub original_chars: usize,
    /// Human-readable summary when truncation occurred.
    pub truncation_message: Option<String>,
}

// ---------------------------------------------------------------------------
// Core logic
// ---------------------------------------------------------------------------

/// Truncate `content` according to `config`.
///
/// The algorithm applies character truncation first (breaking at the nearest
/// preceding newline), then line truncation. If either limit triggers, a
/// `truncation_message` is produced.
#[must_use]
pub fn truncate_output(content: &str, config: &TruncateConfig) -> TruncatedOutput {
    let original_chars = content.len();
    let original_lines = content.lines().count();

    // --- Phase 1: character limit (snap to nearest preceding newline) -------
    let after_chars = if original_chars > config.max_chars {
        snap_to_newline(content, config.max_chars)
    } else {
        content
    };

    // --- Phase 2: line limit ------------------------------------------------
    let after_lines = limit_lines(after_chars, config.max_lines);

    let was_truncated = after_lines.len() < original_chars;

    let truncation_message = if was_truncated {
        let kept_lines = after_lines.lines().count();
        let dropped_lines = original_lines.saturating_sub(kept_lines);
        let dropped_chars = original_chars.saturating_sub(after_lines.len());
        Some(format!(
            "[...truncated {dropped_lines} lines, {dropped_chars} chars]"
        ))
    } else {
        None
    };

    TruncatedOutput {
        content: after_lines.to_owned(),
        was_truncated,
        original_lines,
        original_chars,
        truncation_message,
    }
}

/// Return the longest prefix of `s` that is at most `max` bytes, snapped back
/// to the last newline boundary. If there is no newline within the first `max`
/// bytes the hard limit is used (character-boundary safe).
fn snap_to_newline(s: &str, max: usize) -> &str {
    let safe = safe_char_boundary(s, max);
    let region = &s[..safe];
    if let Some(pos) = region.rfind('\n') {
        &s[..pos + 1]
    } else {
        region
    }
}

/// Return the prefix of `s` containing at most `max` lines.
fn limit_lines(s: &str, max: usize) -> &str {
    if max == 0 {
        return "";
    }
    let mut count = 0usize;
    for (idx, byte) in s.bytes().enumerate() {
        if byte == b'\n' {
            count += 1;
            if count == max {
                return &s[..idx + 1];
            }
        }
    }
    // Fewer than `max` newlines — return everything.
    s
}

/// Find the largest byte index <= `max` that sits on a UTF-8 character boundary.
fn safe_char_boundary(s: &str, max: usize) -> usize {
    if max >= s.len() {
        return s.len();
    }
    let mut idx = max;
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- no truncation ------------------------------------------------------

    #[test]
    fn short_input_is_not_truncated() {
        let input = "line one\nline two\nline three\n";
        let config = TruncateConfig {
            max_lines: 100,
            max_chars: 10_000,
        };
        let out = truncate_output(input, &config);

        assert_eq!(out.content, input);
        assert!(!out.was_truncated);
        assert_eq!(out.original_lines, 3);
        assert_eq!(out.original_chars, input.len());
        assert!(out.truncation_message.is_none());
    }

    // -- truncation by lines ------------------------------------------------

    #[test]
    fn truncates_by_line_limit() {
        let input = "a\nb\nc\nd\ne\n";
        let config = TruncateConfig {
            max_lines: 3,
            max_chars: 100_000,
        };
        let out = truncate_output(input, &config);

        assert_eq!(out.content, "a\nb\nc\n");
        assert!(out.was_truncated);
        assert_eq!(out.original_lines, 5);
        assert!(out.truncation_message.is_some());
    }

    // -- truncation by chars ------------------------------------------------

    #[test]
    fn truncates_by_char_limit_snaps_to_newline() {
        // 4 lines, each "abcdefghij\n" = 11 chars, total 44 chars.
        let input = "abcdefghij\nabcdefghij\nabcdefghij\nabcdefghij\n";
        let config = TruncateConfig {
            max_lines: 1000,
            max_chars: 25, // cuts inside line 3
        };
        let out = truncate_output(input, &config);

        // Should snap back to end of line 2.
        assert_eq!(out.content, "abcdefghij\nabcdefghij\n");
        assert!(out.was_truncated);
        assert_eq!(out.original_chars, 44);
        assert!(out.truncation_message.is_some());
    }

    // -- both limits trigger ------------------------------------------------

    #[test]
    fn both_limits_trigger_char_then_line() {
        // 10 lines of 20 chars each = 200 chars total.
        let line = "12345678901234567890";
        let input: String = (0..10).map(|_| format!("{line}\n")).collect();
        let config = TruncateConfig {
            max_lines: 3,
            max_chars: 80, // would keep ~3-4 lines by chars
        };
        let out = truncate_output(&input, &config);

        // char limit snaps to end of line 3 (63 chars), then line limit
        // keeps 3 lines — same result either way.
        let kept_lines = out.content.lines().count();
        assert!(kept_lines <= 3);
        assert!(out.was_truncated);
        assert!(out.truncation_message.is_some());
    }

    // -- empty input --------------------------------------------------------

    #[test]
    fn empty_input_returns_empty() {
        let config = TruncateConfig::for_tool_output();
        let out = truncate_output("", &config);

        assert_eq!(out.content, "");
        assert!(!out.was_truncated);
        assert_eq!(out.original_lines, 0);
        assert_eq!(out.original_chars, 0);
        assert!(out.truncation_message.is_none());
    }

    // -- truncation_message format ------------------------------------------

    #[test]
    fn truncation_message_format() {
        let input = "a\nb\nc\nd\ne\nf\n";
        let config = TruncateConfig {
            max_lines: 2,
            max_chars: 100_000,
        };
        let out = truncate_output(input, &config);
        let msg = out.truncation_message.as_deref().unwrap();

        assert!(msg.starts_with("[...truncated "));
        assert!(msg.contains(" lines, "));
        assert!(msg.ends_with(" chars]"));
    }

    // -- presets ------------------------------------------------------------

    #[test]
    fn preset_for_read_file() {
        let cfg = TruncateConfig::for_read_file();
        assert_eq!(cfg.max_lines, READ_DISPLAY_MAX_LINES);
        assert_eq!(cfg.max_chars, READ_DISPLAY_MAX_CHARS);
    }

    #[test]
    fn preset_for_tool_output() {
        let cfg = TruncateConfig::for_tool_output();
        assert_eq!(cfg.max_lines, TOOL_OUTPUT_DISPLAY_MAX_LINES);
        assert_eq!(cfg.max_chars, TOOL_OUTPUT_DISPLAY_MAX_CHARS);
    }

    // -- edge: no trailing newline ------------------------------------------

    #[test]
    fn input_without_trailing_newline() {
        let input = "a\nb\nc";
        let config = TruncateConfig {
            max_lines: 2,
            max_chars: 100_000,
        };
        let out = truncate_output(input, &config);

        assert_eq!(out.content, "a\nb\n");
        assert!(out.was_truncated);
    }
}
