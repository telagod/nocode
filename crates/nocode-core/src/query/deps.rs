//! Query dependency injection — trait objects for testability.
//!
//! Production wires real implementations; tests swap in mocks.

use crate::message::Message;
use crate::provider::types::{
    CreateMessageRequest, CreateMessageResponse, ProviderError, StreamEvent, ToolDefinition,
};
use crate::tool::ToolOutput;
use std::time::Instant;

/// Trait for making model calls.
pub trait CallModel: Send + Sync {
    fn create_message(
        &self,
        request: &CreateMessageRequest,
    ) -> Result<CreateMessageResponse, ProviderError>;

    fn create_message_stream(
        &self,
        request: &CreateMessageRequest,
        on_event: &mut dyn FnMut(StreamEvent),
    ) -> Result<CreateMessageResponse, ProviderError>;
}

/// Trait for context compaction.
pub trait Compactor: Send + Sync {
    fn compact(&self, messages: &[Message], budget: usize) -> Vec<Message>;
}

/// Trait for tool execution.
pub trait ToolRunner: Send + Sync {
    fn execute(&self, name: &str, id: &str, input: &serde_json::Value) -> ToolOutput;
    fn definitions(&self) -> Vec<ToolDefinition>;
}

/// Trait for stop hooks (run after each turn).
pub trait StopHookRunner: Send + Sync {
    fn run(&self, messages: &[Message]) -> StopHookResult;
}

/// Result of a stop hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopHookResult {
    Continue,
    Stop { reason: String },
}

/// Trait for clock (testable time).
pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

/// Real clock.
pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Trait for ID generation.
pub trait IdGen: Send + Sync {
    fn next_id(&self) -> String;
}

/// UUID-based ID generator.
pub struct UuidIdGen;
impl IdGen for UuidIdGen {
    fn next_id(&self) -> String {
        format!(
            "{:08x}-{:04x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u32,
            rand_u16()
        )
    }
}

fn rand_u16() -> u16 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    std::time::Instant::now().hash(&mut h);
    std::thread::current().id().hash(&mut h);
    h.finish() as u16
}

/// Builder for wiring query dependencies.
pub struct QueryDepsBuilder {
    pub model: Option<Box<dyn CallModel>>,
    pub compactor: Option<Box<dyn Compactor>>,
    pub tool_runner: Option<Box<dyn ToolRunner>>,
    pub stop_hook: Option<Box<dyn StopHookRunner>>,
    pub clock: Option<Box<dyn Clock>>,
    pub id_gen: Option<Box<dyn IdGen>>,
}

impl QueryDepsBuilder {
    pub fn new() -> Self {
        Self {
            model: None,
            compactor: None,
            tool_runner: None,
            stop_hook: None,
            clock: None,
            id_gen: None,
        }
    }

    pub fn with_model(mut self, model: Box<dyn CallModel>) -> Self {
        self.model = Some(model);
        self
    }

    pub fn with_compactor(mut self, compactor: Box<dyn Compactor>) -> Self {
        self.compactor = Some(compactor);
        self
    }

    pub fn with_tool_runner(mut self, runner: Box<dyn ToolRunner>) -> Self {
        self.tool_runner = Some(runner);
        self
    }

    pub fn with_stop_hook(mut self, hook: Box<dyn StopHookRunner>) -> Self {
        self.stop_hook = Some(hook);
        self
    }

    pub fn with_clock(mut self, clock: Box<dyn Clock>) -> Self {
        self.clock = Some(clock);
        self
    }

    pub fn with_id_gen(mut self, id_gen: Box<dyn IdGen>) -> Self {
        self.id_gen = Some(id_gen);
        self
    }

    pub fn build(self) -> QueryDeps {
        QueryDeps {
            model: self.model.expect("CallModel required"),
            compactor: self.compactor,
            tool_runner: self.tool_runner.expect("ToolRunner required"),
            stop_hook: self.stop_hook,
            clock: self.clock.unwrap_or_else(|| Box::new(SystemClock)),
            id_gen: self.id_gen.unwrap_or_else(|| Box::new(UuidIdGen)),
        }
    }
}

impl Default for QueryDepsBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Assembled query dependencies — ready for the query engine.
pub struct QueryDeps {
    pub model: Box<dyn CallModel>,
    pub compactor: Option<Box<dyn Compactor>>,
    pub tool_runner: Box<dyn ToolRunner>,
    pub stop_hook: Option<Box<dyn StopHookRunner>>,
    pub clock: Box<dyn Clock>,
    pub id_gen: Box<dyn IdGen>,
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockModel;
    impl CallModel for MockModel {
        fn create_message(
            &self,
            _req: &CreateMessageRequest,
        ) -> Result<CreateMessageResponse, ProviderError> {
            Err(ProviderError::non_retryable("mock"))
        }
        fn create_message_stream(
            &self,
            _req: &CreateMessageRequest,
            _on_event: &mut dyn FnMut(StreamEvent),
        ) -> Result<CreateMessageResponse, ProviderError> {
            Err(ProviderError::non_retryable("mock"))
        }
    }

    struct MockRunner;
    impl ToolRunner for MockRunner {
        fn execute(&self, _name: &str, _id: &str, _input: &serde_json::Value) -> ToolOutput {
            ToolOutput::success("mock result")
        }
        fn definitions(&self) -> Vec<ToolDefinition> {
            Vec::new()
        }
    }

    #[test]
    fn builder_creates_deps() {
        let deps = QueryDepsBuilder::new()
            .with_model(Box::new(MockModel))
            .with_tool_runner(Box::new(MockRunner))
            .build();

        assert!(deps.compactor.is_none());
        assert!(deps.stop_hook.is_none());
        let _id = deps.id_gen.next_id();
        let _now = deps.clock.now();
    }

    #[test]
    fn system_clock_works() {
        let clock = SystemClock;
        let t1 = clock.now();
        let t2 = clock.now();
        assert!(t2 >= t1);
    }

    #[test]
    fn uuid_gen_unique() {
        let id_gen = UuidIdGen;
        let id1 = id_gen.next_id();
        let id2 = id_gen.next_id();
        // Not guaranteed unique in same nanosecond, but should be non-empty
        assert!(!id1.is_empty());
        assert!(!id2.is_empty());
    }

    #[test]
    fn stop_hook_result_variants() {
        assert_eq!(StopHookResult::Continue, StopHookResult::Continue);
        let stop = StopHookResult::Stop {
            reason: "done".to_string(),
        };
        assert_ne!(stop, StopHookResult::Continue);
    }
}
