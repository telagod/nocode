/// Permission mode for tool execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionMode {
    /// Auto-approve all tool calls.
    Auto,
    /// Ask user for approval on each tool call.
    Ask,
    /// Deny all tool calls.
    Deny,
    /// Read-only: allow read commands, block all writes.
    ReadOnly,
}

impl Default for PermissionMode {
    fn default() -> Self {
        Self::Ask
    }
}

/// Decision returned by a permission prompter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionDecision {
    /// Allow this single call.
    Allow,
    /// Deny this single call.
    Deny,
    /// Always allow this tool (for the rest of the session).
    AlwaysAllow,
}

/// Trait for interactive permission prompting.
/// Implementations block until the user responds.
pub trait PermissionPrompter: Send + Sync {
    fn prompt(&self, tool_name: &str, arguments_summary: &str) -> PermissionDecision;
}

// ---------------------------------------------------------------------------
// QuestionPrompter — AskUserQuestion interaction bridge
// ---------------------------------------------------------------------------

/// User's answer to a question posed by AskUserQuestion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserAnswer {
    /// The selected option label for each question (index matches question order).
    pub selections: Vec<String>,
}

/// Trait for interactive question prompting (AskUserQuestion).
/// Implementations block until the user responds.
pub trait QuestionPrompter: Send + Sync {
    /// Present questions to the user and block until they answer.
    /// Returns the user's selections or an error string if cancelled/timeout.
    fn prompt_questions(&self, questions: &serde_json::Value) -> Result<UserAnswer, String>;
}

/// Auto-answer prompter that picks the first option (for non-interactive mode).
pub struct AutoFirstOptionPrompter;

impl QuestionPrompter for AutoFirstOptionPrompter {
    fn prompt_questions(&self, questions: &serde_json::Value) -> Result<UserAnswer, String> {
        let Some(arr) = questions.as_array() else {
            return Err("Invalid questions format".to_string());
        };
        let selections = arr
            .iter()
            .map(|q| {
                q["options"]
                    .as_array()
                    .and_then(|opts| opts.first())
                    .and_then(|o| o["label"].as_str())
                    .unwrap_or("N/A")
                    .to_string()
            })
            .collect();
        Ok(UserAnswer { selections })
    }
}

/// Auto-approve prompter (for non-interactive / Auto mode).
pub struct AutoApprovePrompter;

impl PermissionPrompter for AutoApprovePrompter {
    fn prompt(&self, _tool_name: &str, _arguments_summary: &str) -> PermissionDecision {
        PermissionDecision::Allow
    }
}

/// Auto-deny prompter (for Deny mode).
pub struct AutoDenyPrompter;

impl PermissionPrompter for AutoDenyPrompter {
    fn prompt(&self, _tool_name: &str, _arguments_summary: &str) -> PermissionDecision {
        PermissionDecision::Deny
    }
}

// ---------------------------------------------------------------------------
// ToolClassifier — risk-based automatic approval
// ---------------------------------------------------------------------------

/// Risk level assigned to a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ToolRiskLevel {
    /// Read-only operations — auto-approve.
    Safe,
    /// Write operations — require single confirmation.
    Write,
    /// Destructive or high-impact — require explicit confirmation.
    Destructive,
}

/// Classifier approval decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassifierApproval {
    /// Auto-approved based on risk level.
    AutoApproved,
    /// Needs user confirmation (single).
    NeedsConfirmation,
    /// Needs explicit double-confirmation (destructive).
    NeedsExplicitConfirmation,
}

/// Classifies tools by risk level and determines approval requirements.
pub struct ToolClassifier;

