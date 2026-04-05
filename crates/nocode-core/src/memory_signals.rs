//! Automatic memory signal detection.
//!
//! Scans user text for patterns that indicate something worth persisting
//! (corrections, preferences, role info, project context, etc.) and returns
//! structured [`MemorySignal`] values the caller can act on.

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// The kind of memory a signal suggests creating.
///
/// Kept local so this module compiles independently of `memory_store`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestedMemoryType {
    User,
    Feedback,
    Project,
    Reference,
}

/// Classification of the detected signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemorySignalType {
    /// "don't do X", "stop doing Y", "no not that"
    UserCorrection,
    /// "I prefer", "always use", "never use"
    UserPreference,
    /// "I'm a", "I work on", "my role is"
    UserRole,
    /// "we're working on", "the deadline is", "the goal is"
    ProjectContext,
    /// "yes exactly", "perfect", "keep doing that"
    PositiveFeedback,
    /// "check the", "it's tracked in", "the dashboard at"
    ExternalReference,
    /// "remember that", "save this", "note that"
    ExplicitRemember,
    /// "forget that", "remove the memory", "don't remember"
    ExplicitForget,
}

/// A single detected memory signal with metadata.
#[derive(Debug, Clone)]
pub struct MemorySignal {
    pub signal_type: MemorySignalType,
    pub source_text: String,
    pub suggested_memory_type: SuggestedMemoryType,
    pub confidence: f32,
    pub suggested_name: String,
    pub suggested_content: String,
}

// ---------------------------------------------------------------------------
// Pattern table
// ---------------------------------------------------------------------------

struct PatternEntry {
    phrases: &'static [&'static str],
    signal_type: MemorySignalType,
    memory_type: SuggestedMemoryType,
    confidence: f32,
}

const PATTERNS: &[PatternEntry] = &[
    PatternEntry {
        phrases: &["don't ", "do not ", "stop ", "no not ", "never "],
        signal_type: MemorySignalType::UserCorrection,
        memory_type: SuggestedMemoryType::Feedback,
        confidence: 0.8,
    },
    PatternEntry {
        phrases: &["i prefer", "always use", "never use", "i like", "i want"],
        signal_type: MemorySignalType::UserPreference,
        memory_type: SuggestedMemoryType::Feedback,
        confidence: 0.7,
    },
    PatternEntry {
        phrases: &["i'm a", "i am a", "my role", "i work on", "i work as"],
        signal_type: MemorySignalType::UserRole,
        memory_type: SuggestedMemoryType::User,
        confidence: 0.8,
    },
    PatternEntry {
        phrases: &[
            "we're working on",
            "the project",
            "the deadline",
            "the goal is",
            "we need to",
        ],
        signal_type: MemorySignalType::ProjectContext,
        memory_type: SuggestedMemoryType::Project,
        confidence: 0.6,
    },
    PatternEntry {
        phrases: &[
            "yes exactly",
            "perfect",
            "keep doing that",
            "that's right",
            "good approach",
        ],
        signal_type: MemorySignalType::PositiveFeedback,
        memory_type: SuggestedMemoryType::Feedback,
        confidence: 0.6,
    },
    PatternEntry {
        phrases: &[
            "check the",
            "it's tracked in",
            "the dashboard at",
            "bugs are in",
        ],
        signal_type: MemorySignalType::ExternalReference,
        memory_type: SuggestedMemoryType::Reference,
        confidence: 0.7,
    },
    PatternEntry {
        phrases: &["remember that", "save this", "note that", "keep in mind"],
        signal_type: MemorySignalType::ExplicitRemember,
        memory_type: SuggestedMemoryType::User, // caller should refine
        confidence: 0.9,
    },
    PatternEntry {
        phrases: &[
            "forget that",
            "forget about",
            "remove the memory",
            "don't remember",
        ],
        signal_type: MemorySignalType::ExplicitForget,
        memory_type: SuggestedMemoryType::User, // placeholder
        confidence: 0.9,
    },
];

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Scan `text` for memory-worthy signals and return all matches.
pub fn detect_memory_signals(text: &str) -> Vec<MemorySignal> {
    let lower = text.to_lowercase();
    let mut signals = Vec::new();

    for entry in PATTERNS {
        for &phrase in entry.phrases {
            if let Some(pos) = lower.find(phrase) {
                let end = pos + phrase.len();
                let context = extract_signal_context(text, pos, end);
                let mut signal = MemorySignal {
                    signal_type: entry.signal_type.clone(),
                    source_text: context.clone(),
                    suggested_memory_type: entry.memory_type,
                    confidence: entry.confidence,
                    suggested_name: String::new(),
                    suggested_content: context,
                };
                signal.suggested_name = suggest_memory_name(&signal);
                signals.push(signal);
                // one match per pattern group is enough
                break;
            }
        }
    }

    signals
}

