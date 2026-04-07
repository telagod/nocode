use super::{QueryEngine, QueryPlanStep, QuerySubmissionPlan, SubmitMessageOptions};
use crate::assistant_turn::{AssistantTurn, AssistantTurnStatus};
use crate::budget::{TokenBudgetDecision, estimate_turn_tokens};
use crate::history_store::HistoryEntry;
use crate::model_response::{ModelResponse, ModelResponseStopReason};
use crate::persistence_backend::{NoopPersistenceBackend, PersistenceBackend};
use crate::provider::{
    ModelError, ModelInvocation, ModelRequest, ModelStreamEvent, ModelStreamSink,
    RecordingModelStream,
};
use crate::provider_transport::ProviderTransportConfig;
use crate::query_loop::{
    QueryLoopAction, QueryLoopContinueReason, QueryLoopOutcome, QueryLoopRunner, QueryLoopTerminal,
};
use crate::stop_hook::StopHookResult;
use crate::tool_execution::{
    DefaultToolExecutor, ToolCallResult, ToolExecutionTrace, ToolExecutor,
};

pub(super) fn submit_message(
    engine: &mut QueryEngine,
    prompt: String,
    options: SubmitMessageOptions,
) -> QuerySubmissionPlan {
    let executor = DefaultToolExecutor::new(engine.config().cwd.clone()).with_provider(
        engine.config().model_provider,
        engine.config().user_specified_model.clone(),
    );
    let mut persistence = super::persistence::build_local_persistence_backend(engine);
    submit_message_with_runtime_and_stream(
        engine,
        prompt,
        options,
        &executor,
        &mut persistence,
        None,
    )
}

pub(super) fn submit_message_with_stream(
    engine: &mut QueryEngine,
    prompt: String,
    options: SubmitMessageOptions,
    stream: &mut dyn ModelStreamSink,
) -> QuerySubmissionPlan {
    let executor = DefaultToolExecutor::new(engine.config().cwd.clone()).with_provider(
        engine.config().model_provider,
        engine.config().user_specified_model.clone(),
    );
    let mut persistence = super::persistence::build_local_persistence_backend(engine);
    submit_message_with_runtime_and_stream(
        engine,
        prompt,
        options,
        &executor,
        &mut persistence,
        Some(stream),
    )
}

pub(super) fn submit_message_with_executor(
    engine: &mut QueryEngine,
    prompt: String,
    options: SubmitMessageOptions,
    executor: &impl ToolExecutor,
) -> QuerySubmissionPlan {
    let mut persistence = NoopPersistenceBackend;
    submit_message_with_runtime_and_stream(
        engine,
        prompt,
        options,
        executor,
        &mut persistence,
        None,
    )
}

pub(super) fn submit_message_with_executor_and_stream(
    engine: &mut QueryEngine,
    prompt: String,
    options: SubmitMessageOptions,
    executor: &impl ToolExecutor,
    stream: &mut dyn ModelStreamSink,
) -> QuerySubmissionPlan {
    let mut persistence = NoopPersistenceBackend;
    submit_message_with_runtime_and_stream(
        engine,
        prompt,
        options,
        executor,
        &mut persistence,
        Some(stream),
    )
}

pub(super) fn submit_message_with_runtime(
    engine: &mut QueryEngine,
    prompt: String,
    options: SubmitMessageOptions,
    executor: &impl ToolExecutor,
    persistence_backend: &mut impl PersistenceBackend,
) -> QuerySubmissionPlan {
    submit_message_with_runtime_and_stream(
        engine,
        prompt,
        options,
        executor,
        persistence_backend,
        None,
    )
}

