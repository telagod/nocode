use crate::assistant_turn::AssistantTurn;
use crate::budget::estimate_turn_tokens;
use crate::message::QueryMessage;
use crate::tool_execution::ToolCallResult;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UsageTotals {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageSnapshot {
    pub turn_index: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub assistant_message_count: usize,
    pub tool_call_count: usize,
    pub tool_result_count: usize,
    pub total_usage: UsageTotals,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UsageTracker {
    totals: UsageTotals,
    completed_turns: u32,
}

impl UsageTracker {
    pub fn totals(&self) -> &UsageTotals {
        &self.totals
    }

    pub fn completed_turns(&self) -> u32 {
        self.completed_turns
    }

    pub fn record_turn(
        &mut self,
        request_messages: &[QueryMessage],
        system_prompt: &[QueryMessage],
        assistant_turn: &AssistantTurn,
        tool_results: &[ToolCallResult],
    ) -> UsageSnapshot {
        let input_tokens =
            estimate_turn_tokens(request_messages) + estimate_turn_tokens(system_prompt);
        let output_tokens = estimate_turn_tokens(assistant_turn.response_messages.as_slice());

        self.totals.input_tokens += input_tokens;
        self.totals.output_tokens += output_tokens;
        self.completed_turns += 1;

        UsageSnapshot {
            turn_index: self.completed_turns,
            input_tokens,
            output_tokens,
            assistant_message_count: assistant_turn.response_messages.len(),
            tool_call_count: assistant_turn.tool_uses.len(),
            tool_result_count: tool_results.len(),
            total_usage: self.totals.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{UsageTotals, UsageTracker};
    use crate::assistant_turn::{AssistantTurn, AssistantTurnStatus};
    use crate::message::QueryMessage;
    use crate::tool_execution::{ToolCallInput, ToolCallOutput, ToolCallResult};

    #[test]
    fn usage_tracker_accumulates_turn_totals() {
        let mut tracker = UsageTracker::default();
        let tool_result = ToolCallResult::Completed {
            call: ToolCallInput::new("Read", "tool-1"),
            user_modified: false,
            output: ToolCallOutput {
                summary: String::from("tool output"),
                generated_messages: Vec::new(),
                context_label: None,
                progress_updates: Vec::new(),
            },
        };
        let assistant_turn = AssistantTurn::new(
            1,
            AssistantTurnStatus::Continue,
            vec![QueryMessage::assistant("reply tokens")],
            std::slice::from_ref(&tool_result),
            3,
        );

        let snapshot = tracker.record_turn(
            &[QueryMessage::user("hello world")],
            &[QueryMessage::system("system prompt")],
            &assistant_turn,
            &[tool_result],
        );

        assert_eq!(snapshot.turn_index, 1);
        assert_eq!(snapshot.assistant_message_count, 1);
        assert_eq!(snapshot.tool_call_count, 1);
        assert_eq!(snapshot.tool_result_count, 1);
        assert_eq!(tracker.completed_turns(), 1);
        assert_eq!(
            tracker.totals(),
            &UsageTotals {
                input_tokens: 4,
                output_tokens: 2,
            }
        );
    }
}
