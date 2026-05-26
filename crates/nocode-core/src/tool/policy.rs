//! Policy engine — the **explainable** gate.
//!
//! ## Why this exists
//!
//! Before this module, a tool call walked through six top-level gates inlined
//! in `executor.rs` (trust → hooks → permission → bash → sandbox → execute), each
//! returning a bare boolean or a custom error string. The harness was strict but
//! opaque: a denial said "Permission denied for tool 'X'" with no explanation of
//! *which* gate produced the decision or *why*.
//!
//! `PolicyEngine` collapses the conceptual surface to three:
//!
//! 1. **Schema** — JSON-schema validation (still in [`super::tool_validation`]).
//! 2. **Policy** — trust + permission-mode + classifier + sandbox-path, unified.
//! 3. **Hooks** — external `PreToolUse` / `PostToolUse` commands (informational
//!    or denying, owned by [`super::hook_runner`]).
//!
//! The engine returns a [`GateDecision`] that carries the *gate* and *reason*
//! that produced it, so the TUI can render a "why-trail" instead of a flat
//! refusal. The Executor still has the final say (it can choose to error a
//! bridged-tool call differently, etc.), but the *judgement* is now in one
//! file you can read top-to-bottom.
//!
//! ## Backwards compatibility
//!
//! This file is purely additive. `executor.rs` continues to gate through its
//! existing inline checks — we wire the engine in incrementally and let tests
//! prove it. Once parity is verified, the inline checks become this engine's
//! delegates.

use crate::config::runtime::SandboxConfig;
use crate::tool::file_safety;
use crate::tool::permission::{
    ClassifierApproval, PermissionDecision, PermissionMode, PermissionPrompter, ToolClassifier,
    ToolRiskLevel,
};
use crate::tool::session_tools::is_plan_mode;
use crate::tool::trust::{PermissionEnforcer, TrustDecision};
use serde_json::Value;

/// Which gate produced the decision — surfaced in [`GateDecision::gate`] so the
/// TUI / logs can render a precise trail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateName {
    /// `PlanMode` forbids write tools regardless of permission_mode.
    PlanMode,
    /// `Trust` policy explicitly listed the tool as deny/allow.
    Trust,
    /// `PermissionMode` (Auto/Ask/Deny/ReadOnly) combined with the
    /// risk classifier.
    PermissionMode,
    /// Interactive prompter (user clicked Deny / AlwaysAllow / Allow).
    Prompter,
    /// Sandbox path/network policy.
    Sandbox,
}

/// Outcome of evaluating the policy gate. Always pair the verdict with a
/// human-readable reason — the whole point of this module is *explainability*.
#[derive(Debug, Clone)]
pub enum GateDecision {
    /// Tool may proceed. `reason` may be empty for the common allow case but
    /// is populated whenever a non-trivial reason exists (e.g. "auto-allowed
    /// because PermissionMode::Auto").
    Allow {
        gate: GateName,
        reason: String,
        /// True if the prompter asked "always allow"; the caller should record
        /// the tool name in its session allow-list.
        remember: bool,
    },
    /// Tool was denied. `reason` MUST be populated.
    Deny { gate: GateName, reason: String },
}

impl GateDecision {
    pub const fn is_allow(&self) -> bool {
        matches!(self, Self::Allow { .. })
    }

    pub fn deny<S: Into<String>>(gate: GateName, reason: S) -> Self {
        Self::Deny {
            gate,
            reason: reason.into(),
        }
    }

    pub fn allow<S: Into<String>>(gate: GateName, reason: S) -> Self {
        Self::Allow {
            gate,
            reason: reason.into(),
            remember: false,
        }
    }

    pub fn allow_remember<S: Into<String>>(gate: GateName, reason: S) -> Self {
        Self::Allow {
            gate,
            reason: reason.into(),
            remember: true,
        }
    }

    /// Format the decision in the standard `gate: reason` style used by the
    /// TUI and tool-result error messages.
    pub fn format_trail(&self) -> String {
        match self {
            Self::Allow { gate, reason, .. } => {
                if reason.is_empty() {
                    format!("{} ok", gate_label(*gate))
                } else {
                    format!("{}: {}", gate_label(*gate), reason)
                }
            }
            Self::Deny { gate, reason } => format!("{}: {}", gate_label(*gate), reason),
        }
    }

