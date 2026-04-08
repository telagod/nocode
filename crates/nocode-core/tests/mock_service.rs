//! Integration test: MockAnthropicService — deterministic provider scenarios.

use nocode_core::message::{ContentBlock, Message};
use nocode_core::provider::Provider;
use nocode_core::provider::types::{
    CreateMessageRequest, CreateMessageResponse, ProviderError, StopReason, StreamEvent, Usage,
};

/// Mock provider with deterministic scenarios and request capture.
struct MockAnthropicService {
    scenario: MockScenario,
    captured: std::sync::Mutex<Vec<CapturedRequest>>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct CapturedRequest {
    model: String,
    message_count: usize,
    tool_count: usize,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
enum MockScenario {
    SimpleText,
    ToolUseEcho,
    MultiTurn,
    EmptyResponse,
    MaxTokens,
    AuthError,
    RateLimited,
    Overloaded,
    ThinkingModel,
    MultiToolUse,
    EndTurnAfterTool,
    StreamError,
}

impl MockAnthropicService {
    fn new(scenario: MockScenario) -> Self {
        Self {
            scenario,
            captured: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn captured_requests(&self) -> Vec<CapturedRequest> {
        self.captured.lock().unwrap().clone()
    }
}

impl Provider for MockAnthropicService {
    fn create_message(
        &self,
        request: &CreateMessageRequest,
    ) -> Result<CreateMessageResponse, ProviderError> {
        self.captured.lock().unwrap().push(CapturedRequest {
            model: request.model.clone(),
            message_count: request.messages.len(),
            tool_count: request.tools.len(),
        });

        match self.scenario {
            MockScenario::SimpleText => Ok(CreateMessageResponse {
                id: "msg-mock-1".to_string(),
                content: vec![ContentBlock::text("Hello from mock!")],
                stop_reason: StopReason::EndTurn,
                usage: Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                    ..Default::default()
                },
                model: "mock-model".to_string(),
            }),
            MockScenario::ToolUseEcho => Ok(CreateMessageResponse {
                id: "msg-mock-2".to_string(),
                content: vec![ContentBlock::ToolUse {
                    id: "tu-1".to_string(),
                    name: "Bash".to_string(),
                    input: serde_json::json!({"command": "echo mock"}),
                }],
                stop_reason: StopReason::ToolUse,
                usage: Usage {
                    input_tokens: 15,
                    output_tokens: 10,
                    ..Default::default()
                },
                model: "mock-model".to_string(),
            }),
            MockScenario::EmptyResponse => Ok(CreateMessageResponse {
                id: "msg-mock-3".to_string(),
                content: vec![],
                stop_reason: StopReason::EndTurn,
                usage: Usage::default(),
                model: "mock-model".to_string(),
            }),
            MockScenario::MaxTokens => Ok(CreateMessageResponse {
                id: "msg-mock-4".to_string(),
                content: vec![ContentBlock::text("truncated...")],
                stop_reason: StopReason::MaxTokens,
                usage: Usage {
                    input_tokens: 100,
                    output_tokens: 4096,
                    ..Default::default()
                },
                model: "mock-model".to_string(),
            }),
            MockScenario::AuthError => Err(ProviderError::non_retryable("401 Unauthorized")),
            MockScenario::RateLimited => Err(ProviderError::retryable("429 Too Many Requests")),
            MockScenario::Overloaded => Err(ProviderError::retryable("529 Overloaded")),
            MockScenario::ThinkingModel => Ok(CreateMessageResponse {
                id: "msg-mock-5".to_string(),
                content: vec![
                    ContentBlock::Thinking {
                        thinking: "Let me think...".to_string(),
                    },
                    ContentBlock::text("After thinking, here's my answer."),
                ],
                stop_reason: StopReason::EndTurn,
                usage: Usage {
                    input_tokens: 20,
                    output_tokens: 30,
                    ..Default::default()
                },
                model: "mock-model".to_string(),
            }),
            MockScenario::MultiToolUse => Ok(CreateMessageResponse {
                id: "msg-mock-6".to_string(),
                content: vec![
                    ContentBlock::ToolUse {
                        id: "tu-a".to_string(),
                        name: "Bash".to_string(),
                        input: serde_json::json!({"command": "echo first"}),
                    },
                    ContentBlock::ToolUse {
                        id: "tu-b".to_string(),
                        name: "Bash".to_string(),
                        input: serde_json::json!({"command": "echo second"}),
                    },
                ],
                stop_reason: StopReason::ToolUse,
                usage: Usage {
                    input_tokens: 20,
                    output_tokens: 15,
                    ..Default::default()
                },
                model: "mock-model".to_string(),
            }),
            MockScenario::EndTurnAfterTool | MockScenario::MultiTurn => Ok(CreateMessageResponse {
                id: "msg-mock-7".to_string(),
                content: vec![ContentBlock::text("Done.")],
                stop_reason: StopReason::EndTurn,
                usage: Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                    ..Default::default()
                },
                model: "mock-model".to_string(),
            }),
            MockScenario::StreamError => Err(ProviderError::retryable("stream interrupted")),
        }
    }

    fn create_message_stream(
        &self,
        request: &CreateMessageRequest,
        _on_event: &mut dyn FnMut(StreamEvent),
    ) -> Result<CreateMessageResponse, ProviderError> {
        // Delegate to non-streaming for mock
        self.create_message(request)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn simple_text_response() {
    let mock = MockAnthropicService::new(MockScenario::SimpleText);
    let req = make_request("hello");
    let resp = mock.create_message(&req).unwrap();
    assert_eq!(resp.stop_reason, StopReason::EndTurn);
    assert_eq!(resp.text_content(), "Hello from mock!");
    assert_eq!(mock.captured_requests().len(), 1);
}

#[test]
fn tool_use_response() {
    let mock = MockAnthropicService::new(MockScenario::ToolUseEcho);
    let req = make_request("run echo");
    let resp = mock.create_message(&req).unwrap();
    assert_eq!(resp.stop_reason, StopReason::ToolUse);
    assert!(resp.has_tool_use());
    let tools = resp.tool_uses();
    assert_eq!(tools.len(), 1);
}

#[test]
fn auth_error_not_retryable() {
    let mock = MockAnthropicService::new(MockScenario::AuthError);
    let req = make_request("test");
    let err = mock.create_message(&req).unwrap_err();
    assert!(!err.retryable);
    assert!(err.message.contains("401"));
}

#[test]
fn rate_limited_is_retryable() {
    let mock = MockAnthropicService::new(MockScenario::RateLimited);
    let req = make_request("test");
    let err = mock.create_message(&req).unwrap_err();
    assert!(err.retryable);
}

#[test]
fn overloaded_is_retryable() {
    let mock = MockAnthropicService::new(MockScenario::Overloaded);
    let req = make_request("test");
    let err = mock.create_message(&req).unwrap_err();
    assert!(err.retryable);
}

#[test]
fn max_tokens_stops_generation() {
    let mock = MockAnthropicService::new(MockScenario::MaxTokens);
    let req = make_request("long prompt");
    let resp = mock.create_message(&req).unwrap();
    assert_eq!(resp.stop_reason, StopReason::MaxTokens);
    assert!(resp.usage.output_tokens > 0);
}

#[test]
fn thinking_model_includes_thinking_block() {
    let mock = MockAnthropicService::new(MockScenario::ThinkingModel);
    let req = make_request("think about this");
    let resp = mock.create_message(&req).unwrap();
    assert!(
        resp.content
            .iter()
            .any(|b| matches!(b, ContentBlock::Thinking { .. }))
    );
    assert!(!resp.text_content().is_empty());
}

#[test]
fn multi_tool_use() {
    let mock = MockAnthropicService::new(MockScenario::MultiToolUse);
    let req = make_request("do two things");
    let resp = mock.create_message(&req).unwrap();
    assert_eq!(resp.tool_uses().len(), 2);
}

#[test]
fn empty_response_handled() {
    let mock = MockAnthropicService::new(MockScenario::EmptyResponse);
    let req = make_request("empty");
    let resp = mock.create_message(&req).unwrap();
    assert!(resp.content.is_empty());
    assert_eq!(resp.text_content(), "");
}

#[test]
fn captured_requests_track_metadata() {
    let mock = MockAnthropicService::new(MockScenario::SimpleText);
    let _ = mock.create_message(&make_request("first"));
    let _ = mock.create_message(&make_request("second"));
    let captured = mock.captured_requests();
    assert_eq!(captured.len(), 2);
    assert_eq!(captured[0].model, "mock-model");
    assert_eq!(captured[1].message_count, 1);
}

#[test]
fn stream_delegates_to_non_stream() {
    let mock = MockAnthropicService::new(MockScenario::SimpleText);
    let req = make_request("stream test");
    let resp = mock.create_message_stream(&req, &mut |_| {}).unwrap();
    assert_eq!(resp.text_content(), "Hello from mock!");
}

#[test]
fn stream_error_propagates() {
    let mock = MockAnthropicService::new(MockScenario::StreamError);
    let req = make_request("fail");
    let err = mock.create_message_stream(&req, &mut |_| {}).unwrap_err();
    assert!(err.retryable);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_request(prompt: &str) -> CreateMessageRequest {
    CreateMessageRequest {
        model: "mock-model".to_string(),
        max_tokens: 4096,
        system: vec![],
        messages: vec![Message::user_text(prompt)],
        tools: vec![],
        stream: false,
    }
}
