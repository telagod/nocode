//! Tool-specific formatting for tool calls and results.
//!
//! Renders structured, human-readable representations of tool invocations
//! and their outputs, using [`crate::tool_truncate`] for length management.

use crate::tool_truncate::{self, TruncateConfig};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Safely extract a string field from a JSON value, returning `"(unknown)"` if
/// the field is missing or not a string.
pub fn extract_json_field(value: &Value, field: &str) -> String {
    value
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or("(unknown)")
        .to_owned()
}

/// Truncate `s` to at most `max` characters, appending `"..."` when truncated.
pub fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_owned()
    } else {
        let boundary = safe_char_boundary(s, max);
        format!("{}...", &s[..boundary])
    }
}

/// Find the largest byte index <= `max` on a UTF-8 character boundary.
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
// format_tool_call_start
// ---------------------------------------------------------------------------

/// Render the opening block for a tool invocation.
///
/// Returns a `Vec<String>` of display lines including a header, tool-specific
/// detail lines, and a closing border.
#[must_use]
pub fn format_tool_call_start(tool_name: &str, input: &Value) -> Vec<String> {
    let mut lines = Vec::new();

    match tool_name {
        "Bash" => {
            lines.push(String::from("┌─ 🔧 Bash ─────────"));
            let cmd = extract_json_field(input, "command");
            lines.push(format!("│ $ {cmd}"));
        }
        "Read" => {
            lines.push(String::from("┌─ 📄 Read ─────────"));
            let fp = extract_json_field(input, "file_path");
            lines.push(format!("│ {fp}"));
        }
        "Write" => {
            lines.push(String::from("┌─ ✏️ Write ────────"));
            let fp = extract_json_field(input, "file_path");
            let content_lines = input
                .get("content")
                .and_then(Value::as_str)
                .map(|c| c.lines().count())
                .unwrap_or(0);
            lines.push(format!("│ {fp} ({content_lines} lines)"));
        }
        "Edit" => {
            lines.push(String::from("┌─ 📝 Edit ─────────"));
            let fp = extract_json_field(input, "file_path");
            lines.push(format!("│ {fp}"));
            // old_string preview (first 3 lines)
            if let Some(old) = input.get("old_string").and_then(Value::as_str) {
                lines.push(String::from("│ old:"));
                for line in old.lines().take(3) {
                    lines.push(format!("│   {line}"));
                }
            }
            // new_string preview (first 3 lines)
            if let Some(new) = input.get("new_string").and_then(Value::as_str) {
                lines.push(String::from("│ new:"));
                for line in new.lines().take(3) {
                    lines.push(format!("│   {line}"));
                }
            }
        }
        "Glob" => {
            lines.push(String::from("┌─ 🔍 Glob ─────────"));
            let pattern = extract_json_field(input, "pattern");
            lines.push(format!("│ pattern: {pattern}"));
        }
        "Grep" => {
            lines.push(String::from("┌─ 🔍 Grep ─────────"));
            let pattern = extract_json_field(input, "pattern");
            let path = extract_json_field(input, "path");
            lines.push(format!("│ pattern: {pattern} path: {path}"));
        }
        other => {
            lines.push(format!("┌─ 🔧 {other} ──"));
            let summary = truncate_str(&input.to_string(), 200);
            lines.push(format!("│ {summary}"));
        }
    }

    lines.push(String::from("└─────────────────"));
    lines
}

// ---------------------------------------------------------------------------
// format_tool_result
// ---------------------------------------------------------------------------

/// Render the result block for a completed tool invocation.
///
/// Returns a `Vec<String>` of display lines, each prefixed with `"  ▸ "`.
#[must_use]
pub fn format_tool_result(tool_name: &str, output: &str) -> Vec<String> {
    let raw_lines: Vec<String> = match tool_name {
        "Bash" => format_bash_result(output),
        "Read" => format_read_result(output),
        "Write" => vec![format!("✔ wrote file")],
        "Edit" => vec![format!("✔ edited file")],
        _ => format_generic_result(output),
    };

    raw_lines
        .into_iter()
        .map(|line| format!("  ▸ {line}"))
        .collect()
}

// ---------------------------------------------------------------------------
// Private result formatters
// ---------------------------------------------------------------------------

