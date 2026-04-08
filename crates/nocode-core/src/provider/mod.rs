pub mod claude;
pub mod gemini;
pub mod openai;
pub mod transport;
pub mod types;

use types::{CreateMessageRequest, CreateMessageResponse, ProviderError, StreamEvent};

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

// ---------------------------------------------------------------------------
// CallModel bridge — connects DI layer to Provider trait
// ---------------------------------------------------------------------------

use crate::query::deps::CallModel;

/// Blanket bridge: any Provider automatically implements CallModel.
impl<T: Provider> CallModel for T {
    fn create_message(
        &self,
        request: &CreateMessageRequest,
    ) -> Result<CreateMessageResponse, ProviderError> {
        Provider::create_message(self, request)
    }

    fn create_message_stream(
        &self,
        request: &CreateMessageRequest,
        on_event: &mut dyn FnMut(StreamEvent),
    ) -> Result<CreateMessageResponse, ProviderError> {
        Provider::create_message_stream(self, request, on_event)
    }
}