impl ToolClassifier {
    /// Classify a tool call's risk level.
    pub fn classify(tool_name: &str, input: &serde_json::Value) -> ToolRiskLevel {
        match tool_name {
            // Safe: read-only tools
            "FileRead" | "Glob" | "Grep" | "TaskGet" | "TaskList" | "TaskOutput"
            | "MemoryList" | "MemorySearch" | "CronList" | "ToolSearch"
            | "ListMcpResources" | "ReadMcpResource" | "AskUserQuestion"
            | "ExitPlanMode" => ToolRiskLevel::Safe,

            // Bash: depends on command content
            "Bash" => Self::classify_bash(input),

            // Write: file modifications
            "FileWrite" | "FileEdit" | "NotebookEdit" | "TodoWrite"
            | "Config" | "MemorySave" | "MemoryDelete" => ToolRiskLevel::Write,

            // Potentially destructive
            "EnterWorktree" | "ExitWorktree" => ToolRiskLevel::Write,

            // Agent/Team: spawning processes
            "Agent" | "TeamCreate" | "TeamDelete" | "SendMessage" => ToolRiskLevel::Write,

            // Web: external network access
            "WebFetch" | "WebSearch" => ToolRiskLevel::Write,

            // MCP: external tool execution
            "Mcp" => ToolRiskLevel::Write,

            // Cron: scheduled execution
            "CronCreate" | "CronDelete" => ToolRiskLevel::Write,

            // Unknown tools default to Write
            _ => ToolRiskLevel::Write,
        }
    }

    /// Determine approval requirement based on risk level and permission mode.
    pub fn approval_for(risk: ToolRiskLevel, mode: PermissionMode) -> ClassifierApproval {
        match mode {
            PermissionMode::Auto => ClassifierApproval::AutoApproved,
            PermissionMode::Deny => ClassifierApproval::NeedsExplicitConfirmation,
            PermissionMode::ReadOnly => {
                if risk == ToolRiskLevel::Safe {
                    ClassifierApproval::AutoApproved
                } else {
                    ClassifierApproval::NeedsExplicitConfirmation
                }
            }
            PermissionMode::Ask => match risk {
                ToolRiskLevel::Safe => ClassifierApproval::AutoApproved,
                ToolRiskLevel::Write => ClassifierApproval::NeedsConfirmation,
                ToolRiskLevel::Destructive => ClassifierApproval::NeedsExplicitConfirmation,
            },
        }
    }

    fn classify_bash(input: &serde_json::Value) -> ToolRiskLevel {
        let cmd = input["command"].as_str().unwrap_or("");
        if cmd.is_empty() {
            return ToolRiskLevel::Write;
        }

        // Destructive patterns
        let destructive_patterns = [
            "rm -rf", "rm -r", "mkfs", "dd if=", "shutdown", "reboot",
            "kill -9", "pkill", "DROP TABLE", "DROP DATABASE",
            "truncate", "format", "> /dev/",
        ];
        let cmd_lower = cmd.to_lowercase();
        for pattern in &destructive_patterns {
            if cmd_lower.contains(&pattern.to_lowercase()) {
                return ToolRiskLevel::Destructive;
            }
        }

        // Read-only check
        if crate::tool::bash_validation::is_read_only_command(cmd) {
            return ToolRiskLevel::Safe;
        }

        ToolRiskLevel::Write
    }
}

// ---------------------------------------------------------------------------
// PermissionRule — persistent per-tool permission rules
// ---------------------------------------------------------------------------

use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, OnceLock};

/// A persistent permission rule for a specific tool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PermissionRule {
    pub tool_name: String,
    pub action: RuleAction,
    /// Optional argument pattern (e.g., command contains "docker").
    #[serde(default)]
    pub argument_pattern: Option<String>,
}

/// What a rule does when matched.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuleAction {
    Allow,
    Deny,
    AlwaysAsk,
}

/// Persistent store for permission rules (JSON file).
pub struct PermissionRuleStore {
    path: std::path::PathBuf,
    rules: Vec<PermissionRule>,
}

impl PermissionRuleStore {
    pub fn new(path: &str) -> Self {
        let path = std::path::PathBuf::from(path);
        let rules = Self::load_from_file(&path);
        Self { path, rules }
    }

