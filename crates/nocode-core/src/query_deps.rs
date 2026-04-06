use crate::{
    message::QueryMessage,
    provider::{
        ModelCallOutput, ModelError, ModelRequest, ModelStreamEvent, ModelStreamMode,
        ModelStreamSink, StreamingModelParser, parse_model_response,
    },
    provider_transport::ProviderTransportConfig,
    session_compaction::{CompactionConfig, RichCompactor},
    stop_hook::StopHookResult,
    tool_execution::{ToolCallInput, ToolCallOutput, ToolCallResult, ToolProgressUpdate},
};
use std::{
    fmt::Debug,
    sync::{Arc, atomic::AtomicU64, atomic::Ordering},
};

pub trait CallModel: Send + Sync + Debug {
    fn call_model(
        &self,
        request: &ModelRequest,
        stream: &mut dyn ModelStreamSink,
    ) -> Result<ModelCallOutput, ModelError>;
}

pub trait Compactor: Send + Sync + Debug {
    fn compact(&self, messages: &[QueryMessage]) -> Vec<QueryMessage>;
}

pub trait ToolRunner: Send + Sync + Debug {
    fn run_tool(&self, request: ToolCallInput) -> ToolCallResult;
}

pub trait StopHookRunner: Send + Sync + Debug {
    fn run_stop_hooks(&self, messages: &[QueryMessage]) -> StopHookResult;
}

pub trait Clock: Send + Sync + Debug {
    fn now_ms(&self) -> u64;
}

pub trait IdGen: Send + Sync + Debug {
    fn generate(&self) -> String;
}

#[derive(Debug, Clone)]
pub struct QueryDeps {
    pub call_model: Arc<dyn CallModel>,
    pub microcompact: Arc<dyn Compactor>,
    pub autocompact: Arc<dyn Compactor>,
    pub tool_runner: Arc<dyn ToolRunner>,
    pub stop_hook_runner: Arc<dyn StopHookRunner>,
    pub clock: Arc<dyn Clock>,
    pub id_gen: Arc<dyn IdGen>,
}

impl QueryDeps {
    pub fn builder() -> QueryDepsBuilder {
        QueryDepsBuilder::new()
    }
}

pub fn production_deps() -> QueryDeps {
    let compactor = RichCompactor::new(CompactionConfig::default());
    QueryDeps::builder()
        .with_microcompact(compactor)
        .with_tool_runner(RescuingToolRunner)
        .build()
}

pub fn production_deps_with_tool_runner(runner: impl ToolRunner + 'static) -> QueryDeps {
    let compactor = RichCompactor::new(CompactionConfig::default());
    QueryDeps::builder()
        .with_microcompact(compactor)
        .with_tool_runner(runner)
        .build()
}

/// Production deps with `DefaultToolRunner` (returns failed) — used when a custom
/// executor is provided and fallback should not rescue failed tool calls.
pub fn production_deps_without_rescue() -> QueryDeps {
    let compactor = RichCompactor::new(CompactionConfig::default());
    QueryDeps::builder()
        .with_microcompact(compactor)
        .with_tool_runner(DefaultToolRunner)
        .build()
}

pub struct QueryDepsBuilder {
    call_model: Option<Arc<dyn CallModel>>,
    microcompact: Option<Arc<dyn Compactor>>,
    autocompact: Option<Arc<dyn Compactor>>,
    tool_runner: Option<Arc<dyn ToolRunner>>,
    stop_hook_runner: Option<Arc<dyn StopHookRunner>>,
    clock: Option<Arc<dyn Clock>>,
    id_gen: Option<Arc<dyn IdGen>>,
}

impl Default for QueryDepsBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl QueryDepsBuilder {
    pub fn new() -> Self {
        Self {
            call_model: None,
            microcompact: None,
            autocompact: None,
            tool_runner: None,
            stop_hook_runner: None,
            clock: None,
            id_gen: None,
        }
    }

    pub fn with_call_model(mut self, call_model: impl CallModel + 'static) -> Self {
        self.call_model = Some(Arc::new(call_model));
        self
    }

    pub fn with_microcompact(mut self, compact: impl Compactor + 'static) -> Self {
        self.microcompact = Some(Arc::new(compact));
        self
    }

    pub fn with_autocompact(mut self, compact: impl Compactor + 'static) -> Self {
        self.autocompact = Some(Arc::new(compact));
        self
    }

    pub fn with_tool_runner(mut self, runner: impl ToolRunner + 'static) -> Self {
        self.tool_runner = Some(Arc::new(runner));
        self
    }

