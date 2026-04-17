pub mod claude;
pub mod foundry;
pub mod gemini;
pub mod model_caps;
pub mod openai;
pub mod openai_responses;
pub mod pricing;
pub mod resolve;
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

    /// Verify the API key is valid. Returns Ok(model_info) or Err on failure.
    fn verify_key(&self) -> Result<String, ProviderError> {
        Ok("key verification not implemented".to_string())
    }
}

/// Helper: box any Provider reference into a owned trait object.
/// This uses a wrapper that delegates to an Arc, avoiding the need for Clone.
pub struct ProviderBox {
    inner: std::sync::Arc<dyn Provider + 'static>,
}

impl ProviderBox {
    pub fn new(provider: impl Provider + 'static) -> Self {
        Self {
            inner: std::sync::Arc::new(provider),
        }
    }

    pub fn from_arc(arc: std::sync::Arc<dyn Provider + 'static>) -> Self {
        Self { inner: arc }
    }
}

impl Provider for ProviderBox {
    fn create_message(
        &self,
        request: &CreateMessageRequest,
    ) -> Result<CreateMessageResponse, ProviderError> {
        self.inner.create_message(request)
    }

    fn create_message_stream(
        &self,
        request: &CreateMessageRequest,
        on_event: &mut dyn FnMut(StreamEvent),
    ) -> Result<CreateMessageResponse, ProviderError> {
        self.inner.create_message_stream(request, on_event)
    }

    fn verify_key(&self) -> Result<String, ProviderError> {
        self.inner.verify_key()
    }
}

impl Clone for ProviderBox {
    fn clone(&self) -> Self {
        Self {
            inner: std::sync::Arc::clone(&self.inner),
        }
    }
}

impl AsRef<dyn Provider + 'static> for ProviderBox {
    fn as_ref(&self) -> &(dyn Provider + 'static) {
        &*self.inner
    }
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