    /// Add a rule. Replaces existing rule for same tool+pattern.
    pub fn add(&mut self, rule: PermissionRule) -> Result<(), String> {
        self.rules.retain(|r| {
            !(r.tool_name == rule.tool_name && r.argument_pattern == rule.argument_pattern)
        });
        self.rules.push(rule);
        self.save()
    }

    /// Remove a rule by tool name (and optional pattern).
    pub fn remove(&mut self, tool_name: &str, pattern: Option<&str>) -> Result<bool, String> {
        let before = self.rules.len();
        self.rules.retain(|r| {
            !(r.tool_name == tool_name && r.argument_pattern.as_deref() == pattern)
        });
        let removed = self.rules.len() < before;
        if removed {
            self.save()?;
        }
        Ok(removed)
    }

    /// Check if a rule matches a tool call. Returns the action if matched.
    pub fn check(&self, tool_name: &str, args_summary: &str) -> Option<RuleAction> {
        // More specific rules (with pattern) take priority
        for rule in &self.rules {
            if rule.tool_name != tool_name && rule.tool_name != "*" {
                continue;
            }
            if let Some(pattern) = &rule.argument_pattern
                && args_summary.to_lowercase().contains(&pattern.to_lowercase())
            {
                return Some(rule.action);
            }
        }
        // Then check rules without patterns
        for rule in &self.rules {
            if rule.tool_name != tool_name && rule.tool_name != "*" {
                continue;
            }
            if rule.argument_pattern.is_none() {
                return Some(rule.action);
            }
        }
        None
    }

    /// List all rules.
    pub fn list(&self) -> &[PermissionRule] {
        &self.rules
    }

    /// Clear all rules.
    pub fn clear(&mut self) -> Result<(), String> {
        self.rules.clear();
        self.save()
    }

    fn save(&self) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let json = serde_json::to_string_pretty(&self.rules)
            .map_err(|e| format!("serialize error: {e}"))?;
        std::fs::write(&self.path, json).map_err(|e| format!("write error: {e}"))
    }

    fn load_from_file(path: &std::path::PathBuf) -> Vec<PermissionRule> {
        let Ok(raw) = std::fs::read_to_string(path) else {
            return Vec::new();
        };
        serde_json::from_str(&raw).unwrap_or_default()
    }
}

/// Global singleton permission rule store.
static GLOBAL_PERMISSION_RULES: OnceLock<Arc<Mutex<PermissionRuleStore>>> = OnceLock::new();