pub(super) fn submit_message_with_runtime_and_stream(
    engine: &mut QueryEngine,
    prompt: String,
    options: SubmitMessageOptions,
    executor: &impl ToolExecutor,
    persistence_backend: &mut impl PersistenceBackend,
    mut stream: Option<&mut dyn ModelStreamSink>,
) -> QuerySubmissionPlan {
    engine.state.mutable_messages = engine
        .deps
        .microcompact
        .compact(engine.state.mutable_messages.as_slice());
    engine
        .state
        .mutable_messages
        .push(crate::message::QueryMessage::user(prompt.clone()));
    engine.state.history_store.record_entry(HistoryEntry::new(
        engine.config.session_id.clone(),
        engine.config.cwd.clone(),
        prompt.clone(),
    ));
    engine
        .state
        .budget_state
        .begin_turn(engine.state.total_usage.output_tokens);

    let mut file_history = engine.state.file_history.request_snapshot();
    let query_config = engine.build_query_config();
    let params = query_config.to_loop_params(engine.state.mutable_messages.clone());
    let mut runner = QueryLoopRunner::new(params.clone());
    let tool_pool = engine.resolve_tool_pool();
    let mut steps = Vec::new();
    let mut requested_tools = Vec::new();
    let mut tool_results = Vec::new();
    let mut model_invocation = None;
    let mut model_error: Option<ModelError> = None;

    let initial_outcome = runner.apply(QueryLoopAction::BindToolUseContext {
        label: String::from("sdk-bootstrap"),
    });
    record_step(&mut steps, "bind-tool-context", &initial_outcome);

    // Incremental persistence: flush initial transcript entries (system prompt + user message)
    flush_incremental_transcript(&mut runner, persistence_backend);

    let mut outcome = initial_outcome;
    for tool_call in engine.bootstrap_tool_calls(&tool_pool) {
        requested_tools.push(tool_call.clone());
        outcome = runner.apply(QueryLoopAction::RequestTool {
            call: tool_call.clone(),
        });
        record_step(
            &mut steps,
            format!("request-tool:{}", tool_call.tool_name),
            &outcome,
        );

        let execution = execute_tool_call(engine, executor, tool_call.clone(), &tool_pool);
        handle_tool_execution(
            engine,
            &mut runner,
            &mut steps,
            &mut tool_results,
            execution,
            &mut outcome,
        );
        // Bootstrap tools don't stream results

        // Incremental persistence: flush bootstrap tool entries
        flush_incremental_transcript(&mut runner, persistence_backend);

        if matches!(outcome, QueryLoopOutcome::Terminal(_)) {
            break;
        }
    }

    if !requested_tools.is_empty() && matches!(outcome, QueryLoopOutcome::Continue(_)) {
        outcome = runner.apply(QueryLoopAction::FlushToolBatch);
        record_step(&mut steps, "flush-tool-batch", &outcome);
    }

    if matches!(outcome, QueryLoopOutcome::Continue(_)) {
        let stop_hook_result = engine
            .deps
            .stop_hook_runner
            .run_stop_hooks(runner.state().messages.as_slice());
        if stop_hook_result_is_meaningful(&stop_hook_result) {
            outcome = runner.apply(QueryLoopAction::RecordStopHookResult {
                result: stop_hook_result.clone(),
            });
            record_step(
                &mut steps,
                stop_hook_step_label(&stop_hook_result),
                &outcome,
            );
            if stop_hook_result.prevent_continuation {
                outcome = runner.apply(QueryLoopAction::Continue(
                    QueryLoopContinueReason::StopHookBlocking,
                ));
                record_step(&mut steps, "continue:stop-hook-blocking", &outcome);
            }
        }
    }

    if engine.config.task_budget.is_some()
        && matches!(outcome, QueryLoopOutcome::Continue(_))
        && !runner.state().stop_hook_active
    {
        let estimated_turn_tokens = estimate_turn_tokens(runner.state().messages.as_slice());
        outcome = runner.apply(QueryLoopAction::CheckTokenBudget {
            global_turn_tokens: estimated_turn_tokens,
            now_ms: engine.deps.clock.now_ms(),
        });
        let decision_label = runner
            .state()
            .token_budget_decision
            .as_ref()
            .map_or("none", TokenBudgetDecision::action_label);
        record_step(
            &mut steps,
            format!("token-budget:{decision_label}"),
            &outcome,
        );
    }

    // Agentic loop: call model → execute tools → flush → repeat until done.
    let max_loop_turns = engine.config.max_turns.unwrap_or(10) as usize;
    let mut loop_turn = 0;

    while matches!(outcome, QueryLoopOutcome::Continue(_))
        && !runner.state().stop_hook_active
        && loop_turn < max_loop_turns
    {
        loop_turn += 1;
        let model_request = build_model_request(&query_config, runner.state());
        let mut fanout = FanoutModelStream::new(stream);
        match model_request.to_transport_http_request() {
            Ok(http_request) => match engine
                .deps
                .call_model
                .call_model(&model_request, &mut fanout)
            {
                Ok(output) => {
                    let transport_request =
                        ProviderTransportConfig::for_provider(model_request.selection.provider)
                            .prepare_http_request(&http_request);

                    // Extract tool calls from model response.
                    let model_tool_calls = output.tool_calls.clone();

                    let (recording, returned_stream) = fanout.finish();
                    stream = returned_stream;
                    let invocation = ModelInvocation::from_call(
                        &model_request,
                        http_request,
                        transport_request,
                        output,
                        recording.events,
                    );
                    outcome = runner.apply(QueryLoopAction::PushAssistantMessage {
                        message: invocation.output_message.clone(),
                    });
                    record_step(&mut steps, "call-model", &outcome);
                    model_invocation = Some(invocation);

                    // Incremental persistence: flush assistant message entry
                    flush_incremental_transcript(&mut runner, persistence_backend);

                    // If model requested tool calls, execute them.
                    if !model_tool_calls.is_empty()
                        && matches!(outcome, QueryLoopOutcome::Continue(_))
                    {
                        for tc in &model_tool_calls {
                            let tool_call = crate::tool_execution::ToolCallInput::new(
                                tc.name.as_str(),
                                tc.id.as_str(),
                            )
                            .with_arguments_map(&tc.arguments);
                            requested_tools.push(tool_call.clone());
                            outcome = runner.apply(QueryLoopAction::RequestTool {
                                call: tool_call.clone(),
                            });
                            record_step(&mut steps, format!("model-tool:{}", tc.name), &outcome);

                            let execution =
                                execute_tool_call(engine, executor, tool_call, &tool_pool);
                            handle_tool_execution(
                                engine,
                                &mut runner,
                                &mut steps,
                                &mut tool_results,
                                execution,
                                &mut outcome,
                            );
                            // Push tool result to stream for real-time TUI display
                            if let Some(last_result) = tool_results.last() {
                                push_tool_result_to_stream(&mut stream, last_result);
                            }

                            // Incremental persistence: flush tool result entries
                            flush_incremental_transcript(&mut runner, persistence_backend);

                            if matches!(outcome, QueryLoopOutcome::Terminal(_)) {
                                break;
                            }
                        }

                        if !requested_tools.is_empty()
                            && matches!(outcome, QueryLoopOutcome::Continue(_))
                        {
                            outcome = runner.apply(QueryLoopAction::FlushToolBatch);
                            record_step(&mut steps, "flush-model-tool-batch", &outcome);

                            // Incremental persistence: flush turn boundary entries
                            flush_incremental_transcript(&mut runner, persistence_backend);
                        }
                        // Loop back to call model again with tool results
                        continue;
                    }

                    // No tool calls — model is done, complete.
                    if matches!(outcome, QueryLoopOutcome::Continue(_)) {
                        outcome = runner.apply(QueryLoopAction::Complete);
                        record_step(&mut steps, "complete", &outcome);
                    }
                    break;
                }
                Err(error) => {
                    let (_recording, _returned_stream) = fanout.finish();
                    model_error = Some(error.clone());
                    outcome = runner.apply(QueryLoopAction::Fail(QueryLoopTerminal::ModelError {
                        error,
                    }));
                    record_step(&mut steps, "call-model:error", &outcome);
                    break;
                }
            },
            Err(error) => {
                let (_recording, _returned_stream) = fanout.finish();
                model_error = Some(error.clone());
                outcome = runner.apply(QueryLoopAction::Fail(QueryLoopTerminal::ModelError {
                    error,
                }));
                record_step(&mut steps, "call-model:error", &outcome);
                break;
            }
        }
    }

    let assistant_turn =
        build_assistant_turn(engine, &params, &tool_results, &outcome, runner.state());
    let model_response =
        build_model_response(engine, &assistant_turn, requested_tools.len(), &outcome);
    let response_result = model_invocation
        .as_ref()
        .and_then(|invocation| invocation.response_result.clone());
    let usage_snapshot = engine.state.usage_tracker.record_turn(
        params.messages.as_slice(),
        params.system_prompt.as_slice(),
        &assistant_turn,
        tool_results.as_slice(),
    );

    engine
        .state
        .budget_state
        .sync_turn_output_tokens(usage_snapshot.total_usage.output_tokens);
    engine
        .state
        .budget_state
        .record_decision(runner.state().token_budget_decision.clone());
    if file_history.snapshot_requested && engine.config.persist_session {
        file_history = engine.state.file_history.commit_snapshot();
    }

    let history_store = engine
        .state
        .history_store
        .record_submit(&engine.config.session_id);
    let session_persistence = engine.state.session_persistence.record_submit(
        runner.state().transcript.entries.len(),
        runner.state().transcript.entries.len(),
        &history_store,
        &file_history,
    );
    let persistence_dispatch = super::persistence::persist_submission(
        persistence_backend,
        &runner.state().transcript,
        &history_store,
        &file_history,
        &session_persistence,
    );

    engine.state.mutable_messages = engine
        .deps
        .autocompact
        .compact(runner.state().messages.as_slice());
    engine.state.completed_turns.push(assistant_turn.clone());
    engine
        .state
        .completed_responses
        .push(model_response.clone());
    engine.state.total_usage = usage_snapshot.total_usage.clone();

    QuerySubmissionPlan {
        prompt,
        prompt_uuid: options.uuid,
        is_meta: options.is_meta,
        query_config,
        message_count_after_submit: engine.state.mutable_messages.len(),
        loop_params: params,
        available_tools: tool_pool.available_tools,
        unavailable_tools: tool_pool.unavailable_tools,
        steps,
        requested_tools,
        tool_results,
        token_budget_decision: runner.state().token_budget_decision.clone(),
        budget_state: engine.state.budget_state.clone(),
        history_store,
        file_history,
        assistant_turn,
        model_response,
        model_error,
        model_invocation,
        response_result,
        transcript: runner.state().transcript.clone(),
        usage_snapshot,
        session_persistence,
        persistence_dispatch,
        loop_outcome: outcome,
    }
}