    /// Convenience: get the gate that produced this decision.
    pub const fn gate(&self) -> GateName {
        match self {
            Self::Allow { gate, .. } | Self::Deny { gate, .. } => *gate,
        }
    }

    /// Convenience: borrow the reason string.
    pub fn reason(&self) -> &str {
        match self {
            Self::Allow { reason, .. } | Self::Deny { reason, .. } => reason,
        }
    }
}

fn gate_label(gate: GateName) -> &'static str {
    match gate {
        GateName::PlanMode => "plan-mode",
        GateName::Trust => "trust",
        GateName::PermissionMode => "permission",
        GateName::Prompter => "prompter",
        GateName::Sandbox => "sandbox",
    }
}

/// The unified policy gate. Built fresh per tool call — cheap.
pub struct PolicyEngine<'a> {
    pub mode: PermissionMode,
    pub trust: Option<&'a PermissionEnforcer>,
    pub sandbox: Option<&'a SandboxConfig>,
    pub prompter: Option<&'a dyn PermissionPrompter>,
    /// Whether the tool name was previously marked "always allowed" by the
    /// prompter. Caller looks this up from its session state.
    pub previously_allowed: bool,
}

impl<'a> PolicyEngine<'a> {
    /// Evaluate the gate for a tool call. Returns the first non-allowing
    /// decision encountered, or the most informative allow if everything
    /// passes.
    pub fn evaluate(&self, tool_name: &str, input: &Value) -> GateDecision {
        // 1. Plan mode — preempts everything.
        if is_plan_mode() && !is_read_only_for_plan(tool_name, input) {
            return GateDecision::deny(
                GateName::PlanMode,
                format!("'{tool_name}' is a write tool; plan mode is read-only"),
            );
        }

        // 2. Trust policy
        if let Some(enforcer) = self.trust {
            match enforcer.check(tool_name, "model") {
                TrustDecision::Deny => {
                    return GateDecision::deny(
                        GateName::Trust,
                        format!("trust policy lists '{tool_name}' in deny_labels"),
                    );
                }
                TrustDecision::Allow | TrustDecision::PromptUser => {}
            }
        }

        // 3. Permission mode + risk classifier
        let risk = ToolClassifier::classify(tool_name, input);
        let perm_decision = self.permission_decision(tool_name, input, risk);
        if !perm_decision.is_allow() {
            return perm_decision;
        }

        // 4. Sandbox (path/network)
        if let Some(sandbox) = self.sandbox
            && sandbox.enabled
            && let Some(violation) = sandbox_violation(tool_name, input, sandbox)
        {
            return GateDecision::deny(GateName::Sandbox, violation);
        }

        perm_decision
    }

    fn permission_decision(
        &self,
        tool_name: &str,
        input: &Value,
        risk: ToolRiskLevel,
    ) -> GateDecision {
        let approval = ToolClassifier::approval_for(risk, self.mode);

        match self.mode {
            PermissionMode::Auto => GateDecision::allow(
                GateName::PermissionMode,
                format!("auto-mode; classified {}", risk_label(risk)),
            ),
            PermissionMode::Deny => GateDecision::deny(
                GateName::PermissionMode,
                "permission_mode=deny rejects every tool".to_owned(),
            ),
            PermissionMode::ReadOnly => {
                if risk == ToolRiskLevel::Safe {
                    GateDecision::allow(
                        GateName::PermissionMode,
                        "read-only mode and tool is classified Safe".to_owned(),
                    )
                } else {
                    GateDecision::deny(
                        GateName::PermissionMode,
                        format!(
                            "read-only mode rejects tool classified {}",
                            risk_label(risk)
                        ),
                    )
                }
            }
            PermissionMode::Ask => {
                if approval == ClassifierApproval::AutoApproved {
                    return GateDecision::allow(
                        GateName::PermissionMode,
                        "ask-mode auto-approves Safe-classified tools".to_owned(),
                    );
                }
                if self.previously_allowed {
                    return GateDecision::allow(
                        GateName::PermissionMode,
                        format!("'{tool_name}' was AlwaysAllow'd earlier this session"),
                    );
                }
                let Some(prompter) = self.prompter else {
                    return GateDecision::allow(
                        GateName::PermissionMode,
                        "ask-mode without an interactive prompter; defaulting to allow".to_owned(),
                    );
                };
                let summary = summarize_input(input);
                match prompter.prompt(tool_name, &summary) {
                    PermissionDecision::Allow => GateDecision::allow(
                        GateName::Prompter,
                        format!("user approved {tool_name}"),
                    ),
                    PermissionDecision::AlwaysAllow => GateDecision::allow_remember(
                        GateName::Prompter,
                        format!("user chose AlwaysAllow for {tool_name}"),
                    ),
                    PermissionDecision::Deny => GateDecision::deny(
                        GateName::Prompter,
                        format!("user denied {tool_name}"),
                    ),
                }
            }
        }
    }
}