pub fn global_permission_rules() -> &'static Arc<Mutex<PermissionRuleStore>> {
    GLOBAL_PERMISSION_RULES.get_or_init(|| {
        let home = std::env::var("HOME").unwrap_or_default();
        let path = format!("{home}/.nocode/permission_rules.json");
        Arc::new(Mutex::new(PermissionRuleStore::new(&path)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_approve_always_allows() {
        let p = AutoApprovePrompter;
        assert_eq!(p.prompt("Bash", "cmd=ls"), PermissionDecision::Allow);
    }

    #[test]
    fn auto_deny_always_denies() {
        let p = AutoDenyPrompter;
        assert_eq!(p.prompt("Bash", "cmd=ls"), PermissionDecision::Deny);
    }

    #[test]
    fn default_mode_is_ask() {
        assert_eq!(PermissionMode::default(), PermissionMode::Ask);
    }

    // --- ToolClassifier ---

    #[test]
    fn classify_read_tools_as_safe() {
        let input = serde_json::json!({});
        assert_eq!(ToolClassifier::classify("FileRead", &input), ToolRiskLevel::Safe);
        assert_eq!(ToolClassifier::classify("Glob", &input), ToolRiskLevel::Safe);
        assert_eq!(ToolClassifier::classify("Grep", &input), ToolRiskLevel::Safe);
        assert_eq!(ToolClassifier::classify("TaskList", &input), ToolRiskLevel::Safe);
    }

    #[test]
    fn classify_write_tools() {
        let input = serde_json::json!({});
        assert_eq!(ToolClassifier::classify("FileWrite", &input), ToolRiskLevel::Write);
        assert_eq!(ToolClassifier::classify("FileEdit", &input), ToolRiskLevel::Write);
        assert_eq!(ToolClassifier::classify("Agent", &input), ToolRiskLevel::Write);
        assert_eq!(ToolClassifier::classify("WebFetch", &input), ToolRiskLevel::Write);
    }

    #[test]
    fn classify_bash_read_only() {
        let input = serde_json::json!({"command": "ls -la"});
        assert_eq!(ToolClassifier::classify("Bash", &input), ToolRiskLevel::Safe);
    }

    #[test]
    fn classify_bash_write() {
        let input = serde_json::json!({"command": "cp file1.txt file2.txt"});
        assert_eq!(ToolClassifier::classify("Bash", &input), ToolRiskLevel::Write);
    }

    #[test]
    fn classify_bash_destructive() {
        let input = serde_json::json!({"command": "rm -rf /tmp/stuff"});
        assert_eq!(ToolClassifier::classify("Bash", &input), ToolRiskLevel::Destructive);
    }

    #[test]
    fn classify_bash_destructive_patterns() {
        for cmd in &["mkfs /dev/sda", "dd if=/dev/zero", "shutdown -h now", "DROP TABLE users"] {
            let input = serde_json::json!({"command": cmd});
            assert_eq!(
                ToolClassifier::classify("Bash", &input),
                ToolRiskLevel::Destructive,
                "Expected Destructive for: {cmd}"
            );
        }
    }

    #[test]
    fn approval_auto_mode_always_approves() {
        assert_eq!(
            ToolClassifier::approval_for(ToolRiskLevel::Destructive, PermissionMode::Auto),
            ClassifierApproval::AutoApproved
        );
    }

    #[test]
    fn approval_ask_mode_safe_auto() {
        assert_eq!(
            ToolClassifier::approval_for(ToolRiskLevel::Safe, PermissionMode::Ask),
            ClassifierApproval::AutoApproved
        );
    }

    #[test]
    fn approval_ask_mode_write_needs_confirm() {
        assert_eq!(
            ToolClassifier::approval_for(ToolRiskLevel::Write, PermissionMode::Ask),
            ClassifierApproval::NeedsConfirmation
        );
    }

    #[test]
    fn approval_ask_mode_destructive_needs_explicit() {
        assert_eq!(
            ToolClassifier::approval_for(ToolRiskLevel::Destructive, PermissionMode::Ask),
            ClassifierApproval::NeedsExplicitConfirmation
        );
    }

    #[test]
    fn approval_readonly_blocks_writes() {
        assert_eq!(
            ToolClassifier::approval_for(ToolRiskLevel::Write, PermissionMode::ReadOnly),
            ClassifierApproval::NeedsExplicitConfirmation
        );
        assert_eq!(
            ToolClassifier::approval_for(ToolRiskLevel::Safe, PermissionMode::ReadOnly),
            ClassifierApproval::AutoApproved
        );
    }

    #[test]
    fn unknown_tool_defaults_to_write() {
        let input = serde_json::json!({});
        assert_eq!(ToolClassifier::classify("SomeNewTool", &input), ToolRiskLevel::Write);
    }

    // --- PermissionRuleStore ---

    #[test]
    fn rule_store_add_and_check() {
        let tmp = format!("/tmp/nocode_perm_test_{}.json", std::process::id());
        let _ = std::fs::remove_file(&tmp);
        let mut store = PermissionRuleStore::new(&tmp);
        store.add(PermissionRule {
            tool_name: "Bash".to_string(),
            action: RuleAction::Allow,
            argument_pattern: None,
        }).unwrap();
        assert_eq!(store.check("Bash", ""), Some(RuleAction::Allow));
        assert_eq!(store.check("FileWrite", ""), None);
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn rule_store_pattern_match() {
        let tmp = format!("/tmp/nocode_perm_test2_{}.json", std::process::id());
        let _ = std::fs::remove_file(&tmp);
        let mut store = PermissionRuleStore::new(&tmp);
        store.add(PermissionRule {
            tool_name: "Bash".to_string(),
            action: RuleAction::Deny,
            argument_pattern: Some("docker".to_string()),
        }).unwrap();
        // Pattern match
        assert_eq!(store.check("Bash", "docker run nginx"), Some(RuleAction::Deny));
        // No pattern match — no rule without pattern
        assert_eq!(store.check("Bash", "ls -la"), None);
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn rule_store_remove() {
        let tmp = format!("/tmp/nocode_perm_test3_{}.json", std::process::id());
        let _ = std::fs::remove_file(&tmp);
        let mut store = PermissionRuleStore::new(&tmp);
        store.add(PermissionRule {
            tool_name: "FileWrite".to_string(),
            action: RuleAction::Allow,
            argument_pattern: None,
        }).unwrap();
        assert!(store.remove("FileWrite", None).unwrap());
        assert_eq!(store.check("FileWrite", ""), None);
        assert!(!store.remove("FileWrite", None).unwrap());
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn rule_store_persist_and_reload() {
        let tmp = format!("/tmp/nocode_perm_test4_{}.json", std::process::id());
        let _ = std::fs::remove_file(&tmp);
        {
            let mut store = PermissionRuleStore::new(&tmp);
            store.add(PermissionRule {
                tool_name: "Agent".to_string(),
                action: RuleAction::Deny,
                argument_pattern: None,
            }).unwrap();
        }
        let store2 = PermissionRuleStore::new(&tmp);
        assert_eq!(store2.check("Agent", ""), Some(RuleAction::Deny));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn rule_store_wildcard() {
        let tmp = format!("/tmp/nocode_perm_test5_{}.json", std::process::id());
        let _ = std::fs::remove_file(&tmp);
        let mut store = PermissionRuleStore::new(&tmp);
        store.add(PermissionRule {
            tool_name: "*".to_string(),
            action: RuleAction::AlwaysAsk,
            argument_pattern: None,
        }).unwrap();
        assert_eq!(store.check("Bash", ""), Some(RuleAction::AlwaysAsk));
        assert_eq!(store.check("FileWrite", ""), Some(RuleAction::AlwaysAsk));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn rule_store_clear() {
        let tmp = format!("/tmp/nocode_perm_test6_{}.json", std::process::id());
        let _ = std::fs::remove_file(&tmp);
        let mut store = PermissionRuleStore::new(&tmp);
        store.add(PermissionRule {
            tool_name: "Bash".to_string(),
            action: RuleAction::Allow,
            argument_pattern: None,
        }).unwrap();
        store.clear().unwrap();
        assert!(store.list().is_empty());
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn rule_store_replaces_duplicate() {
        let tmp = format!("/tmp/nocode_perm_test7_{}.json", std::process::id());
        let _ = std::fs::remove_file(&tmp);
        let mut store = PermissionRuleStore::new(&tmp);
        store.add(PermissionRule {
            tool_name: "Bash".to_string(),
            action: RuleAction::Allow,
            argument_pattern: None,
        }).unwrap();
        store.add(PermissionRule {
            tool_name: "Bash".to_string(),
            action: RuleAction::Deny,
            argument_pattern: None,
        }).unwrap();
        assert_eq!(store.list().len(), 1);
        assert_eq!(store.check("Bash", ""), Some(RuleAction::Deny));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn rule_action_serde_roundtrip() {
        for action in &[RuleAction::Allow, RuleAction::Deny, RuleAction::AlwaysAsk] {
            let json = serde_json::to_string(action).unwrap();
            let parsed: RuleAction = serde_json::from_str(&json).unwrap();
            assert_eq!(&parsed, action);
        }
    }
}
