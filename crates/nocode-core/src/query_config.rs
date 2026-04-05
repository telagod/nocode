use crate::message::QueryMessage;
use crate::provider::{ModelSelection, ModelStreamMode, ToolSchema};
use crate::query_loop::{QueryLoopParams, QuerySource, TaskBudget};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QueryRuntimeGates {
    pub verbose: bool,
    pub replay_user_messages: bool,
    pub include_partial_messages: bool,
    pub stream_model_responses: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryConfig {
    pub system_prompt: Vec<QueryMessage>,
    pub user_context_keys: Vec<String>,
    pub system_context_keys: Vec<String>,
    pub model_selection: ModelSelection,
    pub model_reasoning_effort: Option<String>,
    pub json_schema: Option<String>,
    pub query_source: QuerySource,
    pub max_turns: Option<u32>,
    pub task_budget: Option<TaskBudget>,
    pub runtime_gates: QueryRuntimeGates,
    pub tool_definitions: Vec<ToolSchema>,
}

impl QueryConfig {
    pub fn selected_model(&self) -> Option<&str> {
        self.model_selection.selected_model()
    }

    pub fn fallback_model(&self) -> Option<&str> {
        self.model_selection.fallback_model.as_deref()
    }

    pub fn stream_mode(&self) -> ModelStreamMode {
        if self.runtime_gates.stream_model_responses {
            ModelStreamMode::Enabled
        } else {
            ModelStreamMode::Disabled
        }
    }

    pub fn to_loop_params(&self, messages: Vec<QueryMessage>) -> QueryLoopParams {
        QueryLoopParams {
            messages,
            system_prompt: self.system_prompt.clone(),
            user_context_keys: self.user_context_keys.clone(),
            system_context_keys: self.system_context_keys.clone(),
            fallback_model: self.model_selection.fallback_model.clone(),
            query_source: self.query_source,
            max_output_tokens_override: None,
            max_turns: self.max_turns,
            task_budget: self.task_budget,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{QueryConfig, QueryRuntimeGates};
    use crate::message::QueryMessage;
    use crate::provider::{ModelProvider, ModelSelection, ModelStreamMode};
    use crate::query_loop::{QuerySource, TaskBudget};

    fn sample_config() -> QueryConfig {
        QueryConfig {
            system_prompt: vec![QueryMessage::system("system")],
            user_context_keys: vec![String::from("cwd")],
            system_context_keys: vec![String::from("model")],
            model_selection: ModelSelection {
                provider: ModelProvider::Mock,
                requested_model: Some(String::from("sonnet")),
                fallback_model: Some(String::from("haiku")),
            },
            model_reasoning_effort: None,
            json_schema: None,
            query_source: QuerySource::Sdk,
            max_turns: Some(4),
            task_budget: Some(TaskBudget { total: 10_000 }),
            runtime_gates: QueryRuntimeGates {
                verbose: true,
                replay_user_messages: false,
                include_partial_messages: false,
                stream_model_responses: true,
            },
            tool_definitions: Vec::new(),
        }
    }

    #[test]
    fn selected_model_prefers_user_choice() {
        let config = sample_config();
        assert_eq!(config.selected_model(), Some("sonnet"));
        assert_eq!(config.stream_mode(), ModelStreamMode::Enabled);
    }

    #[test]
    fn selected_model_falls_back_when_requested_missing() {
        let mut config = sample_config();
        config.model_selection.requested_model = None;

        assert_eq!(config.selected_model(), Some("haiku"));
    }

    #[test]
    fn selected_model_can_be_absent() {
        let mut config = sample_config();
        config.model_selection.requested_model = None;
        config.model_selection.fallback_model = None;

        assert_eq!(config.selected_model(), None);
    }

    #[test]
    fn loop_params_are_built_from_snapshot() {
        let params = sample_config().to_loop_params(vec![QueryMessage::user("prompt")]);
        assert_eq!(params.query_source, QuerySource::Sdk);
        assert_eq!(params.max_turns, Some(4));
        assert_eq!(params.task_budget, Some(TaskBudget { total: 10_000 }));
        assert_eq!(params.system_prompt, vec![QueryMessage::system("system")]);
        assert_eq!(params.messages, vec![QueryMessage::user("prompt")]);
    }
}
