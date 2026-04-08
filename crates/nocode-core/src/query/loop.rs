use crate::message::{ContentBlock, Message, SystemBlock};
use crate::provider::Provider;
use crate::provider::types::{
    CreateMessageRequest, ProviderError, StopReason, StreamDelta, StreamEvent, ToolDefinition,
};
use crate::query::budget::TokenBudget;
use crate::query::events::ModelStreamEvent;
use crate::recovery::{self, RecoveryAction, RecoveryRecipe};
use crate::session::compaction::{Compactor, TailCompactor};
use crate::tool::executor::ToolExecutor;
use std::sync::mpsc;

/// Configuration for the agentic loop.
pub struct LoopConfig {
    pub model: String,
    pub max_tokens: u32,
    pub max_turns: u32,
    pub system: Vec<SystemBlock>,
    pub tools: Vec<ToolDefinition>,
    /// Enable parallel tool execution (default: true).
    pub parallel_tool_execution: bool,
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
pub trait LoopObserver: Send {
    fn on_stream_event(&mut self, _event: &StreamEvent) {}
    fn on_model_event(&mut self, _event: &ModelStreamEvent) {}
    fn on_tool_start(&mut self, _name: &str, _id: &str) {}
    fn on_tool_done(&mut self, _name: &str, _id: &str, _result: &ContentBlock) {}
    fn on_tool_result(&mut self, _name: &str, _block: &ContentBlock) {}
    fn on_turn_complete(&mut self, _turn: u32) {}
    /// Called after messages are updated (assistant response + tool results pushed).
    /// Use for incremental session persistence.
    fn on_messages_updated(&mut self, _messages: &[Message]) {}
}

/// No-op observer for headless usage.
pub struct NoopObserver;
impl LoopObserver for NoopObserver {}

fn emit_tool_result_event(
    observer: &mut dyn LoopObserver,
    id: &str,
    name: &str,
    result: &ContentBlock,
) {
    observer.on_tool_done(name, id, result);
    observer.on_tool_result(name, result);
    observer.on_model_event(&ModelStreamEvent::ToolResult {
        tool_use_id: id.to_string(),
        name: name.to_string(),
        content: if let ContentBlock::ToolResult { content, .. } = result {
            content.clone()
        } else {
            String::new()
        },
        is_error: if let ContentBlock::ToolResult { is_error, .. } = result {
            *is_error
        } else {
            false
        },
    });
}

/// Execute tool calls in parallel, returning results in order.
fn execute_tools_parallel(
    executor: &ToolExecutor<'_>,
    tool_calls: &[(String, String, serde_json::Value)],
    observer: &mut dyn LoopObserver,
) -> Vec<ContentBlock> {
    if tool_calls.len() <= 1 {
        return tool_calls
            .iter()
            .map(|(id, name, input)| {
                observer.on_tool_start(name, id);
                let result = executor.execute_tool_use(id, name, input);
                emit_tool_result_event(observer, id, name, &result);
                result
            })
            .collect();
    }

    // Notify all tool starts before parallel execution
    for (id, name, _) in tool_calls {
        observer.on_tool_start(name, id);
    }

    // True parallel execution via scoped threads
    let (tx, rx) = mpsc::channel();

    std::thread::scope(|s| {
        for (idx, (id, name, input)) in tool_calls.iter().enumerate() {
            let tx = tx.clone();
            let id = id.clone();
            let name = name.clone();
            let input = input.clone();

            s.spawn(move || {
                let result = executor.execute_tool_use(&id, &name, &input);
                let _ = tx.send((idx, name, id, result));
            });
        }
    });
    drop(tx);

    // Collect results and sort by original index
    let mut indexed_results: Vec<(usize, String, String, ContentBlock)> = rx.into_iter().collect();
    indexed_results.sort_by_key(|(idx, _, _, _)| *idx);

    indexed_results
        .into_iter()
        .map(|(_, name, id, result)| {
            emit_tool_result_event(observer, &id, &name, &result);
            result
        })
        .collect()
}

/// Run the agentic loop: model call → tool execution → repeat.
/// Driven by `stop_reason`, aligned with Claude Code's loop pattern.
pub fn run_agentic_loop(
    provider: &dyn Provider,
    executor: &ToolExecutor<'_>,
    config: &LoopConfig,
    initial_messages: Vec<Message>,
    observer: &mut dyn LoopObserver,
) -> Result<LoopResult, ProviderError> {
    run_agentic_loop_with_budget(
        provider,
        executor,
        config,
        initial_messages,
        observer,
        &mut TokenBudget::default(),
    )
}

/// Run the agentic loop with explicit budget tracking.
pub fn run_agentic_loop_with_budget(
    provider: &dyn Provider,
    executor: &ToolExecutor<'_>,
    config: &LoopConfig,
    initial_messages: Vec<Message>,
    observer: &mut dyn LoopObserver,
    budget: &mut TokenBudget,
) -> Result<LoopResult, ProviderError> {
    let mut messages = initial_messages;
    let mut total_input_tokens: u64 = 0;
    let mut total_output_tokens: u64 = 0;
    let mut turns: u32 = 0;
    let mut final_stop_reason = StopReason::EndTurn;
    let mut compaction_count: u32 = 0;
    const MAX_COMPACTIONS: u32 = 3;

    loop {
        if turns >= config.max_turns {
            break;
        }
        if budget.is_exhausted() {
            // Try compaction before giving up
            if compaction_count < MAX_COMPACTIONS {
                let compactor = TailCompactor::new(10);
                let result = compactor.compact(&messages);
                if result.compacted_count > 0 {
                    messages = result.messages;
                    compaction_count += 1;
                    observer.on_model_event(&ModelStreamEvent::StreamError {
                        message: format!(
                            "Budget pressure — compacted {} messages, ~{} tokens saved",
                            result.compacted_count, result.tokens_saved
                        ),
                        retryable: true,
                    });
                    continue;
                }
            }
            observer.on_model_event(&ModelStreamEvent::StreamError {
                message: "Token budget exhausted".to_string(),
                retryable: false,
            });
            break;
        }
        turns += 1;

        let effective_max = budget
            .effective_max_tokens()
            .min(u64::from(config.max_tokens)) as u32;

        let request = CreateMessageRequest {
            model: config.model.clone(),
            max_tokens: effective_max,
            system: config.system.clone(),
            messages: messages.clone(),
            tools: config.tools.clone(),
            stream: true,
        };

        // Stream the model response, forwarding events — with recovery
        let stream_result = provider.create_message_stream(&request, &mut |event| {
            observer.on_stream_event(&event);
            match &event {
                StreamEvent::ContentBlockStart {
                    content_block: ContentBlock::ToolUse { id, name, .. },
                    ..
                } => {
                    observer.on_model_event(&ModelStreamEvent::ToolUseStart {
                        id: id.clone(),
                        name: name.clone(),
                    });
                }
                StreamEvent::ContentBlockDelta { delta, .. } => match delta {
                    StreamDelta::TextDelta { text } => {
                        observer
                            .on_model_event(&ModelStreamEvent::TextDelta { text: text.clone() });
                    }
                    StreamDelta::ThinkingDelta { thinking } => {
                        observer.on_model_event(&ModelStreamEvent::ThinkingDelta {
                            thinking: thinking.clone(),
                        });
                    }
                    StreamDelta::InputJsonDelta { .. } => {
                        // Forwarded via on_stream_event above — TUI handles directly
                    }
                },
                StreamEvent::MessageDelta { usage, .. } => {
                    observer.on_model_event(&ModelStreamEvent::UsageUpdate {
                        usage: usage.clone(),
                    });
                }
                _ => {}
            }
        });

        let response = match stream_result {
            Ok(r) => r,
            Err(e) => {
                let scenario = recovery::classify_error(e.status_code, &e.message);
                let recipe = RecoveryRecipe::for_scenario(scenario);

                let mut recovered = false;
                for action in &recipe.actions {
                    match action {
                        RecoveryAction::Retry { delay_ms } => {
                            observer.on_model_event(&ModelStreamEvent::StreamError {
                                message: format!("Retrying after {delay_ms}ms: {}", e.message),
                                retryable: true,
                            });
                            std::thread::sleep(std::time::Duration::from_millis(*delay_ms));
                            recovered = true;
                            break;
                        }
                        RecoveryAction::RetryWithBackoff { base_ms, .. } => {
                            let delay = base_ms * 2u64.pow(turns.min(4));
                            observer.on_model_event(&ModelStreamEvent::StreamError {
                                message: format!("Backoff {delay}ms: {}", e.message),
                                retryable: true,
                            });
                            std::thread::sleep(std::time::Duration::from_millis(delay));
                            recovered = true;
                            break;
                        }
                        RecoveryAction::CompactAndRetry => {
                            let compactor = TailCompactor::new(10);
                            let result = compactor.compact(&messages);
                            if result.compacted_count > 0 {
                                messages = result.messages;
                                observer.on_model_event(&ModelStreamEvent::StreamError {
                                    message: format!(
                                        "Compacted {} messages, retrying",
                                        result.compacted_count
                                    ),
                                    retryable: true,
                                });
                                recovered = true;
                                break;
                            }
                        }
                        RecoveryAction::Abort { reason } => {
                            return Err(ProviderError::non_retryable(format!(
                                "{reason}: {}",
                                e.message
                            )));
                        }
                        _ => {}
                    }
                }

                if recovered {
                    continue; // retry the loop iteration
                }
                return Err(e);
            }
        };

        total_input_tokens += response.usage.input_tokens;
        total_output_tokens += response.usage.output_tokens;
        budget.record(response.usage.input_tokens, response.usage.output_tokens);
        final_stop_reason = response.stop_reason;

        match response.stop_reason {
            StopReason::EndTurn => {
                if !response.content.is_empty() {
                    messages.push(Message::assistant(response.content));
                }
                observer.on_messages_updated(&messages);
                break;
            }
            StopReason::ToolUse => {
                // 1. Push assistant message (contains tool_use blocks)
                messages.push(Message::assistant(response.content.clone()));

                // 2. Extract tool calls
                let tool_calls: Vec<(String, String, serde_json::Value)> = response
                    .content
                    .iter()
                    .filter_map(|block| {
                        if let ContentBlock::ToolUse { id, name, input } = block {
                            Some((id.clone(), name.clone(), input.clone()))
                        } else {
                            None
                        }
                    })
                    .collect();

                // 3. Execute tools (parallel or sequential)
                let results = if config.parallel_tool_execution {
                    execute_tools_parallel(executor, &tool_calls, observer)
                } else {
                    tool_calls
                        .iter()
                        .map(|(id, name, input)| {
                            observer.on_tool_start(name, id);
                            let result = executor.execute_tool_use(id, name, input);
                            observer.on_tool_done(name, id, &result);
                            observer.on_tool_result(name, &result);
                            result
                        })
                        .collect()
                };

                // 4. Push tool results as user message
                messages.push(Message::user(results));

                observer.on_messages_updated(&messages);
                observer.on_turn_complete(turns);
                observer.on_model_event(&ModelStreamEvent::TurnComplete { turn: turns });
            }
            StopReason::MaxTokens => {
                if !response.content.is_empty() {
                    messages.push(Message::assistant(response.content));
                }
                // Try compaction to free context space and continue
                if compaction_count < MAX_COMPACTIONS {
                    let compactor = TailCompactor::new(10);
                    let result = compactor.compact(&messages);
                    if result.compacted_count > 0 {
                        messages = result.messages;
                        compaction_count += 1;
                        observer.on_model_event(&ModelStreamEvent::StreamError {
                            message: format!(
                                "MaxTokens — compacted {} messages, ~{} tokens saved, continuing",
                                result.compacted_count, result.tokens_saved
                            ),
                            retryable: true,
                        });
                        continue;
                    }
                }
                break;
            }
            StopReason::PauseTurn => {
                if !response.content.is_empty() {
                    messages.push(Message::assistant(response.content));
                }
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