fn risk_label(risk: ToolRiskLevel) -> &'static str {
    match risk {
        ToolRiskLevel::Safe => "Safe",
        ToolRiskLevel::Write => "Write",
        ToolRiskLevel::Destructive => "Destructive",
    }
}

fn summarize_input(input: &Value) -> String {
    let raw = input.to_string();
    if raw.len() <= 200 {
        return raw;
    }
    let mut idx = 200;
    while idx > 0 && !raw.is_char_boundary(idx) {
        idx -= 1;
    }
    format!("{}...", &raw[..idx])
}

fn is_read_only_for_plan(tool_name: &str, input: &Value) -> bool {
    matches!(
        tool_name,
        "FileRead"
            | "Glob"
            | "Grep"
            | "CronList"
            | "ToolSearch"
            | "AskUserQuestion"
            | "EnterPlanMode"
            | "ExitPlanMode"
            | "Skill"
    ) || tool_name == "Memory"
        && matches!(
            input["action"].as_str().unwrap_or("list"),
            "list" | "search"
        )
        || tool_name == "Mcp"
            && matches!(
                input["action"].as_str().unwrap_or("call"),
                "list_resources" | "read_resource"
            )
        || tool_name == "Bash"
            && crate::tool::bash_validation::is_read_only_command(
                input["command"].as_str().unwrap_or(""),
            )
}

