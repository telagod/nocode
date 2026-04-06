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
        memory_type: SuggestedMemoryType::User, // refined by infer_memory_type()
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
        memory_type: SuggestedMemoryType::User, // refined by infer_memory_type()
        confidence: 0.9,
    },
];

// ---------------------------------------------------------------------------
// Content-based memory type inference
// ---------------------------------------------------------------------------

/// Infer the most appropriate memory type from the signal content.
///
/// Used to refine `ExplicitRemember` and `ExplicitForget` signals whose
/// pattern table entry defaults to `User`. Scans the content for secondary
/// cues that indicate Feedback, Project, or Reference types.
fn infer_memory_type(content: &str) -> SuggestedMemoryType {
    let lower = content.to_lowercase();

    // Feedback cues: preferences, corrections, workflow guidance
    const FEEDBACK_CUES: &[&str] = &[
        "prefer",
        "always use",
        "never use",
        "don't use",
        "do not use",
        "stop ",
        "avoid ",
        "instead of",
        "rather than",
        "keep doing",
    ];
    // Project cues: team context, deadlines, goals, infrastructure
    const PROJECT_CUES: &[&str] = &[
        "we use",
        "we're using",
        "the project",
        "the team",
        "the deadline",
        "the goal",
        "we need",
        "we have",
        "our ",
        "the repo",
        "the codebase",
        "the database",
        "the server",
        "the api",
        "the service",
        "version",
        "deploy",
        "migration",
        "sprint",
        "release",
    ];
    // Reference cues: external systems, URLs, dashboards
    const REFERENCE_CUES: &[&str] = &[
        "tracked in",
        "dashboard",
        "http://",
        "https://",
        "slack",
        "linear",
        "jira",
        "grafana",
        "confluence",
        "notion",
        "the docs at",
        "the board",
        "the channel",
    ];

    // Score each type by counting cue matches
    let feedback_score = FEEDBACK_CUES.iter().filter(|c| lower.contains(*c)).count();
    let project_score = PROJECT_CUES.iter().filter(|c| lower.contains(*c)).count();
    let reference_score = REFERENCE_CUES.iter().filter(|c| lower.contains(*c)).count();

    // User cues: role, identity (checked via existing UserRole pattern)
    let user_score = ["i'm a", "i am a", "my role", "i work on", "i work as"]
        .iter()
        .filter(|c| lower.contains(*c))
        .count();

    let max = feedback_score
        .max(project_score)
        .max(reference_score)
        .max(user_score);
    if max == 0 {
        return SuggestedMemoryType::User; // no cues → default
    }

    // Tie-break order: Feedback > Project > Reference > User
    if feedback_score == max {
        SuggestedMemoryType::Feedback
    } else if project_score == max {
        SuggestedMemoryType::Project
    } else if reference_score == max {
        SuggestedMemoryType::Reference
    } else {
        SuggestedMemoryType::User
    }
}

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
                // Refine memory type for explicit remember/forget signals
                if matches!(
                    signal.signal_type,
                    MemorySignalType::ExplicitRemember | MemorySignalType::ExplicitForget
                ) {
                    signal.suggested_memory_type = infer_memory_type(&signal.source_text);
                }
                signal.suggested_name = suggest_memory_name(&signal);
                signals.push(signal);
                // one match per pattern group is enough
                break;
            }
        }
    }

    signals
}

// ---------------------------------------------------------------------------
// MemorySignalProcessor — bridges signals to MemoryStore
// ---------------------------------------------------------------------------

use crate::memory_store::{MemoryEntry, MemoryStore, MemoryType};

/// Converts a [`SuggestedMemoryType`] to a [`MemoryType`].
fn to_memory_type(suggested: SuggestedMemoryType) -> MemoryType {
    match suggested {
        SuggestedMemoryType::User => MemoryType::User,
        SuggestedMemoryType::Feedback => MemoryType::Feedback,
        SuggestedMemoryType::Project => MemoryType::Project,
        SuggestedMemoryType::Reference => MemoryType::Reference,
    }
}

/// Result of processing memory signals for a piece of text.
#[derive(Debug, Clone, Default)]
pub struct SignalProcessingResult {
    pub signals_detected: usize,
    pub memories_saved: usize,
    pub memories_deleted: usize,
    pub errors: Vec<String>,
}

/// Minimum confidence threshold for auto-saving a memory signal.
const DEFAULT_CONFIDENCE_THRESHOLD: f32 = 0.7;