    pub fn with_stop_hook_runner(mut self, runner: impl StopHookRunner + 'static) -> Self {
        self.stop_hook_runner = Some(Arc::new(runner));
        self
    }

    pub fn with_clock(mut self, clock: impl Clock + 'static) -> Self {
        self.clock = Some(Arc::new(clock));
        self
    }

    pub fn with_id_gen(mut self, id_gen: impl IdGen + 'static) -> Self {
        self.id_gen = Some(Arc::new(id_gen));
        self
    }

    pub fn build(self) -> QueryDeps {
        QueryDeps {
            call_model: self
                .call_model
                .unwrap_or_else(|| Arc::new(DefaultCallModel)),
            microcompact: self
                .microcompact
                .unwrap_or_else(|| Arc::new(DefaultCompactor)),
            autocompact: self
                .autocompact
                .unwrap_or_else(|| Arc::new(DefaultCompactor)),
            tool_runner: self
                .tool_runner
                .unwrap_or_else(|| Arc::new(RescuingToolRunner)),
            stop_hook_runner: self
                .stop_hook_runner
                .unwrap_or_else(|| Arc::new(DefaultStopHookRunner)),
            clock: self.clock.unwrap_or_else(|| Arc::new(SystemClock)),
            id_gen: self.id_gen.unwrap_or_else(|| Arc::new(DefaultIdGen::new())),
        }
    }
}

#[derive(Debug)]
struct DefaultCallModel;

impl CallModel for DefaultCallModel {
    fn call_model(
        &self,
        request: &ModelRequest,
        stream: &mut dyn ModelStreamSink,
    ) -> Result<ModelCallOutput, ModelError> {
        if request.selection.provider == crate::provider::ModelProvider::Mock {
            return call_mock_model(request, stream);
        }

        let transport = ProviderTransportConfig::from_env(request.selection.provider)?;
        let http_request = request.to_transport_http_request()?;
        let selected_model = request
            .selection
            .selected_model()
            .ok_or_else(ModelError::no_model_configured)?;

        stream.push(ModelStreamEvent::Start {
            provider: request.selection.provider,
            model: selected_model.to_string(),
        });

        let output = if matches!(request.stream_mode, ModelStreamMode::Enabled) {
            let mut parser = StreamingModelParser::new(request);
            let response = transport
                .execute_streaming(&http_request, |frame| parser.push_frame(frame, stream))?;
            let is_event_stream = response
                .headers
                .get("content-type")
                .or_else(|| response.headers.get("Content-Type"))
                .is_some_and(|value| value.contains("text/event-stream"));
            if is_event_stream {
                parser.finish(stream)?
            } else {
                let output = parse_model_response(request, &response.body)?;
                stream.push(ModelStreamEvent::Delta {
                    text: output.message.content.clone(),
                });
                output
            }
        } else {
            let response = transport.execute(&http_request)?;
            parse_model_response(request, &response.body)?
        };
        stream.push(ModelStreamEvent::Complete {
            message: output.message.clone(),
        });

        Ok(output)
    }
}

fn call_mock_model(
    request: &ModelRequest,
    stream: &mut dyn ModelStreamSink,
) -> Result<ModelCallOutput, ModelError> {
    let selected_model = request
        .selection
        .selected_model()
        .ok_or_else(ModelError::no_model_configured)?;
    let content = request
        .reply_target()
        .map(|target| format!("nocode response: {target}"))
        .unwrap_or_else(|| format!("nocode response: {selected_model}"));
    let message = QueryMessage::assistant(content);

    stream.push(ModelStreamEvent::Start {
        provider: request.selection.provider,
        model: selected_model.to_string(),
    });
    if matches!(request.stream_mode, ModelStreamMode::Enabled) {
        stream.push(ModelStreamEvent::Delta {
            text: message.content.clone(),
        });
    }
    stream.push(ModelStreamEvent::Complete {
        message: message.clone(),
    });

    Ok(ModelCallOutput::new(
        request.selection.provider,
        selected_model,
        message,
    ))
}

#[derive(Debug)]
struct DefaultCompactor;

impl Compactor for DefaultCompactor {
    fn compact(&self, messages: &[QueryMessage]) -> Vec<QueryMessage> {
        messages.to_vec()
    }
}

/// Token-aware compactor that keeps system messages + recent turns when over threshold.
#[derive(Debug)]
pub struct TruncatingCompactor {
    /// Approximate max tokens before compaction triggers (~4 chars per token).
    pub max_tokens: usize,
    /// Number of recent user+assistant message pairs to keep.
    pub keep_recent_messages: usize,
}

