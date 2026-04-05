//! Summary compression — trims compaction summaries to a character/line budget.
//!
//! Ported from `claw-code/rust/crates/runtime/src/summary_compression.rs`.

use std::collections::BTreeSet;

// ---------------------------------------------------------------------------
// Budget & Result types
// ---------------------------------------------------------------------------

/// Budget knobs that control how aggressively a summary is compressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SummaryCompressionBudget {
    /// Maximum total characters in the compressed output (default 1200).
    pub max_chars: usize,
    /// Maximum number of lines in the compressed output (default 24).
    pub max_lines: usize,
    /// Maximum characters per individual line (default 160).
    pub max_line_chars: usize,
}

impl Default for SummaryCompressionBudget {
    fn default() -> Self {
        Self {
            max_chars: 1_200,
            max_lines: 24,
            max_line_chars: 160,
        }
    }
}

/// Statistics produced by [`compress_summary`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryCompressionResult {
    pub summary: String,
    pub original_chars: usize,
    pub compressed_chars: usize,
    pub original_lines: usize,
    pub compressed_lines: usize,
    pub removed_duplicate_lines: usize,
    pub omitted_lines: usize,
    pub truncated: bool,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Compress a summary string to fit within the given budget.
///
/// Steps: normalise whitespace, deduplicate lines, truncate long lines,
/// priority-select lines that fit the budget, emit an omission notice if needed.
#[must_use]
pub fn compress_summary(
    summary: &str,
    budget: &SummaryCompressionBudget,
) -> SummaryCompressionResult {
    let original_chars = summary.chars().count();
    let original_lines = summary.lines().count();

    let normalized = normalize_lines(summary, budget.max_line_chars);

    if normalized.lines.is_empty() || budget.max_chars == 0 || budget.max_lines == 0 {
        return SummaryCompressionResult {
            summary: String::new(),
            original_chars,
            compressed_chars: 0,
            original_lines,
            compressed_lines: 0,
            removed_duplicate_lines: normalized.removed_duplicate_lines,
            omitted_lines: normalized.lines.len(),
            truncated: original_chars > 0,
        };
    }

    let selected = select_line_indexes(&normalized.lines, budget);
    let mut compressed_lines: Vec<String> = selected
        .iter()
        .map(|i| normalized.lines[*i].clone())
        .collect();

    if compressed_lines.is_empty() {
        compressed_lines.push(truncate_line(&normalized.lines[0], budget.max_chars));
    }

    let omitted_lines = normalized.lines.len().saturating_sub(compressed_lines.len());

    if omitted_lines > 0 {
        let notice = format!("- ... {omitted_lines} additional line(s) omitted.");
        push_line_with_budget(&mut compressed_lines, notice, budget);
    }

    let compressed_summary = compressed_lines.join("\n");

    SummaryCompressionResult {
        compressed_chars: compressed_summary.chars().count(),
        compressed_lines: compressed_lines.len(),
        removed_duplicate_lines: normalized.removed_duplicate_lines,
        omitted_lines,
        truncated: compressed_summary != summary.trim(),
        summary: compressed_summary,
        original_chars,
        original_lines,
    }
}

/// Convenience wrapper — returns only the compressed text using default budget.
#[must_use]
pub fn compress_summary_text(summary: &str) -> String {
    compress_summary(summary, &SummaryCompressionBudget::default()).summary
}

/// Returns `true` when `line` starts with one of the core detail prefixes.
#[must_use]
pub fn is_core_detail_line(line: &str) -> bool {
    is_core_detail(line)
}