fn format_bash_result(output: &str) -> Vec<String> {
    let mut lines = Vec::new();

    // Try to parse as JSON with stdout/stderr/exit_code fields.
    if let Ok(parsed) = serde_json::from_str::<Value>(output) {
        if let Some(code) = parsed.get("exit_code").and_then(Value::as_i64) {
            lines.push(format!("exit: {code}"));
        }
        if let Some(stdout) = parsed.get("stdout").and_then(Value::as_str)
            && !stdout.is_empty()
        {
                let truncated =
                    tool_truncate::truncate_output(stdout, &TruncateConfig::for_tool_output());
                lines.push(String::from("stdout:"));
                for line in truncated.content.lines() {
                    lines.push(format!("  {line}"));
                }
                if let Some(msg) = truncated.truncation_message {
                    lines.push(msg);
                }
            }
        if let Some(stderr) = parsed.get("stderr").and_then(Value::as_str)
            && !stderr.is_empty()
        {
                let truncated =
                    tool_truncate::truncate_output(stderr, &TruncateConfig::for_tool_output());
                lines.push(String::from("stderr:"));
                for line in truncated.content.lines() {
                    lines.push(format!("  {line}"));
                }
                if let Some(msg) = truncated.truncation_message {
                    lines.push(msg);
                }
            }
        if !lines.is_empty() {
            return lines;
        }
    }

    // Fallback: treat as plain text.
    let truncated = tool_truncate::truncate_output(output, &TruncateConfig::for_tool_output());
    for line in truncated.content.lines() {
        lines.push(line.to_owned());
    }
    if let Some(msg) = truncated.truncation_message {
        lines.push(msg);
    }
    lines
}

fn format_read_result(output: &str) -> Vec<String> {
    let truncated = tool_truncate::truncate_output(output, &TruncateConfig::for_read_file());
    let mut lines: Vec<String> = truncated.content.lines().map(String::from).collect();
    if let Some(msg) = truncated.truncation_message {
        lines.push(msg);
    }
    lines
}