fn flush_incremental_transcript(
    runner: &mut QueryLoopRunner,
    backend: &mut impl PersistenceBackend,
) {
    let pending = runner.drain_unflushed_transcript_entries();
    if !pending.is_empty() {
        backend.append_transcript_entries(pending);
    }
}

struct FanoutModelStream<'a> {
    recording: RecordingModelStream,
    external: Option<&'a mut dyn ModelStreamSink>,
}

impl<'a> FanoutModelStream<'a> {
    fn new(external: Option<&'a mut dyn ModelStreamSink>) -> Self {
        Self {
            recording: RecordingModelStream::default(),
            external,
        }
    }

    /// Finish recording and return both the recording and the borrowed stream.
    fn finish(self) -> (RecordingModelStream, Option<&'a mut dyn ModelStreamSink>) {
        (self.recording, self.external)
    }
}

impl ModelStreamSink for FanoutModelStream<'_> {
    fn push(&mut self, event: ModelStreamEvent) {
        self.recording.push(event.clone());
        if let Some(external) = self.external.as_deref_mut() {
            external.push(event);
        }
    }
}

fn execute_tool_call(
    engine: &QueryEngine,
    executor: &impl ToolExecutor,
    tool_call: crate::tool_execution::ToolCallInput,
    tool_pool: &crate::tool_registry::ToolRegistrySelection,
) -> ToolExecutionTrace {
    let request = engine.bootstrap_execution_request(tool_call.clone(), tool_pool);
    let execution = executor.execute(request);
    if !matches!(execution.result, ToolCallResult::Completed { .. })
        && tool_pool.has_tool(tool_call.tool_name.as_str())
    {
        let fallback_result = engine.deps.tool_runner.run_tool(tool_call);
        return ToolExecutionTrace {
            progress_updates: execution.progress_updates,
            result: fallback_result,
            permission_denial: execution.permission_denial,
        };
    }
    execution
}

