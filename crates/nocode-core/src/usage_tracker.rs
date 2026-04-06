use crate::assistant_turn::AssistantTurn;
use crate::budget::estimate_turn_tokens;
use crate::message::QueryMessage;
use crate::model_pricing::{CostEstimate, estimate_cost};
use crate::tool_execution::ToolCallResult;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UsageTotals {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UsageSnapshot {
    pub turn_index: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub assistant_message_count: usize,
    pub tool_call_count: usize,
    pub tool_result_count: usize,
    pub total_usage: UsageTotals,
    pub turn_cost: CostEstimate,
    pub cumulative_cost: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsageTracker {
    totals: UsageTotals,
    completed_turns: u32,
    model_name: Option<String>,
    cumulative_cost: f64,
}

impl UsageTracker {
    pub fn totals(&self) -> &UsageTotals {
        &self.totals
    }

    pub fn completed_turns(&self) -> u32 {
        self.completed_turns
    }

    pub fn cumulative_cost(&self) -> f64 {
        self.cumulative_cost
    }

    pub fn set_model(&mut self, model: &str) {
        self.model_name = Some(model.to_string());
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

        let turn_cost = match &self.model_name {
            Some(model) => estimate_cost(model, input_tokens, output_tokens, 0, 0),
            None => CostEstimate::default(),
        };
        self.cumulative_cost += turn_cost.total;

        UsageSnapshot {
            turn_index: self.completed_turns,
            input_tokens,
            output_tokens,
            assistant_message_count: assistant_turn.response_messages.len(),
            tool_call_count: assistant_turn.tool_uses.len(),
            tool_result_count: tool_results.len(),
            total_usage: self.totals.clone(),
            turn_cost,
            cumulative_cost: self.cumulative_cost,
        }
    }

    /// Format cumulative cost for display.
    pub fn format_cost(&self) -> String {
        crate::model_pricing::format_usd(self.cumulative_cost)
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

    #[test]
    fn default_tracker_starts_at_zero() {
        let tracker = UsageTracker::default();
        assert_eq!(tracker.completed_turns(), 0);
        assert_eq!(
            tracker.totals(),
            &UsageTotals {
                input_tokens: 0,
                output_tokens: 0,
            }
        );
    }

    #[test]
    fn multiple_turns_accumulate_totals() {
        let mut tracker = UsageTracker::default();

        // Turn 1
        let turn1 = AssistantTurn::new(
            1,
            AssistantTurnStatus::Continue,
            vec![QueryMessage::assistant("first reply")],
            &[],
            1,
        );
        let snap1 = tracker.record_turn(
            &[QueryMessage::user("hello world")],
            &[QueryMessage::system("sys")],
            &turn1,
            &[],
        );
        assert_eq!(snap1.turn_index, 1);

        // Turn 2
        let turn2 = AssistantTurn::new(
            2,
            AssistantTurnStatus::Completed,
            vec![QueryMessage::assistant("second reply here")],
            &[],
            1,
        );
        let snap2 = tracker.record_turn(
            &[QueryMessage::user("more input tokens here")],
            &[QueryMessage::system("sys")],
            &turn2,
            &[],
        );
        assert_eq!(snap2.turn_index, 2);
        assert_eq!(tracker.completed_turns(), 2);

        // Totals should be sum of both turns
        let totals = tracker.totals();
        assert!(totals.input_tokens > snap1.input_tokens);
        assert!(totals.output_tokens > snap1.output_tokens);
        assert_eq!(totals.input_tokens, snap1.input_tokens + snap2.input_tokens);
        assert_eq!(
            totals.output_tokens,
            snap1.output_tokens + snap2.output_tokens
        );
    }

    #[test]
    fn snapshot_total_usage_matches_tracker_totals() {
        let mut tracker = UsageTracker::default();
        let turn = AssistantTurn::new(
            1,
            AssistantTurnStatus::Completed,
            vec![QueryMessage::assistant("done")],
            &[],
            1,
        );
        let snapshot = tracker.record_turn(&[QueryMessage::user("query")], &[], &turn, &[]);
        assert_eq!(&snapshot.total_usage, tracker.totals());
    }

    #[test]
    fn cost_tracking_with_model() {
        let mut tracker = UsageTracker::default();
        tracker.set_model("claude-sonnet-4");
        let turn = AssistantTurn::new(
            1,
            AssistantTurnStatus::Completed,
            vec![QueryMessage::assistant("response text here")],
            &[],
            1,
        );
        let snapshot = tracker.record_turn(
            &[QueryMessage::user("hello world")],
            &[QueryMessage::system("system")],
            &turn,
            &[],
        );
        assert!(snapshot.turn_cost.total > 0.0);
        assert!(snapshot.cumulative_cost > 0.0);
        assert!((tracker.cumulative_cost() - snapshot.cumulative_cost).abs() < f64::EPSILON);
    }

    #[test]
    fn cost_tracking_without_model() {
        let mut tracker = UsageTracker::default();
        let turn = AssistantTurn::new(
            1,
            AssistantTurnStatus::Completed,
            vec![QueryMessage::assistant("reply")],
            &[],
            1,
        );
        let snapshot = tracker.record_turn(&[QueryMessage::user("hi")], &[], &turn, &[]);
        assert!((snapshot.turn_cost.total).abs() < f64::EPSILON);
        assert!((snapshot.cumulative_cost).abs() < f64::EPSILON);
    }

    #[test]
    fn cumulative_cost_accumulates() {
        let mut tracker = UsageTracker::default();
        tracker.set_model("opus");
        for i in 0..3 {
            let turn = AssistantTurn::new(
                i + 1,
                AssistantTurnStatus::Continue,
                vec![QueryMessage::assistant("reply")],
                &[],
                1,
            );
            tracker.record_turn(&[QueryMessage::user("input")], &[], &turn, &[]);
        }
        assert!(tracker.cumulative_cost() > 0.0);
        assert_eq!(tracker.completed_turns(), 3);
    }

    #[test]
    fn format_cost_display() {
        let mut tracker = UsageTracker::default();
        assert_eq!(tracker.format_cost(), "$0.00");
        tracker.set_model("haiku");
        let turn = AssistantTurn::new(
            1,
            AssistantTurnStatus::Completed,
            vec![QueryMessage::assistant("r")],
            &[],
            1,
        );
        tracker.record_turn(&[QueryMessage::user("q")], &[], &turn, &[]);
        assert!(tracker.format_cost().starts_with('$'));
    }
}
