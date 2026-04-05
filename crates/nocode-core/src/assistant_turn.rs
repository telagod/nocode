use crate::message::QueryMessage;
use crate::tool_execution::ToolCallResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssistantTurnStatus {
    Continue,
    Completed,
    Terminal,
}

impl AssistantTurnStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Continue => "continue",
            Self::Completed => "completed",
            Self::Terminal => "terminal",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantToolUse {
    pub tool_name: String,
    pub tool_use_id: String,
    pub status: String,
}

impl AssistantToolUse {
    pub fn from_result(result: &ToolCallResult) -> Self {
        Self {
            tool_name: result.call().tool_name.clone(),
            tool_use_id: result.call().tool_use_id.clone(),
            status: result.status_label().to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantTurn {
    pub sequence: u32,
    pub status: AssistantTurnStatus,
    pub response_messages: Vec<QueryMessage>,
    pub tool_uses: Vec<AssistantToolUse>,
    pub transcript_entries: usize,
}

impl AssistantTurn {
    pub fn new(
        sequence: u32,
        status: AssistantTurnStatus,
        response_messages: Vec<QueryMessage>,
        tool_results: &[ToolCallResult],
        transcript_entries: usize,
    ) -> Self {
        Self {
            sequence,
            status,
            response_messages,
            tool_uses: tool_results
                .iter()
                .map(AssistantToolUse::from_result)
                .collect(),
            transcript_entries,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AssistantTurn, AssistantTurnStatus};
    use crate::message::QueryMessage;
    use crate::tool_execution::ToolCallInput;
    use crate::tool_execution::ToolCallResult;

    #[test]
    fn assistant_turn_collects_tool_summaries() {
        let result = ToolCallResult::failed(ToolCallInput::new("Read", "toolu-1"), "boom");
        let turn = AssistantTurn::new(
            1,
            AssistantTurnStatus::Terminal,
            vec![QueryMessage::tool("tool-failed")],
            &[result],
            3,
        );

        assert_eq!(turn.sequence, 1);
        assert_eq!(turn.status, AssistantTurnStatus::Terminal);
        assert_eq!(turn.tool_uses.len(), 1);
        assert_eq!(turn.tool_uses[0].tool_name, "Read");
        assert_eq!(turn.tool_uses[0].status, "failed");
    }
}