impl TruncatingCompactor {
    pub fn new(max_tokens: usize, keep_recent_messages: usize) -> Self {
        Self {
            max_tokens,
            keep_recent_messages,
        }
    }

    fn estimate_tokens(messages: &[QueryMessage]) -> usize {
        messages.iter().map(|m| m.content.len() / 4 + 1).sum()
    }
}

impl Compactor for TruncatingCompactor {
    fn compact(&self, messages: &[QueryMessage]) -> Vec<QueryMessage> {
        let tokens = Self::estimate_tokens(messages);
        if tokens <= self.max_tokens {
            return messages.to_vec();
        }

        let mut result = Vec::new();

        // Keep all system messages.
        for msg in messages {
            if msg.role == crate::message::QueryMessageRole::System {
                result.push(msg.clone());
            }
        }

        // Keep the most recent N non-system messages.
        let non_system: Vec<&QueryMessage> = messages
            .iter()
            .filter(|m| m.role != crate::message::QueryMessageRole::System)
            .collect();
        let keep_count = self.keep_recent_messages.min(non_system.len());
        let skip = non_system.len().saturating_sub(keep_count);

        // Insert a compaction marker if we dropped messages.
        if skip > 0 {
            result.push(QueryMessage::system(format!(
                "[context compacted: {skip} earlier messages removed to fit context window]"
            )));
        }

        for msg in non_system.into_iter().skip(skip) {
            result.push(msg.clone());
        }

        result
    }
}

#[derive(Debug)]
struct DefaultToolRunner;

impl ToolRunner for DefaultToolRunner {
    fn run_tool(&self, request: ToolCallInput) -> ToolCallResult {
        ToolCallResult::failed(request, "tool runner disabled")
    }
}

/// A tool runner that always returns `Completed` — used as fallback in `ask()` and tests.
#[derive(Debug)]
pub struct RescuingToolRunner;

impl ToolRunner for RescuingToolRunner {
    fn run_tool(&self, request: ToolCallInput) -> ToolCallResult {
        let summary = format!("rescued {}", request.tool_name);
        let tool_use_id = request.tool_use_id.clone();
        let context_label = request.context_label.clone();
        ToolCallResult::Completed {
            call: request,
            user_modified: false,
            output: ToolCallOutput {
                summary,
                generated_messages: vec![QueryMessage::assistant(format!(
                    "tool-message: rescued {tool_use_id}"
                ))],
                context_label: Some(context_label),
                progress_updates: vec![ToolProgressUpdate::new(tool_use_id, "tool complete")],
            },
        }
    }
}

#[derive(Debug)]
struct DefaultStopHookRunner;

impl StopHookRunner for DefaultStopHookRunner {
    fn run_stop_hooks(&self, _messages: &[QueryMessage]) -> StopHookResult {
        StopHookResult::default()
    }
}

#[derive(Debug)]
struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|dur| dur.as_millis() as u64)
            .unwrap_or_default()
    }
}

#[derive(Debug)]
struct DefaultIdGen(AtomicU64);

impl DefaultIdGen {
    fn new() -> Self {
        Self(AtomicU64::new(1))
    }
}