/// Process detected memory signals: save new memories, handle forget requests.
///
/// - Signals with confidence >= threshold are auto-saved to the store.
/// - `ExplicitForget` signals trigger a search + delete.
/// - `ExplicitRemember` signals are always saved regardless of threshold.
pub fn process_signals(
    store: &MemoryStore,
    text: &str,
    confidence_threshold: Option<f32>,
) -> SignalProcessingResult {
    let threshold = confidence_threshold.unwrap_or(DEFAULT_CONFIDENCE_THRESHOLD);
    let signals = detect_memory_signals(text);
    let mut result = SignalProcessingResult {
        signals_detected: signals.len(),
        ..Default::default()
    };

    for signal in &signals {
        match signal.signal_type {
            MemorySignalType::ExplicitForget => {
                // Try to find and delete matching memory.
                // Extract search terms by stripping the trigger phrase prefix.
                let query = extract_forget_query(&signal.source_text);
                match store.search(&query) {
                    Ok(matches) => {
                        for m in matches {
                            if let Err(e) = store.delete(&m.file_name) {
                                result.errors.push(e);
                            } else {
                                let _ = store.remove_from_index(&m.file_name);
                                result.memories_deleted += 1;
                            }
                        }
                    }
                    Err(e) => result.errors.push(e),
                }
            }
            MemorySignalType::ExplicitRemember => {
                // Always save explicit remember requests.
                let file_name = format!("{}.md", sanitize_filename(&signal.suggested_name));
                let entry = MemoryEntry {
                    name: signal.suggested_name.clone(),
                    description: truncate(&signal.source_text, 100),
                    memory_type: to_memory_type(signal.suggested_memory_type),
                    content: signal.suggested_content.clone(),
                    file_name,
                };
                match store.save(&entry) {
                    Ok(()) => {
                        let _ = store.add_to_index(&entry);
                        result.memories_saved += 1;
                    }
                    Err(e) => result.errors.push(e),
                }
            }
            _ => {
                if signal.confidence >= threshold {
                    let file_name = format!("{}.md", sanitize_filename(&signal.suggested_name));
                    let entry = MemoryEntry {
                        name: signal.suggested_name.clone(),
                        description: truncate(&signal.source_text, 100),
                        memory_type: to_memory_type(signal.suggested_memory_type),
                        content: signal.suggested_content.clone(),
                        file_name,
                    };
                    match store.save(&entry) {
                        Ok(()) => {
                            let _ = store.add_to_index(&entry);
                            result.memories_saved += 1;
                        }
                        Err(e) => result.errors.push(e),
                    }
                }
            }
        }
    }

    result
}

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}

