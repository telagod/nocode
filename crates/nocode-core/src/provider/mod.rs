pub mod types;
pub mod transport;
pub mod claude;
pub mod openai;
pub mod gemini;

use types::{
    CreateMessageRequest, CreateMessageResponse, ProviderError, StreamEvent,
};

/// Trait for model providers (Claude, OpenAI, Gemini, etc.)
pub trait Provider: Send + Sync {
    /// Make a non-streaming model call.
    fn create_message(
        &self,
        request: &CreateMessageRequest,
    ) -> Result<CreateMessageResponse, ProviderError>;

    /// Make a streaming model call, pushing events to the callback.
    fn create_message_stream(
        &self,
        request: &CreateMessageRequest,
        on_event: &mut dyn FnMut(StreamEvent),
    ) -> Result<CreateMessageResponse, ProviderError>;
}