/// Extract surrounding context (up to 100 chars each side) around a match.
pub fn extract_signal_context(
    text: &str,
    pattern_start: usize,
    pattern_end: usize,
) -> String {
    let start = text[..pattern_start]
        .char_indices()
        .rev()
        .nth(99)
        .map_or(0, |(i, _)| i);
    let end = text[pattern_end..]
        .char_indices()
        .nth(100)
        .map_or(text.len(), |(i, _)| pattern_end + i);
    text[start..end].trim().to_string()
}

/// Derive a short snake_case name from the first 5 words of the signal content.
pub fn suggest_memory_name(signal: &MemorySignal) -> String {
    signal
        .suggested_content
        .split_whitespace()
        .take(5)
        .collect::<Vec<_>>()
        .join("_")
        .to_lowercase()
        .replace(|c: char| !c.is_alphanumeric() && c != '_', "")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_user_correction() {
        let sigs = detect_memory_signals("Don't use tabs in this project");
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].signal_type, MemorySignalType::UserCorrection);
    }

    #[test]
    fn detect_user_preference() {
        let sigs = detect_memory_signals("I prefer spaces over tabs");
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].signal_type, MemorySignalType::UserPreference);
    }

    #[test]
    fn detect_user_role() {
        let sigs = detect_memory_signals("I'm a backend engineer working on payments");
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].signal_type, MemorySignalType::UserRole);
    }

    #[test]
    fn detect_project_context() {
        let sigs = detect_memory_signals("We're working on the v2 migration");
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].signal_type, MemorySignalType::ProjectContext);
    }

    #[test]
    fn detect_positive_feedback() {
        let sigs = detect_memory_signals("Yes exactly, keep doing that");
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].signal_type, MemorySignalType::PositiveFeedback);
    }

    #[test]
    fn detect_external_reference() {
        let sigs = detect_memory_signals("Bugs are in the Linear project INGEST");
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].signal_type, MemorySignalType::ExternalReference);
    }

    #[test]
    fn detect_explicit_remember() {
        let sigs = detect_memory_signals("Remember that we use PostgreSQL 15");
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].signal_type, MemorySignalType::ExplicitRemember);
    }

    #[test]
    fn detect_explicit_forget() {
        let sigs = detect_memory_signals("Forget about the old Redis config");
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].signal_type, MemorySignalType::ExplicitForget);
    }

    #[test]
    fn no_signals_in_plain_text() {
        let sigs = detect_memory_signals("The function returns a boolean value");
        assert!(sigs.is_empty());
    }

    #[test]
    fn multiple_signals_in_one_text() {
        let text = "I'm a data scientist. Remember that we use Python 3.11.";
        let sigs = detect_memory_signals(text);
        assert!(sigs.len() >= 2);
        let types: Vec<_> = sigs.iter().map(|s| &s.signal_type).collect();
        assert!(types.contains(&&MemorySignalType::UserRole));
        assert!(types.contains(&&MemorySignalType::ExplicitRemember));
    }

    #[test]
    fn confidence_levels_correct() {
        // ExplicitRemember should be 0.9
        let sigs = detect_memory_signals("Remember that X");
        assert!((sigs[0].confidence - 0.9).abs() < f32::EPSILON);

        // UserPreference should be 0.7
        let sigs = detect_memory_signals("I prefer Y");
        assert!((sigs[0].confidence - 0.7).abs() < f32::EPSILON);

        // ProjectContext should be 0.6
        let sigs = detect_memory_signals("The goal is Z");
        assert!((sigs[0].confidence - 0.6).abs() < f32::EPSILON);
    }

    #[test]
    fn suggested_memory_type_matches_signal() {
        let sigs = detect_memory_signals("I'm a developer");
        assert_eq!(sigs[0].suggested_memory_type, SuggestedMemoryType::User);

        let sigs = detect_memory_signals("Don't use mocks");
        assert_eq!(sigs[0].suggested_memory_type, SuggestedMemoryType::Feedback);

        let sigs = detect_memory_signals("We're working on auth");
        assert_eq!(sigs[0].suggested_memory_type, SuggestedMemoryType::Project);

        let sigs = detect_memory_signals("Bugs are in Linear");
        assert_eq!(sigs[0].suggested_memory_type, SuggestedMemoryType::Reference);
    }
}