/// Deduplicate lines while preserving order (first occurrence wins).
#[must_use]
pub fn deduplicate_lines<'a>(lines: &[&'a str]) -> Vec<&'a str> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for &line in lines {
        let key = line.to_ascii_lowercase();
        if seen.insert(key) {
            out.push(line);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct NormalizedSummary {
    lines: Vec<String>,
    removed_duplicate_lines: usize,
}

fn normalize_lines(summary: &str, max_line_chars: usize) -> NormalizedSummary {
    let mut seen = BTreeSet::new();
    let mut lines = Vec::new();
    let mut removed_duplicate_lines = 0;

    for raw_line in summary.lines() {
        let normalized = collapse_inline_whitespace(raw_line);
        if normalized.is_empty() {
            continue;
        }

        let truncated = truncate_line(&normalized, max_line_chars);
        let key = truncated.to_ascii_lowercase();
        if !seen.insert(key) {
            removed_duplicate_lines += 1;
            continue;
        }

        lines.push(truncated);
    }

    NormalizedSummary {
        lines,
        removed_duplicate_lines,
    }
}

fn select_line_indexes(lines: &[String], budget: &SummaryCompressionBudget) -> Vec<usize> {
    let mut selected = BTreeSet::<usize>::new();

    for priority in 0..=3 {
        for (index, line) in lines.iter().enumerate() {
            if selected.contains(&index) || line_priority(line) != priority {
                continue;
            }

            let candidate: Vec<&str> = selected
                .iter()
                .map(|i| lines[*i].as_str())
                .chain(std::iter::once(line.as_str()))
                .collect();

            if candidate.len() > budget.max_lines {
                continue;
            }
            if joined_char_count(&candidate) > budget.max_chars {
                continue;
            }

            selected.insert(index);
        }
    }

    selected.into_iter().collect()
}

fn push_line_with_budget(lines: &mut Vec<String>, line: String, budget: &SummaryCompressionBudget) {
    let candidate: Vec<&str> = lines
        .iter()
        .map(String::as_str)
        .chain(std::iter::once(line.as_str()))
        .collect();

    if candidate.len() <= budget.max_lines && joined_char_count(&candidate) <= budget.max_chars {
        lines.push(line);
    }
}

fn joined_char_count(lines: &[&str]) -> usize {
    lines.iter().map(|l| l.chars().count()).sum::<usize>() + lines.len().saturating_sub(1)
}

fn line_priority(line: &str) -> usize {
    if line == "Summary:" || line == "Conversation summary:" || is_core_detail(line) {
        0
    } else if is_section_header(line) {
        1
    } else if line.starts_with("- ") || line.starts_with("  - ") {
        2
    } else {
        3
    }
}

fn is_core_detail(line: &str) -> bool {
    [
        "- Scope:",
        "- Current work:",
        "- Pending work:",
        "- Key files referenced:",
        "- Key files:",
        "- Tools mentioned:",
        "- Tools used:",
        "- Recent user requests:",
        "- Recent requests:",
        "- Previously compacted context:",
        "- Newly compacted context:",
    ]
    .iter()
    .any(|prefix| line.starts_with(prefix))
}

fn is_section_header(line: &str) -> bool {
    line.ends_with(':')
}

fn collapse_inline_whitespace(line: &str) -> String {
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_line(line: &str, max_chars: usize) -> String {
    if max_chars == 0 || line.chars().count() <= max_chars {
        return line.to_string();
    }
    if max_chars == 1 {
        return "\u{2026}".to_string();
    }
    let mut truncated: String = line.chars().take(max_chars.saturating_sub(1)).collect();
    truncated.push('\u{2026}');
    truncated
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compress_removes_duplicates() {
        let input = "- Scope: foo\n- Scope: foo\n- Current work: bar\n";
        let r = compress_summary(input, &SummaryCompressionBudget::default());
        assert_eq!(r.removed_duplicate_lines, 1);
        assert!(r.summary.contains("- Scope: foo"));
        assert!(r.summary.contains("- Current work: bar"));
    }

    #[test]
    fn compress_normalizes_whitespace() {
        let input = "- Scope:   lots   of   spaces\n";
        let r = compress_summary(input, &SummaryCompressionBudget::default());
        assert!(r.summary.contains("- Scope: lots of spaces"));
    }

    #[test]
    fn compress_truncates_long_lines() {
        let long_line = "x".repeat(200);
        let input = format!("- Scope: {long_line}\n");
        let budget = SummaryCompressionBudget {
            max_line_chars: 50,
            ..Default::default()
        };
        let r = compress_summary(&input, &budget);
        for line in r.summary.lines() {
            assert!(line.chars().count() <= 50, "line too long: {}", line.chars().count());
        }
    }

    #[test]
    fn compress_respects_max_lines() {
        let input = (0..40)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let budget = SummaryCompressionBudget {
            max_lines: 5,
            ..Default::default()
        };
        let r = compress_summary(&input, &budget);
        assert!(r.compressed_lines <= 5);
    }

    #[test]
    fn compress_respects_max_chars() {
        let input = (0..40)
            .map(|i| format!("item number {i} with some padding text"))
            .collect::<Vec<_>>()
            .join("\n");
        let budget = SummaryCompressionBudget {
            max_chars: 100,
            ..Default::default()
        };
        let r = compress_summary(&input, &budget);
        assert!(r.compressed_chars <= 100);
    }

    #[test]
    fn core_detail_lines_prioritized() {
        let lines = [
            "Conversation summary:",
            "- Scope: compacted 20 messages.",
            "- Current work: implementing compression.",
            "filler line alpha",
            "filler line beta",
            "filler line gamma",
            "filler line delta",
        ];
        let input = lines.join("\n");
        let budget = SummaryCompressionBudget {
            max_lines: 4,
            max_chars: 300,
            max_line_chars: 160,
        };
        let r = compress_summary(&input, &budget);
        assert!(r.summary.contains("Conversation summary:"));
        assert!(r.summary.contains("- Scope:"));
        assert!(r.summary.contains("- Current work:"));
    }

    #[test]
    fn empty_input_returns_empty() {
        let r = compress_summary("", &SummaryCompressionBudget::default());
        assert!(r.summary.is_empty());
        assert_eq!(r.original_chars, 0);
        assert_eq!(r.compressed_chars, 0);
        assert_eq!(r.compressed_lines, 0);
    }

    #[test]
    fn already_small_input_unchanged() {
        let input = "Summary:\n- Scope: small.";
        let r = compress_summary(input, &SummaryCompressionBudget::default());
        assert_eq!(r.summary, "Summary:\n- Scope: small.");
        assert!(!r.truncated);
        assert_eq!(r.omitted_lines, 0);
    }

    #[test]
    fn deduplicate_lines_preserves_order() {
        let lines = vec!["alpha", "beta", "alpha", "gamma", "beta"];
        let deduped = deduplicate_lines(&lines);
        assert_eq!(deduped, vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn is_core_detail_line_matches() {
        assert!(is_core_detail_line("- Scope: something"));
        assert!(is_core_detail_line("- Current work: stuff"));
        assert!(is_core_detail_line("- Pending work: more"));
        assert!(is_core_detail_line("- Key files: a.rs"));
        assert!(is_core_detail_line("- Recent requests: foo"));
        assert!(!is_core_detail_line("random line"));
        assert!(!is_core_detail_line("- Not a core line"));
    }

    #[test]
    fn compress_summary_text_convenience() {
        let input = "Summary:\nA short line.";
        let compressed = compress_summary_text(input);
        assert_eq!(compressed, "Summary:\nA short line.");
    }
}