fn record_step(
    steps: &mut Vec<QueryPlanStep>,
    action: impl Into<String>,
    outcome: &QueryLoopOutcome,
) {
    steps.push(QueryPlanStep {
        action: action.into(),
        outcome: outcome.clone(),
    });
}

fn handle_tool_execution(
    engine: &mut QueryEngine,
    runner: &mut QueryLoopRunner,
    steps: &mut Vec<QueryPlanStep>,
    tool_results: &mut Vec<ToolCallResult>,
    execution: ToolExecutionTrace,
    outcome: &mut QueryLoopOutcome,
) {
    for progress in &execution.progress_updates {
        *outcome = runner.apply(QueryLoopAction::PushToolProgress {
            update: progress.clone(),
        });
        record_step(
            steps,
            format!("tool-progress:{}", progress.tool_use_id),
            outcome,
        );
    }

    let ToolExecutionTrace {
        result: tool_result,
        permission_denial,
        ..
    } = execution;
    if let Some(deny_reason) = permission_denial {
        engine.state.permission_denials.push(deny_reason);
    }
    tool_results.push(tool_result.clone());
    *outcome = runner.apply(QueryLoopAction::ResolveTool {
        result: tool_result.clone(),
    });
    record_step(
        steps,
        format!("resolve-tool:{}", tool_result.status_label()),
        outcome,
    );
}

