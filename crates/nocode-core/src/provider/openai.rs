use crate::provider::Provider;
use crate::provider::types::{
    CreateMessageRequest, CreateMessageResponse, ProviderError, StreamEvent,
};

/// OpenAI Chat Completions / Responses API provider (stub).
pub struct OpenAiProvider;

impl Provider for OpenAiProvider {
    fn create_message(
        &self,
        _request: &CreateMessageRequest,
    ) -> Result<CreateMessageResponse, ProviderError> {
        Err(ProviderError::non_retryable(
            "OpenAI provider not yet implemented",
        ))
    }

    fn create_message_stream(
        &self,
        _request: &CreateMessageRequest,
        _on_event: &mut dyn FnMut(StreamEvent),
    ) -> Result<CreateMessageResponse, ProviderError> {
        Err(ProviderError::non_retryable(
            "OpenAI streaming not yet implemented",
        ))
    }
}
