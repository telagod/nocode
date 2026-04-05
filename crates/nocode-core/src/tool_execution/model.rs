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
    Allow { user_modified: bool },
    Deny { reason: String },
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

    pub fn settle(self, call: ToolCallInput, output: ToolCallOutput) -> ToolCallResult {
        match self {
            Self::Allow { user_modified } => ToolCallResult::Completed {
                call,
                user_modified,
                output,
            },
            Self::Deny { reason } => ToolCallResult::Denied { call, reason },
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
        ToolCallInput, ToolCallOutput, ToolCallResult, ToolPermissionDecision, ToolProgressUpdate,
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
}