/// Push a tool result event to the stream sink for real-time TUI display.
fn push_tool_result_to_stream(
    stream: &mut Option<&mut dyn ModelStreamSink>,
    tool_result: &ToolCallResult,
) {
    let Some(sink) = stream.as_mut() else { return };
    let (tool_name, content, is_error) = match tool_result {
        ToolCallResult::Completed { call, output, .. } => {
            (call.tool_name.clone(), output.summary.clone(), false)
        }
        ToolCallResult::Failed { call, error, .. } => (call.tool_name.clone(), error.clone(), true),
        ToolCallResult::Denied { call, reason, .. } => {
            (call.tool_name.clone(), reason.clone(), true)
        }
    };
    sink.push(ModelStreamEvent::ToolResult {
        tool_name,
        content,
        is_error,
    });
}

fn build_model_request(
    query_config: &crate::query_config::QueryConfig,
    runner_state: &crate::query_loop::QueryLoopState,
) -> ModelRequest {
    ModelRequest {
        selection: query_config.model_selection.clone(),
        system_prompt: query_config.system_prompt.clone(),
        conversation: runner_state.messages.clone(),
        model_reasoning_effort: query_config.model_reasoning_effort.clone(),
        json_schema: query_config.json_schema.clone(),
        query_source: query_config.query_source,
        stream_mode: query_config.stream_mode(),
        max_turns: query_config.max_turns,
        task_budget: query_config.task_budget,
        verbose: query_config.runtime_gates.verbose,
        replay_user_messages: query_config.runtime_gates.replay_user_messages,
        include_partial_messages: query_config.runtime_gates.include_partial_messages,
        tool_definitions: query_config.tool_definitions.clone(),
    }
}

fn build_assistant_turn(
    engine: &QueryEngine,
    params: &crate::query_loop::QueryLoopParams,
    tool_results: &[ToolCallResult],
    outcome: &QueryLoopOutcome,
    runner_state: &crate::query_loop::QueryLoopState,
) -> AssistantTurn {
    let response_messages = runner_state.messages[params.messages.len()..].to_vec();
    AssistantTurn::new(
        engine.state.completed_turns.len() as u32 + 1,
        assistant_turn_status(outcome),
        response_messages,
        tool_results,
        runner_state.transcript.entries.len(),
    )
}

fn build_model_response(
    engine: &QueryEngine,
    assistant_turn: &AssistantTurn,
    requested_tools: usize,
    outcome: &QueryLoopOutcome,
) -> ModelResponse {
    ModelResponse::new(
        format!("resp-{}", engine.deps.id_gen.generate()),
        assistant_turn.status,
        stop_reason(outcome),
        requested_tools,
        assistant_turn.clone(),
    )
}

fn stop_hook_result_is_meaningful(result: &StopHookResult) -> bool {
    result.prevent_continuation
        || result.has_output
        || result.hook_count > 0
        || !result.blocking_errors.is_empty()
        || result.stop_reason.is_some()
        || !result.hook_errors.is_empty()
        || !result.hook_infos.is_empty()
}

fn stop_hook_step_label(result: &StopHookResult) -> &'static str {
    if result.prevent_continuation {
        "stop-hooks:blocking"
    } else {
        "stop-hooks:recorded"
    }
}

fn assistant_turn_status(outcome: &QueryLoopOutcome) -> AssistantTurnStatus {
    match outcome {
        QueryLoopOutcome::Continue(_) => AssistantTurnStatus::Continue,
        QueryLoopOutcome::Terminal(QueryLoopTerminal::Completed) => AssistantTurnStatus::Completed,
        QueryLoopOutcome::Terminal(_) => AssistantTurnStatus::Terminal,
    }
}

fn stop_reason(outcome: &QueryLoopOutcome) -> ModelResponseStopReason {
    match outcome {
        QueryLoopOutcome::Continue(_) => ModelResponseStopReason::ToolBatchFlushed,
        QueryLoopOutcome::Terminal(QueryLoopTerminal::Completed) => {
            ModelResponseStopReason::Completed
        }
        QueryLoopOutcome::Terminal(QueryLoopTerminal::MaxTurns { .. }) => {
            ModelResponseStopReason::MaxTurns
        }
        QueryLoopOutcome::Terminal(_) => ModelResponseStopReason::Terminal,
    }
}
