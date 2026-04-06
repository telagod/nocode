use crate::message::QueryMessage;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallArgument {
    pub key: String,
    pub value: String,
}

impl ToolCallArgument {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallInput {
    pub tool_name: String,
    pub tool_use_id: String,
    pub arguments: Vec<ToolCallArgument>,
    pub context_label: String,
}

impl ToolCallInput {
    pub fn new(tool_name: impl Into<String>, tool_use_id: impl Into<String>) -> Self {
        Self {
            tool_name: tool_name.into(),
            tool_use_id: tool_use_id.into(),
            arguments: Vec::new(),
            context_label: String::from("default"),
        }
    }

    pub fn with_argument(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.arguments.push(ToolCallArgument::new(key, value));
        self
    }

    pub fn with_context_label(mut self, context_label: impl Into<String>) -> Self {
        self.context_label = context_label.into();
        self
    }

    pub fn with_arguments_map(mut self, map: &std::collections::HashMap<String, String>) -> Self {
        for (key, value) in map {
            self.arguments
                .push(ToolCallArgument::new(key.clone(), value.clone()));
        }
        self
    }

    pub fn argument(&self, key: &str) -> Option<&str> {
        self.arguments
            .iter()
            .find(|argument| argument.key == key)
            .map(|argument| argument.value.as_str())
    }

