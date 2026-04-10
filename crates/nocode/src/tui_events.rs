//! TUI ↔ agentic loop bridge events — extracted from tui_app.rs.

use nocode_core::message::{ContentBlock, Message};
use nocode_core::provider::types::{StreamDelta, StreamEvent};
use nocode_core::query::r#loop::{LoopObserver, LoopResult};
use nocode_core::tool::ToolRegistry;
use std::sync::mpsc;

// ---------------------------------------------------------------------------
// TUI event types
// ---------------------------------------------------------------------------

pub(crate) enum TuiEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ToolStart {
        name: String,
    },
    InputJsonDelta {
        name: String,
        partial_json: String,
    },
    ToolDone {
        name: String,
        content: String,
        is_error: bool,
    },
    PermissionRequest {
        tool_name: String,
        tool_id: String,
        response_tx: mpsc::Sender<nocode_core::tool::permission::PermissionDecision>,
    },
    QuestionRequest {
        questions: serde_json::Value,
        response_tx: mpsc::Sender<Result<nocode_core::tool::permission::UserAnswer, String>>,
    },
    MessagesUpdated(Vec<Message>),
    Complete(Result<LoopResult, String>, ToolRegistry),
}

// ---------------------------------------------------------------------------
// Channel-based LoopObserver for background thread
// ---------------------------------------------------------------------------

pub(crate) struct ChannelObserver {
    pub tx: mpsc::Sender<TuiEvent>,
}

impl LoopObserver for ChannelObserver {
    fn on_stream_event(&mut self, event: &StreamEvent) {
        if let StreamEvent::ContentBlockDelta { delta, .. } = event {
            match delta {
                StreamDelta::TextDelta { text } => {
                    let _ = self.tx.send(TuiEvent::TextDelta(text.clone()));
                }
                StreamDelta::ThinkingDelta { thinking } => {
                    let _ = self.tx.send(TuiEvent::ThinkingDelta(thinking.clone()));
                }
                StreamDelta::InputJsonDelta { partial_json } => {
                    let _ = self.tx.send(TuiEvent::InputJsonDelta {
                        name: String::new(),
                        partial_json: partial_json.clone(),
                    });
                }
            }
        }
    }

    fn on_tool_start(&mut self, name: &str, _id: &str) {
        let _ = self.tx.send(TuiEvent::ToolStart {
            name: name.to_string(),
        });
    }

    fn on_tool_done(&mut self, name: &str, _id: &str, result: &ContentBlock) {
        let (content, is_error) = match result {
            ContentBlock::ToolResult {
                content, is_error, ..
            } => (content.clone(), *is_error),
            _ => (String::new(), false),
        };
        let _ = self.tx.send(TuiEvent::ToolDone {
            name: name.to_string(),
            content,
            is_error,
        });
    }

    fn on_messages_updated(&mut self, messages: &[Message]) {
        let _ = self.tx.send(TuiEvent::MessagesUpdated(messages.to_vec()));
    }
}

// ---------------------------------------------------------------------------
// Permission bridge that sends requests via TuiEvent channel
// ---------------------------------------------------------------------------

/// Permission prompter that sends requests through the TuiEvent channel
/// and blocks until the TUI thread responds via a one-shot response channel.
pub(crate) struct TuiEventPermissionBridge {
    tx: mpsc::Sender<TuiEvent>,
    timeout: std::time::Duration,
}

impl TuiEventPermissionBridge {
    pub fn new(tx: mpsc::Sender<TuiEvent>) -> Self {
        Self {
            tx,
            timeout: std::time::Duration::from_secs(60),
        }
    }
}

impl nocode_core::tool::permission::PermissionPrompter for TuiEventPermissionBridge {
    fn prompt(
        &self,
        tool_name: &str,
        _arguments_summary: &str,
    ) -> nocode_core::tool::permission::PermissionDecision {
        use nocode_core::tool::permission::PermissionDecision;

        let (response_tx, response_rx) = mpsc::channel();
        let event = TuiEvent::PermissionRequest {
            tool_name: tool_name.to_string(),
            tool_id: String::new(),
            response_tx,
        };

        if self.tx.send(event).is_err() {
            return PermissionDecision::Deny;
        }

        response_rx
            .recv_timeout(self.timeout)
            .unwrap_or(PermissionDecision::Deny)
    }
}

// ---------------------------------------------------------------------------
// Question bridge that sends AskUserQuestion requests via TuiEvent channel
// ---------------------------------------------------------------------------

/// Question prompter that sends requests through the TuiEvent channel
/// and blocks until the TUI thread responds via a one-shot response channel.
pub(crate) struct TuiEventQuestionBridge {
    tx: mpsc::Sender<TuiEvent>,
    timeout: std::time::Duration,
}

impl TuiEventQuestionBridge {
    pub fn new(tx: mpsc::Sender<TuiEvent>) -> Self {
        Self {
            tx,
            timeout: std::time::Duration::from_secs(120),
        }
    }
}

impl nocode_core::tool::permission::QuestionPrompter for TuiEventQuestionBridge {
    fn prompt_questions(
        &self,
        questions: &serde_json::Value,
    ) -> Result<nocode_core::tool::permission::UserAnswer, String> {
        let (response_tx, response_rx) = mpsc::channel();
        let event = TuiEvent::QuestionRequest {
            questions: questions.clone(),
            response_tx,
        };

        if self.tx.send(event).is_err() {
            return Err("TUI channel closed".to_string());
        }

        response_rx
            .recv_timeout(self.timeout)
            .map_err(|_| "Question timed out".to_string())?
    }
}