/// Extract meaningful search terms from a forget request by stripping trigger phrases.
fn extract_forget_query(text: &str) -> String {
    let lower = text.to_lowercase();
    const FORGET_PREFIXES: &[&str] = &[
        "forget about ",
        "forget that ",
        "forget ",
        "remove the memory about ",
        "remove the memory of ",
        "remove the memory ",
        "don't remember ",
    ];
    for prefix in FORGET_PREFIXES {
        if let Some(rest) = lower.strip_prefix(prefix) {
            let trimmed = rest
                .trim_start_matches("the ")
                .trim_start_matches("old ")
                .trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    // Fallback: use individual words (3+ chars) as search terms
    lower
        .split_whitespace()
        .filter(|w| {
            w.len() >= 3
                && ![
                    "forget", "about", "that", "the", "don't", "remember", "remove", "memory",
                ]
                .contains(w)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Extract surrounding context (up to 100 chars each side) around a match.
pub fn extract_signal_context(text: &str, pattern_start: usize, pattern_end: usize) -> String {
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
        assert_eq!(
            sigs[0].suggested_memory_type,
            SuggestedMemoryType::Reference
        );
    }

    // -----------------------------------------------------------------------
    // process_signals tests
    // -----------------------------------------------------------------------

    #[test]
    fn process_signals_saves_high_confidence() {
        let tmp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(tmp.path().to_str().unwrap());
        let result = process_signals(&store, "I'm a security researcher", None);
        assert_eq!(result.signals_detected, 1);
        assert_eq!(result.memories_saved, 1);
        assert!(result.errors.is_empty());
        let all = store.list().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].memory_type, MemoryType::User);
    }

    #[test]
    fn process_signals_skips_low_confidence() {
        let tmp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(tmp.path().to_str().unwrap());
        // ProjectContext has confidence 0.6, threshold 0.7 → skip
        let result = process_signals(&store, "The goal is to ship v2", Some(0.7));
        assert_eq!(result.signals_detected, 1);
        assert_eq!(result.memories_saved, 0);
    }

    #[test]
    fn process_signals_explicit_remember_always_saves() {
        let tmp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(tmp.path().to_str().unwrap());
        // ExplicitRemember has confidence 0.9, but even with threshold 1.0 it should save
        let result = process_signals(&store, "Remember that we use Rust 2024 edition", Some(1.0));
        assert_eq!(result.memories_saved, 1);
    }

    #[test]
    fn process_signals_explicit_forget_deletes() {
        let tmp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(tmp.path().to_str().unwrap());
        // First save something about Redis
        let entry = MemoryEntry {
            name: "redis_config".to_string(),
            description: "old Redis config".to_string(),
            memory_type: MemoryType::Project,
            content: "Redis is on port 6379".to_string(),
            file_name: "redis_config.md".to_string(),
        };
        store.save(&entry).unwrap();
        store.add_to_index(&entry).unwrap();
        assert_eq!(store.list().unwrap().len(), 1);

        // Now forget it
        let result = process_signals(&store, "Forget about the old Redis config", None);
        assert!(result.memories_deleted >= 1);
    }

    #[test]
    fn process_signals_no_signals_no_ops() {
        let tmp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(tmp.path().to_str().unwrap());
        let result = process_signals(&store, "The function returns a boolean", None);
        assert_eq!(result.signals_detected, 0);
        assert_eq!(result.memories_saved, 0);
        assert_eq!(result.memories_deleted, 0);
    }

    #[test]
    fn process_signals_multiple_saves() {
        let tmp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(tmp.path().to_str().unwrap());
        let result = process_signals(
            &store,
            "I'm a backend engineer. Don't use mocks in tests.",
            None,
        );
        assert!(result.signals_detected >= 2);
        assert!(result.memories_saved >= 2);
    }

    #[test]
    fn sanitize_filename_works() {
        assert_eq!(sanitize_filename("hello_world"), "hello_world");
        assert_eq!(sanitize_filename("hello world!"), "hello_world");
        assert_eq!(sanitize_filename("__test__"), "test");
    }

    #[test]
    fn truncate_works() {
        assert_eq!(truncate("short", 100), "short");
        assert_eq!(truncate("hello world this is long", 10), "hello w...");
    }

    // -----------------------------------------------------------------------
    // infer_memory_type tests
    // -----------------------------------------------------------------------

    #[test]
    fn infer_type_feedback_from_preference_cues() {
        assert_eq!(
            infer_memory_type("I prefer spaces over tabs"),
            SuggestedMemoryType::Feedback
        );
        assert_eq!(
            infer_memory_type("always use snake_case"),
            SuggestedMemoryType::Feedback
        );
        assert_eq!(
            infer_memory_type("avoid using mocks in tests"),
            SuggestedMemoryType::Feedback
        );
    }

    #[test]
    fn infer_type_project_from_team_cues() {
        assert_eq!(
            infer_memory_type("we use PostgreSQL 15"),
            SuggestedMemoryType::Project
        );
        assert_eq!(
            infer_memory_type("the database is on port 5432"),
            SuggestedMemoryType::Project
        );
        assert_eq!(
            infer_memory_type("the deadline is next Friday"),
            SuggestedMemoryType::Project
        );
    }

    #[test]
    fn infer_type_reference_from_external_cues() {
        assert_eq!(
            infer_memory_type("bugs are tracked in Linear"),
            SuggestedMemoryType::Reference
        );
        assert_eq!(
            infer_memory_type("the dashboard at https://grafana.internal"),
            SuggestedMemoryType::Reference
        );
    }

    #[test]
    fn infer_type_user_from_role_cues() {
        assert_eq!(
            infer_memory_type("I'm a backend engineer"),
            SuggestedMemoryType::User
        );
        assert_eq!(
            infer_memory_type("I work as a security researcher"),
            SuggestedMemoryType::User
        );
    }

    #[test]
    fn infer_type_defaults_to_user_with_no_cues() {
        assert_eq!(
            infer_memory_type("something random here"),
            SuggestedMemoryType::User
        );
    }

    #[test]
    fn explicit_remember_infers_project_type() {
        let sigs = detect_memory_signals("Remember that we use PostgreSQL 15");
        assert_eq!(sigs[0].signal_type, MemorySignalType::ExplicitRemember);
        assert_eq!(sigs[0].suggested_memory_type, SuggestedMemoryType::Project);
    }

    #[test]
    fn explicit_remember_infers_feedback_type() {
        let sigs = detect_memory_signals("Remember that you should always use snake_case");
        let remember_sig = sigs
            .iter()
            .find(|s| s.signal_type == MemorySignalType::ExplicitRemember)
            .expect("should detect ExplicitRemember");
        assert_eq!(
            remember_sig.suggested_memory_type,
            SuggestedMemoryType::Feedback
        );
    }

    #[test]
    fn explicit_forget_infers_reference_type() {
        let sigs = detect_memory_signals("Forget about the old Grafana board");
        let forget_sig = sigs
            .iter()
            .find(|s| s.signal_type == MemorySignalType::ExplicitForget)
            .expect("should detect ExplicitForget");
        assert_eq!(
            forget_sig.suggested_memory_type,
            SuggestedMemoryType::Reference
        );
    }
}