    pub fn summary(&self) -> String {
        if self.arguments.is_empty() {
            return format!("{}#{}", self.tool_name, self.tool_use_id);
        }

        let args = self
            .arguments
            .iter()
            .map(|argument| format!("{}={}", argument.key, argument.value))
            .collect::<Vec<_>>()
            .join(", ");
        format!("{}#{}({args})", self.tool_name, self.tool_use_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolPermissionDecision {
    Allow {
        user_modified: bool,
    },
    Deny {
        reason: String,
    },
    /// Requires interactive user approval before proceeding.
    Prompt {
        tool_name: String,
        reason: String,
    },
}

impl ToolPermissionDecision {
    pub fn allow(user_modified: bool) -> Self {
        Self::Allow { user_modified }
    }

    pub fn deny(reason: impl Into<String>) -> Self {
        Self::Deny {
            reason: reason.into(),
        }
    }

    pub fn prompt(tool_name: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Prompt {
            tool_name: tool_name.into(),
            reason: reason.into(),
        }
    }

    pub fn settle(self, call: ToolCallInput, output: ToolCallOutput) -> ToolCallResult {
        match self {
            Self::Allow { user_modified } => ToolCallResult::Completed {
                call,
                user_modified,
                output,
            },
            Self::Deny { reason } => ToolCallResult::Denied { call, reason },
            Self::Prompt { reason, .. } => ToolCallResult::Denied {
                call,
                reason: format!("awaiting approval: {reason}"),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolProgressUpdate {
    pub tool_use_id: String,
    pub message: String,
}

impl ToolProgressUpdate {
    pub fn new(tool_use_id: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            tool_use_id: tool_use_id.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToolCallOutput {
    pub summary: String,
    pub generated_messages: Vec<QueryMessage>,
    pub context_label: Option<String>,
    pub progress_updates: Vec<ToolProgressUpdate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolExecutionRequest {
    pub call: ToolCallInput,
    pub can_execute: bool,
    pub deny_reason: Option<String>,
}

impl ToolExecutionRequest {
    pub fn allowed(call: ToolCallInput) -> Self {
        Self {
            call,
            can_execute: true,
            deny_reason: None,
        }
    }

    pub fn denied(call: ToolCallInput, reason: impl Into<String>) -> Self {
        Self {
            call,
            can_execute: false,
            deny_reason: Some(reason.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCallResult {
    Completed {
        call: ToolCallInput,
        user_modified: bool,
        output: ToolCallOutput,
    },
    Denied {
        call: ToolCallInput,
        reason: String,
    },
    Failed {
        call: ToolCallInput,
        error: String,
    },
}

impl ToolCallResult {
    pub fn failed(call: ToolCallInput, error: impl Into<String>) -> Self {
        Self::Failed {
            call,
            error: error.into(),
        }
    }

    pub fn call(&self) -> &ToolCallInput {
        match self {
            Self::Completed { call, .. }
            | Self::Denied { call, .. }
            | Self::Failed { call, .. } => call,
        }
    }

    pub fn status_label(&self) -> &'static str {
        match self {
            Self::Completed { .. } => "completed",
            Self::Denied { .. } => "denied",
            Self::Failed { .. } => "failed",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::Completed {
                call,
                user_modified,
                output,
            } => {
                let suffix = if *user_modified {
                    " (user-modified)"
                } else {
                    ""
                };
                format!(
                    "tool-result:{} -> {}{}",
                    call.summary(),
                    output.summary,
                    suffix
                )
            }
            Self::Denied { call, reason } => {
                format!("tool-denied:{} -> {}", call.summary(), reason)
            }
            Self::Failed { call, error } => {
                format!("tool-failed:{} -> {}", call.summary(), error)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolExecutionTrace {
    pub progress_updates: Vec<ToolProgressUpdate>,
    pub result: ToolCallResult,
    pub permission_denial: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{
        ToolCallInput, ToolCallOutput, ToolCallResult, ToolExecutionRequest,
        ToolPermissionDecision, ToolProgressUpdate,
    };
    use crate::message::QueryMessage;

    #[test]
    fn tool_call_summary_includes_arguments() {
        let call = ToolCallInput::new("Read", "toolu-1")
            .with_argument("file_path", "src/query.ts")
            .with_context_label("sdk-bootstrap");
        assert_eq!(call.summary(), "Read#toolu-1(file_path=src/query.ts)");
        assert_eq!(call.context_label, "sdk-bootstrap");
        assert_eq!(call.argument("file_path"), Some("src/query.ts"));
    }

    #[test]
    fn permission_decision_builds_completed_result() {
        let call = ToolCallInput::new("Read", "toolu-2");
        let result = ToolPermissionDecision::allow(false).settle(
            call.clone(),
            ToolCallOutput {
                summary: String::from("loaded"),
                generated_messages: vec![QueryMessage::assistant("tool message")],
                context_label: Some(String::from("sdk-bootstrap")),
                progress_updates: vec![ToolProgressUpdate::new("toolu-2", "done")],
            },
        );

        assert_eq!(result.status_label(), "completed");
        assert_eq!(result.call(), &call);
        assert!(result.message().contains("loaded"));
    }

    #[test]
    fn failed_result_renders_error_message() {
        let call = ToolCallInput::new("Bash", "toolu-3");
        let result = ToolCallResult::failed(call, "command exited 1");
        assert_eq!(result.status_label(), "failed");
        assert!(result.message().contains("command exited 1"));
    }

    #[test]
    fn permission_prompt_settles_to_denied() {
        let call = ToolCallInput::new("Bash", "toolu-p1");
        let decision = ToolPermissionDecision::prompt("Bash", "requires approval");
        let result = decision.settle(call, ToolCallOutput::default());
        assert_eq!(result.status_label(), "denied");
        assert!(result.message().contains("awaiting approval"));
    }

    #[test]
    fn permission_deny_settles_to_denied() {
        let call = ToolCallInput::new("Write", "toolu-p2");
        let decision = ToolPermissionDecision::deny("not allowed");
        let result = decision.settle(call, ToolCallOutput::default());
        assert_eq!(result.status_label(), "denied");
        assert!(result.message().contains("not allowed"));
    }

    #[test]
    fn permission_allow_user_modified() {
        let call = ToolCallInput::new("Edit", "toolu-p3");
        let decision = ToolPermissionDecision::allow(true);
        let result = decision.settle(
            call,
            ToolCallOutput {
                summary: String::from("edited"),
                generated_messages: Vec::new(),
                context_label: None,
                progress_updates: Vec::new(),
            },
        );
        assert_eq!(result.status_label(), "completed");
        assert!(result.message().contains("user-modified"));
    }

    #[test]
    fn tool_call_no_arguments_summary() {
        let call = ToolCallInput::new("Read", "toolu-s1");
        assert_eq!(call.summary(), "Read#toolu-s1");
    }

    #[test]
    fn tool_call_argument_not_found() {
        let call = ToolCallInput::new("Read", "toolu-s2").with_argument("file_path", "foo.rs");
        assert!(call.argument("nonexistent").is_none());
    }

    #[test]
    fn tool_call_with_arguments_map() {
        let mut map = std::collections::HashMap::new();
        map.insert("key1".to_string(), "val1".to_string());
        map.insert("key2".to_string(), "val2".to_string());
        let call = ToolCallInput::new("Bash", "toolu-m1").with_arguments_map(&map);
        assert_eq!(call.arguments.len(), 2);
        assert!(call.argument("key1").is_some());
        assert!(call.argument("key2").is_some());
    }

    #[test]
    fn tool_execution_request_allowed_and_denied() {
        let call = ToolCallInput::new("Read", "toolu-r1");
        let req = ToolExecutionRequest::allowed(call.clone());
        assert!(req.can_execute);
        assert!(req.deny_reason.is_none());

        let req = ToolExecutionRequest::denied(call, "blocked");
        assert!(!req.can_execute);
        assert_eq!(req.deny_reason.as_deref(), Some("blocked"));
    }

    #[test]
    fn tool_call_result_call_accessor() {
        let call = ToolCallInput::new("Glob", "toolu-c1");
        let result = ToolCallResult::Completed {
            call: call.clone(),
            user_modified: false,
            output: ToolCallOutput::default(),
        };
        assert_eq!(result.call().tool_name, "Glob");

        let denied = ToolCallResult::Denied {
            call: call.clone(),
            reason: "no".into(),
        };
        assert_eq!(denied.call().tool_use_id, "toolu-c1");
    }

    #[test]
    fn tool_progress_update_fields() {
        let u = ToolProgressUpdate::new("id-1", "doing stuff");
        assert_eq!(u.tool_use_id, "id-1");
        assert_eq!(u.message, "doing stuff");
    }

    #[test]
    fn tool_call_output_default() {
        let o = ToolCallOutput::default();
        assert!(o.summary.is_empty());
        assert!(o.generated_messages.is_empty());
        assert!(o.context_label.is_none());
        assert!(o.progress_updates.is_empty());
    }
}
