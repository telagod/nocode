use crate::message::{ContentBlock, Message, SystemBlock};
use crate::provider::Provider;
use crate::provider::types::{
    CreateMessageRequest, ProviderError, StopReason, StreamEvent, ToolDefinition,
};
use crate::tool::executor::ToolExecutor;

/// Configuration for the agentic loop.
pub struct LoopConfig {
    pub model: String,
    pub max_tokens: u32,
    pub max_turns: u32,
    pub system: Vec<SystemBlock>,
    pub tools: Vec<ToolDefinition>,
}

/// Result of running the agentic loop.
pub struct LoopResult {
    pub messages: Vec<Message>,
    pub stop_reason: StopReason,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub turns: u32,
}

/// Callback for streaming events + tool execution notifications.
pub trait LoopObserver {
    fn on_stream_event(&mut self, _event: &StreamEvent) {}
    fn on_tool_start(&mut self, _name: &str, _id: &str) {}
    fn on_tool_done(&mut self, _name: &str, _id: &str, _result: &ContentBlock) {}
    fn on_turn_complete(&mut self, _turn: u32) {}
}

/// No-op observer for headless usage.
pub struct NoopObserver;
impl LoopObserver for NoopObserver {}

/// Run the agentic loop: model call → tool execution → repeat.
/// Driven by `stop_reason`, aligned with Claude Code's loop pattern.
pub fn run_agentic_loop(
    provider: &dyn Provider,
    executor: &ToolExecutor<'_>,
    config: &LoopConfig,
    initial_messages: Vec<Message>,
    observer: &mut dyn LoopObserver,
) -> Result<LoopResult, ProviderError> {
    let mut messages = initial_messages;
    let mut total_input_tokens: u64 = 0;
    let mut total_output_tokens: u64 = 0;
    let mut turns: u32 = 0;
    let mut final_stop_reason = StopReason::EndTurn;

    loop {
        if turns >= config.max_turns {
            break;
        }
        turns += 1;

        let request = CreateMessageRequest {
            model: config.model.clone(),
            max_tokens: config.max_tokens,
            system: config.system.clone(),
            messages: messages.clone(),
            tools: config.tools.clone(),
            stream: true,
        };

        // Stream the model response
        let response = provider.create_message_stream(&request, &mut |event| {
            observer.on_stream_event(&event);
        })?;

        total_input_tokens += response.usage.input_tokens;
        total_output_tokens += response.usage.output_tokens;
        final_stop_reason = response.stop_reason;

        match response.stop_reason {
            StopReason::EndTurn => {
                // Model finished — push final assistant message and exit
                if !response.content.is_empty() {
                    messages.push(Message::assistant(response.content));
                }
                break;
            }
            StopReason::ToolUse => {
                // 1. Push assistant message (contains tool_use blocks)
                messages.push(Message::assistant(response.content.clone()));

                // 2. Execute each tool_use, collect results
                let mut results: Vec<ContentBlock> = Vec::new();
                for block in &response.content {
                    if let ContentBlock::ToolUse { id, name, input } = block {
                        observer.on_tool_start(name, id);
                        let result = executor.execute_tool_use(id, name, input);
                        observer.on_tool_done(name, id, &result);
                        results.push(result);
                    }
                }

                // 3. Push tool results as user message
                messages.push(Message::user(results));

                observer.on_turn_complete(turns);
                // Loop back to call model again
            }
            StopReason::MaxTokens => {
                // Context limit — push what we have and exit
                if !response.content.is_empty() {
                    messages.push(Message::assistant(response.content));
                }
                break;
            }
            StopReason::PauseTurn => {
                // Server-side pause — push and continue
                if !response.content.is_empty() {
                    messages.push(Message::assistant(response.content));
                }
                // Continue loop to re-send
            }
        }
    }

    Ok(LoopResult {
        messages,
        stop_reason: final_stop_reason,
        total_input_tokens,
        total_output_tokens,
        turns,
    })
}
