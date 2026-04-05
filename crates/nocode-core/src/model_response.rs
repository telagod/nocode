use crate::assistant_turn::{AssistantToolUse, AssistantTurn, AssistantTurnStatus};
use crate::message::{QueryMessage, QueryMessageRole};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelResponseStopReason {
    ToolBatchFlushed,
    Completed,
    MaxTurns,
    Terminal,
}

impl ModelResponseStopReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ToolBatchFlushed => "tool_batch_flushed",
            Self::Completed => "completed",
            Self::MaxTurns => "max_turns",
            Self::Terminal => "terminal",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelResponseToolPhase {
    pub requested_tools: usize,
    pub resolved_tools: usize,
    pub tool_uses: Vec<AssistantToolUse>,
}

impl ModelResponseToolPhase {
    pub fn new(requested_tools: usize, tool_uses: Vec<AssistantToolUse>) -> Self {
        Self {
            requested_tools,
            resolved_tools: tool_uses.len(),
            tool_uses,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelResponse {
    pub response_id: String,
    pub status: AssistantTurnStatus,
    pub stop_reason: ModelResponseStopReason,
    pub tool_phase: ModelResponseToolPhase,
    pub final_assistant_message: Option<QueryMessage>,
    pub assistant_turn: AssistantTurn,
}

impl ModelResponse {
    pub fn new(
        response_id: impl Into<String>,
        status: AssistantTurnStatus,
        stop_reason: ModelResponseStopReason,
        requested_tools: usize,
        assistant_turn: AssistantTurn,
    ) -> Self {
        let final_assistant_message = assistant_turn
            .response_messages
            .iter()
            .rev()
            .find(|message| message.role == QueryMessageRole::Assistant)
            .cloned();
        let tool_phase =
            ModelResponseToolPhase::new(requested_tools, assistant_turn.tool_uses.clone());

        Self {
            response_id: response_id.into(),
            status,
            stop_reason,
            tool_phase,
            final_assistant_message,
            assistant_turn,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ModelResponse, ModelResponseStopReason};
    use crate::assistant_turn::{AssistantToolUse, AssistantTurn, AssistantTurnStatus};
    use crate::message::QueryMessage;

    #[test]
    fn model_response_extracts_last_assistant_message() {
        let response = ModelResponse::new(
            "resp-1",
            AssistantTurnStatus::Continue,
            ModelResponseStopReason::ToolBatchFlushed,
            2,
            AssistantTurn {
                sequence: 1,
                status: AssistantTurnStatus::Continue,
                response_messages: vec![
                    QueryMessage::tool("tool-result"),
                    QueryMessage::assistant("assistant follow-up"),
                ],
                tool_uses: vec![AssistantToolUse {
                    tool_name: String::from("Read"),
                    tool_use_id: String::from("toolu-1"),
                    status: String::from("completed"),
                }],
                transcript_entries: 4,
            },
        );

        assert_eq!(response.response_id, "resp-1");
        assert_eq!(
            response.stop_reason,
            ModelResponseStopReason::ToolBatchFlushed
        );
        assert_eq!(response.tool_phase.requested_tools, 2);
        assert_eq!(response.tool_phase.resolved_tools, 1);
        assert_eq!(
            response.final_assistant_message,
            Some(QueryMessage::assistant("assistant follow-up"))
        );
    }

    #[test]
    fn no_assistant_message_yields_none() {
        let response = ModelResponse::new(
            "resp-2",
            AssistantTurnStatus::Completed,
            ModelResponseStopReason::Completed,
            0,
            AssistantTurn {
                sequence: 1,
                status: AssistantTurnStatus::Completed,
                response_messages: vec![QueryMessage::tool("only tool output")],
                tool_uses: Vec::new(),
                transcript_entries: 1,
            },
        );

        assert!(response.final_assistant_message.is_none());
        assert_eq!(response.tool_phase.requested_tools, 0);
        assert_eq!(response.tool_phase.resolved_tools, 0);
    }

    #[test]
    fn stop_reason_labels_are_stable() {
        assert_eq!(ModelResponseStopReason::ToolBatchFlushed.as_str(), "tool_batch_flushed");
        assert_eq!(ModelResponseStopReason::Completed.as_str(), "completed");
        assert_eq!(ModelResponseStopReason::MaxTurns.as_str(), "max_turns");
        assert_eq!(ModelResponseStopReason::Terminal.as_str(), "terminal");
    }

    #[test]
    fn tool_phase_new_sets_resolved_from_vec_len() {
        use super::ModelResponseToolPhase;
        let phase = ModelResponseToolPhase::new(5, vec![
            AssistantToolUse {
                tool_name: String::from("Read"),
                tool_use_id: String::from("t1"),
                status: String::from("completed"),
            },
            AssistantToolUse {
                tool_name: String::from("Write"),
                tool_use_id: String::from("t2"),
                status: String::from("completed"),
            },
        ]);
        assert_eq!(phase.requested_tools, 5);
        assert_eq!(phase.resolved_tools, 2);
        assert_eq!(phase.tool_uses.len(), 2);
    }
}