impl IdGen for DefaultIdGen {
    fn generate(&self) -> String {
        self.0.fetch_add(1, Ordering::Relaxed).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{
        ModelProvider, ModelRequest, ModelSelection, ModelStreamEvent, ModelStreamMode,
        RecordingModelStream,
    };
    use crate::tool_execution::{ToolCallOutput, ToolCallResult};

    #[derive(Debug)]
    struct CountingToolRunner;

    impl ToolRunner for CountingToolRunner {
        fn run_tool(&self, request: ToolCallInput) -> ToolCallResult {
            ToolCallResult::Completed {
                call: request,
                user_modified: false,
                output: ToolCallOutput {
                    summary: String::from("ok"),
                    generated_messages: Vec::new(),
                    context_label: None,
                    progress_updates: Vec::new(),
                },
            }
        }
    }

    #[derive(Debug)]
    struct EchoStopHook;

    impl StopHookRunner for EchoStopHook {
        fn run_stop_hooks(&self, messages: &[QueryMessage]) -> StopHookResult {
            let mut result = StopHookResult::default();
            if !messages.is_empty() {
                result.blocking_errors.push(messages[0].clone());
            }
            result
        }
    }

    #[test]
    fn builder_overrides_and_runs_traits() {
        let deps = QueryDeps::builder()
            .with_tool_runner(CountingToolRunner)
            .with_stop_hook_runner(EchoStopHook)
            .with_clock(SystemClock)
            .with_id_gen(DefaultIdGen::new())
            .build();

        let tool_call = ToolCallInput::new("Read", "alpine");
        let result = deps.tool_runner.run_tool(tool_call);
        assert_eq!(result.status_label(), "completed");

        let stop_result = deps
            .stop_hook_runner
            .run_stop_hooks(&[QueryMessage::user("halt me")]);
        assert!(!stop_result.blocking_errors.is_empty());
        assert!(deps.clock.now_ms() > 0);
        assert!(!deps.id_gen.generate().is_empty());
    }

    #[test]
    fn default_call_model_prefers_reply_target_marker() {
        let deps = QueryDeps::builder().build();
        let request = ModelRequest {
            selection: ModelSelection {
                provider: ModelProvider::Mock,
                requested_model: Some(String::from("sonnet")),
                fallback_model: Some(String::from("haiku")),
            },
            system_prompt: vec![QueryMessage::system("system")],
            conversation: vec![QueryMessage::user("rewrite query loop")],
            model_reasoning_effort: None,
            json_schema: None,
            query_source: crate::query_loop::QuerySource::Sdk,
            stream_mode: ModelStreamMode::Enabled,
            max_turns: Some(4),
            task_budget: None,
            verbose: false,
            replay_user_messages: false,
            include_partial_messages: false,
            tool_definitions: Vec::new(),
        };
        let mut stream = RecordingModelStream::default();
        let response = deps
            .call_model
            .call_model(&request, &mut stream)
            .expect("default model call should succeed");

        assert_eq!(
            response.message,
            QueryMessage::assistant("nocode response: rewrite query loop")
        );
        assert_eq!(stream.events.len(), 3);
        assert!(matches!(
            stream.events[0],
            ModelStreamEvent::Start {
                provider: ModelProvider::Mock,
                ..
            }
        ));
    }

    #[test]
    fn builder_defaults_all_deps_without_panic() {
        let deps = QueryDeps::builder().build();
        // All deps should be populated with defaults
        assert!(deps.clock.now_ms() > 0);
        let id1 = deps.id_gen.generate();
        let id2 = deps.id_gen.generate();
        assert_ne!(id1, id2);
    }

    #[test]
    fn truncating_compactor_passes_through_under_threshold() {
        let compactor = TruncatingCompactor::new(1000, 4);
        let messages = vec![
            QueryMessage::system("sys"),
            QueryMessage::user("hi"),
            QueryMessage::assistant("hello"),
        ];
        let result = compactor.compact(&messages);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].content, "sys");
    }

    #[test]
    fn truncating_compactor_keeps_system_and_recent_when_over_threshold() {
        let compactor = TruncatingCompactor::new(5, 2);
        let messages = vec![
            QueryMessage::system("system prompt"),
            QueryMessage::user("old message one that is long enough"),
            QueryMessage::assistant("old reply that is also long enough"),
            QueryMessage::user("recent question"),
            QueryMessage::assistant("recent answer"),
        ];
        let result = compactor.compact(&messages);

        // Should keep system message + compaction marker + 2 most recent non-system
        assert!(result.len() >= 3);
        // First should be the system message
        assert_eq!(result[0].role, crate::message::QueryMessageRole::System);
        assert_eq!(result[0].content, "system prompt");
        // Second should be compaction marker (also system role)
        assert!(result[1].content.contains("context compacted"));
        // Last two should be the recent messages
        let last = &result[result.len() - 1];
        assert_eq!(last.content, "recent answer");
    }

    #[test]
    fn rescuing_tool_runner_returns_completed() {
        let runner = RescuingToolRunner;
        let call = ToolCallInput::new("Bash", "toolu-99").with_context_label("test-ctx");
        let result = runner.run_tool(call);
        assert_eq!(result.status_label(), "completed");
        assert!(result.message().contains("rescued"));
    }

    #[test]
    fn default_tool_runner_returns_failed() {
        let deps = QueryDeps::builder()
            .with_tool_runner(DefaultToolRunner)
            .build();
        let call = ToolCallInput::new("Read", "toolu-1");
        let result = deps.tool_runner.run_tool(call);
        assert_eq!(result.status_label(), "failed");
        assert!(result.message().contains("tool runner disabled"));
    }
}