fn format_generic_result(output: &str) -> Vec<String> {
    let truncated = tool_truncate::truncate_output(output, &TruncateConfig::for_tool_output());
    let mut lines: Vec<String> = truncated.content.lines().map(String::from).collect();
    if let Some(msg) = truncated.truncation_message {
        lines.push(msg);
    }
    lines
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -- format_tool_call_start: Bash -----------------------------------------

    #[test]
    fn bash_call_start_shows_command() {
        let input = json!({"command": "ls -la"});
        let lines = format_tool_call_start("Bash", &input);

        assert_eq!(lines[0], "┌─ 🔧 Bash ─────────");
        assert_eq!(lines[1], "│ $ ls -la");
        assert_eq!(lines.last().unwrap(), "└─────────────────");
    }

    // -- format_tool_call_start: Read -----------------------------------------

    #[test]
    fn read_call_start_shows_file_path() {
        let input = json!({"file_path": "/tmp/foo.rs"});
        let lines = format_tool_call_start("Read", &input);

        assert_eq!(lines[0], "┌─ 📄 Read ─────────");
        assert_eq!(lines[1], "│ /tmp/foo.rs");
        assert_eq!(lines.last().unwrap(), "└─────────────────");
    }

    // -- format_tool_call_start: Write ----------------------------------------

    #[test]
    fn write_call_start_shows_line_count() {
        let input = json!({"file_path": "/tmp/out.rs", "content": "a\nb\nc\n"});
        let lines = format_tool_call_start("Write", &input);

        assert_eq!(lines[0], "┌─ ✏️ Write ────────");
        assert_eq!(lines[1], "│ /tmp/out.rs (3 lines)");
    }

    // -- format_tool_call_start: Edit -----------------------------------------

    #[test]
    fn edit_call_start_shows_old_new_preview() {
        let input = json!({
            "file_path": "/tmp/edit.rs",
            "old_string": "line1\nline2\nline3\nline4",
            "new_string": "new1\nnew2\nnew3\nnew4"
        });
        let lines = format_tool_call_start("Edit", &input);

        assert_eq!(lines[0], "┌─ 📝 Edit ─────────");
        assert_eq!(lines[1], "│ /tmp/edit.rs");
        assert!(lines.contains(&String::from("│ old:")));
        assert!(lines.contains(&String::from("│   line1")));
        assert!(lines.contains(&String::from("│   line3")));
        // line4 should NOT appear (only first 3 lines)
        assert!(!lines.contains(&String::from("│   line4")));
        assert!(lines.contains(&String::from("│ new:")));
        assert!(lines.contains(&String::from("│   new1")));
        assert!(!lines.contains(&String::from("│   new4")));
    }

    // -- format_tool_call_start: Glob -----------------------------------------

    #[test]
    fn glob_call_start_shows_pattern() {
        let input = json!({"pattern": "**/*.rs"});
        let lines = format_tool_call_start("Glob", &input);

        assert_eq!(lines[0], "┌─ 🔍 Glob ─────────");
        assert_eq!(lines[1], "│ pattern: **/*.rs");
    }

    // -- format_tool_call_start: Grep -----------------------------------------

    #[test]
    fn grep_call_start_shows_pattern_and_path() {
        let input = json!({"pattern": "TODO", "path": "/src"});
        let lines = format_tool_call_start("Grep", &input);

        assert_eq!(lines[0], "┌─ 🔍 Grep ─────────");
        assert_eq!(lines[1], "│ pattern: TODO path: /src");
    }

    // -- format_tool_call_start: generic --------------------------------------

    #[test]
    fn generic_tool_call_truncates_input() {
        let long_val = "x".repeat(300);
        let input = json!({"data": long_val});
        let lines = format_tool_call_start("CustomTool", &input);

        assert_eq!(lines[0], "┌─ 🔧 CustomTool ──");
        // The JSON summary on line[1] should be truncated to ~200 + "..."
        assert!(lines[1].len() < 220);
        assert!(lines[1].contains("..."));
    }

    // -- format_tool_call_start: empty input ----------------------------------

    #[test]
    fn empty_input_shows_unknown_fields() {
        let input = json!({});
        let lines = format_tool_call_start("Bash", &input);

        assert_eq!(lines[1], "│ $ (unknown)");
    }

    // -- format_tool_result: Bash JSON --------------------------------------

    #[test]
    fn bash_result_parses_json_output() {
        let output = r#"{"exit_code":0,"stdout":"hello world\n","stderr":""}"#;
        let lines = format_tool_result("Bash", output);

        assert!(lines[0].contains("exit: 0"));
        assert!(lines.iter().any(|l| l.contains("hello world")));
        // All lines should have the prefix
        for line in &lines {
            assert!(line.starts_with("  ▸ "));
        }
    }

    // -- format_tool_result: Bash plain text ----------------------------------

    #[test]
    fn bash_result_falls_back_to_plain_text() {
        let output = "some plain output";
        let lines = format_tool_result("Bash", output);

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], "  ▸ some plain output");
    }

    // -- format_tool_result: Read truncation ----------------------------------

    #[test]
    fn read_result_truncates_long_content() {
        // Generate content exceeding READ_DISPLAY_MAX_LINES (80)
        let long_content: String = (0..200).map(|i| format!("line {i}\n")).collect();
        let lines = format_tool_result("Read", &long_content);

        // Should be truncated — fewer lines than original
        assert!(lines.len() < 200);
        // Last line should be the truncation message
        let last = lines.last().unwrap();
        assert!(last.contains("truncated"));
    }

    // -- format_tool_result: Write / Edit -------------------------------------

    #[test]
    fn write_result_shows_confirmation() {
        let lines = format_tool_result("Write", "ok");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("✔ wrote file"));
    }

    #[test]
    fn edit_result_shows_confirmation() {
        let lines = format_tool_result("Edit", "ok");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("✔ edited file"));
    }

    // -- format_tool_result: generic truncation -------------------------------

    #[test]
    fn generic_result_truncates_output() {
        let long_output: String = (0..200).map(|i| format!("row {i}\n")).collect();
        let lines = format_tool_result("SomeTool", &long_output);

        assert!(lines.len() < 200);
        assert!(lines.last().unwrap().contains("truncated"));
    }

    // -- helpers --------------------------------------------------------------

    #[test]
    fn extract_json_field_returns_value() {
        let v = json!({"name": "test"});
        assert_eq!(extract_json_field(&v, "name"), "test");
    }

    #[test]
    fn extract_json_field_returns_unknown_for_missing() {
        let v = json!({});
        assert_eq!(extract_json_field(&v, "name"), "(unknown)");
    }

    #[test]
    fn extract_json_field_returns_unknown_for_non_string() {
        let v = json!({"count": 42});
        assert_eq!(extract_json_field(&v, "count"), "(unknown)");
    }

    #[test]
    fn truncate_str_short_unchanged() {
        assert_eq!(truncate_str("hello", 10), "hello");
    }

    #[test]
    fn truncate_str_long_gets_ellipsis() {
        let result = truncate_str("hello world", 5);
        assert_eq!(result, "hello...");
    }

    #[test]
    fn truncate_str_empty() {
        assert_eq!(truncate_str("", 10), "");
    }
}