fn sandbox_violation(
    tool_name: &str,
    input: &Value,
    sandbox: &SandboxConfig,
) -> Option<String> {
    match tool_name {
        "FileRead" | "FileWrite" | "FileEdit" => {
            let path = input["file_path"].as_str().or(input["path"].as_str())?;
            if !sandbox.allowed_paths.is_empty()
                && !sandbox.allowed_paths.iter().any(|p| path.starts_with(p))
            {
                return Some(format!(
                    "path '{path}' is outside sandbox.allowed_paths"
                ));
            }
            if let Err(e) = file_safety::validate_file_path(
                path,
                sandbox.allowed_paths.first().map_or("/", String::as_str),
            ) {
                return Some(e);
            }
            None
        }
        "WebFetch" | "WebSearch" => {
            if sandbox.network_enabled {
                None
            } else {
                Some("sandbox.network_enabled=false".to_owned())
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::permission::{AutoApprovePrompter, AutoDenyPrompter};
    use serde_json::json;

    fn engine_with(mode: PermissionMode) -> PolicyEngine<'static> {
        PolicyEngine {
            mode,
            trust: None,
            sandbox: None,
            prompter: None,
            previously_allowed: false,
        }
    }

    #[test]
    fn auto_mode_allows_with_reason() {
        let engine = engine_with(PermissionMode::Auto);
        let d = engine.evaluate("FileRead", &json!({"file_path": "/tmp/x"}));
        assert!(d.is_allow());
        assert_eq!(d.gate(), GateName::PermissionMode);
        assert!(d.reason().contains("Safe"));
    }

    #[test]
    fn deny_mode_rejects_everything() {
        let engine = engine_with(PermissionMode::Deny);
        let d = engine.evaluate("FileRead", &json!({"file_path": "/tmp/x"}));
        assert!(!d.is_allow());
        assert_eq!(d.gate(), GateName::PermissionMode);
    }

    #[test]
    fn readonly_mode_allows_safe_blocks_write() {
        let engine = engine_with(PermissionMode::ReadOnly);
        assert!(
            engine
                .evaluate("FileRead", &json!({"file_path": "/tmp/x"}))
                .is_allow()
        );
        let d = engine.evaluate("FileWrite", &json!({"file_path": "/tmp/x", "content": "y"}));
        assert!(!d.is_allow());
        assert!(d.reason().contains("Write"));
    }

    #[test]
    fn ask_mode_auto_approves_safe() {
        let engine = engine_with(PermissionMode::Ask);
        assert!(engine.evaluate("FileRead", &json!({"file_path": "/tmp"})).is_allow());
    }

    #[test]
    fn ask_mode_with_deny_prompter_blocks_write() {
        let prompter = AutoDenyPrompter;
        let engine = PolicyEngine {
            mode: PermissionMode::Ask,
            trust: None,
            sandbox: None,
            prompter: Some(&prompter),
            previously_allowed: false,
        };
        let d = engine.evaluate("FileWrite", &json!({"file_path": "/tmp/x", "content": "y"}));
        assert!(!d.is_allow());
        assert_eq!(d.gate(), GateName::Prompter);
        assert!(d.reason().contains("denied"));
    }

    #[test]
    fn ask_mode_with_allow_prompter_allows_write() {
        let prompter = AutoApprovePrompter;
        let engine = PolicyEngine {
            mode: PermissionMode::Ask,
            trust: None,
            sandbox: None,
            prompter: Some(&prompter),
            previously_allowed: false,
        };
        let d = engine.evaluate("FileWrite", &json!({"file_path": "/tmp/x", "content": "y"}));
        assert!(d.is_allow());
        assert_eq!(d.gate(), GateName::Prompter);
    }

    #[test]
    fn previously_allowed_skips_prompter() {
        let prompter = AutoDenyPrompter;
        let engine = PolicyEngine {
            mode: PermissionMode::Ask,
            trust: None,
            sandbox: None,
            prompter: Some(&prompter),
            previously_allowed: true,
        };
        let d = engine.evaluate("FileWrite", &json!({"file_path": "/tmp/x", "content": "y"}));
        assert!(d.is_allow());
        // Bypassed the prompter — gate should be PermissionMode, not Prompter.
        assert_eq!(d.gate(), GateName::PermissionMode);
    }

    #[test]
    fn sandbox_blocks_disallowed_path() {
        let sandbox = SandboxConfig {
            enabled: true,
            allowed_paths: vec!["/tmp/work".to_owned()],
            network_enabled: true,
        };
        let engine = PolicyEngine {
            mode: PermissionMode::Auto,
            trust: None,
            sandbox: Some(&sandbox),
            prompter: None,
            previously_allowed: false,
        };
        let d = engine.evaluate("FileWrite", &json!({"file_path": "/etc/shadow", "content": "x"}));
        assert!(!d.is_allow());
        assert_eq!(d.gate(), GateName::Sandbox);
    }

    #[test]
    fn sandbox_blocks_network_when_disabled() {
        let sandbox = SandboxConfig {
            enabled: true,
            allowed_paths: vec![],
            network_enabled: false,
        };
        let engine = PolicyEngine {
            mode: PermissionMode::Auto,
            trust: None,
            sandbox: Some(&sandbox),
            prompter: None,
            previously_allowed: false,
        };
        let d = engine.evaluate("WebFetch", &json!({"url": "https://example.com"}));
        assert!(!d.is_allow());
        assert_eq!(d.gate(), GateName::Sandbox);
    }

    #[test]
    fn trust_deny_short_circuits_before_mode() {
        // Custom resolver that denies everything — the simplest way to verify
        // the trust gate runs before permission_mode.
        struct AlwaysDeny;
        impl crate::tool::trust::TrustResolver for AlwaysDeny {
            fn resolve(
                &self,
                _: &crate::tool::trust::TrustContext,
            ) -> crate::tool::trust::TrustDecision {
                crate::tool::trust::TrustDecision::Deny
            }
        }
        let trust = PermissionEnforcer::new(Box::new(AlwaysDeny));
        let engine = PolicyEngine {
            mode: PermissionMode::Auto,
            trust: Some(&trust),
            sandbox: None,
            prompter: None,
            previously_allowed: false,
        };
        let d = engine.evaluate("FileRead", &json!({"file_path": "/tmp"}));
        assert!(!d.is_allow());
        assert_eq!(d.gate(), GateName::Trust);
    }

    #[test]
    fn format_trail_includes_gate_and_reason() {
        let d = GateDecision::deny(GateName::Sandbox, "no path");
        let trail = d.format_trail();
        assert!(trail.contains("sandbox"));
        assert!(trail.contains("no path"));
    }
}
