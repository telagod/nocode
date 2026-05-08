//! Anthropic Foundry provider — connects to Anthropic Foundry endpoints.
//!
//! Foundry uses the same Messages API as Claude, but with a different
//! base URL pattern and authentication (API key or OAuth bearer token).
//! Endpoint: `https://{foundry_id}.foundry.anthropic.com/v1/messages`

use crate::provider::Provider;
use crate::provider::transport::{HttpTransport, with_retry};
use crate::provider::types::{
    CreateMessageRequest, CreateMessageResponse, ProviderError, StreamEvent,
};
use std::sync::{Arc, atomic::AtomicBool};

/// Anthropic Foundry provider.
pub struct FoundryProvider {
    transport: HttpTransport,
    foundry_id: String,
}

impl FoundryProvider {
    /// Create a Foundry provider with API key auth.
    pub fn new(foundry_id: &str, api_key: &str) -> Self {
        let base_url = format!("https://{foundry_id}.foundry.anthropic.com");
        let transport = HttpTransport::new(&base_url, api_key)
            .with_header("anthropic-version", "2024-06-01")
            .with_header("x-api-key", api_key);
        Self {
            transport,
            foundry_id: foundry_id.to_string(),
        }
    }

    /// Create a Foundry provider with bearer token auth (OAuth).
    pub fn with_bearer_token(foundry_id: &str, token: &str) -> Self {
        let base_url = format!("https://{foundry_id}.foundry.anthropic.com");
        let transport = HttpTransport::new(&base_url, token)
            .with_header("anthropic-version", "2024-06-01")
            .with_header("Authorization", format!("Bearer {token}"));
        Self {
            transport,
            foundry_id: foundry_id.to_string(),
        }
    }

    /// Get the Foundry ID.
    pub fn foundry_id(&self) -> &str {
        &self.foundry_id
    }
    // APPEND_REST

    fn serialize_request(&self, request: &CreateMessageRequest) -> Result<String, ProviderError> {
        serde_json::to_string(request)
            .map_err(|e| ProviderError::non_retryable(format!("Failed to serialize request: {e}")))
    }
}

impl Provider for FoundryProvider {
    fn create_message(
        &self,
        request: &CreateMessageRequest,
    ) -> Result<CreateMessageResponse, ProviderError> {
        let mut req = request.clone();
        req.stream = false;
        let body = self.serialize_request(&req)?;

        let response_text = with_retry(1, || self.transport.post_json("/v1/messages", &body))?;

        serde_json::from_str::<CreateMessageResponse>(&response_text)
            .map_err(|e| ProviderError::non_retryable(format!("Failed to parse response: {e}")))
    }

    fn create_message_stream(
        &self,
        request: &CreateMessageRequest,
        on_event: &mut dyn FnMut(StreamEvent),
    ) -> Result<CreateMessageResponse, ProviderError> {
        let mut req = request.clone();
        req.stream = true;
        let body = self.serialize_request(&req)?;

        let reader = with_retry(1, || self.transport.post_json_stream("/v1/messages", &body))?;

        crate::provider::claude::parse_sse_stream(reader, on_event)
    }

    fn create_message_stream_with_cancel(
        &self,
        request: &CreateMessageRequest,
        on_event: &mut dyn FnMut(StreamEvent),
        cancel_token: Option<Arc<AtomicBool>>,
    ) -> Result<CreateMessageResponse, ProviderError> {
        let mut req = request.clone();
        req.stream = true;
        let body = self.serialize_request(&req)?;

        let reader = with_retry(1, || {
            self.transport
                .post_json_stream_cancellable("/v1/messages", &body, cancel_token.clone())
        })?;

        crate::provider::claude::parse_sse_stream_with_cancel(reader, on_event, cancel_token)
    }

    fn verify_key(&self) -> Result<String, ProviderError> {
        // Foundry uses the same Messages API — send a minimal request
        let test_req = CreateMessageRequest {
            model: "claude-sonnet-4-20250514".to_string(),
            max_tokens: 1,
            system: Vec::new(),
            messages: vec![crate::message::Message::user_text("hi")],
            tools: Vec::new(),
            stream: false,
            thinking: None,
            response_format: None,
        };
        let body = self.serialize_request(&test_req)?;
        let _ = with_retry(1, || self.transport.post_json("/v1/messages", &body))?;
        Ok(format!("Foundry '{}' key valid", self.foundry_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foundry_provider_creates() {
        let provider = FoundryProvider::new("my-foundry", "sk-test-key");
        assert_eq!(provider.foundry_id(), "my-foundry");
    }

    #[test]
    fn foundry_bearer_token_creates() {
        let provider = FoundryProvider::with_bearer_token("my-foundry", "bearer-token");
        assert_eq!(provider.foundry_id(), "my-foundry");
    }

    #[test]
    fn foundry_verify_fails_no_server() {
        let provider = FoundryProvider::new("nonexistent", "sk-fake");
        assert!(provider.verify_key().is_err());
    }

    #[test]
    fn foundry_create_message_fails_no_server() {
        let provider = FoundryProvider::new("nonexistent", "sk-fake");
        let req = CreateMessageRequest {
            model: "claude-sonnet-4-20250514".to_string(),
            max_tokens: 100,
            system: Vec::new(),
            messages: vec![crate::message::Message::user_text("test")],
            tools: Vec::new(),
            stream: false,
            thinking: None,
            response_format: None,
        };
        assert!(provider.create_message(&req).is_err());
    }
}
