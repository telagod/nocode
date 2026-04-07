use crate::provider::Provider;
use crate::provider::types::{
    CreateMessageRequest, CreateMessageResponse, ProviderError, StreamEvent,
};

/// Gemini generateContent API provider (stub).
pub struct GeminiProvider;

impl Provider for GeminiProvider {
    fn create_message(
        &self,
        _request: &CreateMessageRequest,
    ) -> Result<CreateMessageResponse, ProviderError> {
        Err(ProviderError::non_retryable(
            "Gemini provider not yet implemented",
        ))
    }

    fn create_message_stream(
        &self,
        _request: &CreateMessageRequest,
        _on_event: &mut dyn FnMut(StreamEvent),
    ) -> Result<CreateMessageResponse, ProviderError> {
        Err(ProviderError::non_retryable(
            "Gemini streaming not yet implemented",
        ))
    }
}
