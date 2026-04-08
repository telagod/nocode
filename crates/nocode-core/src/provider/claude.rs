use crate::message::{CacheControl, ContentBlock};
use crate::provider::Provider;
use crate::provider::transport::{HttpTransport, SseReader, with_retry};
use crate::provider::types::{
    CreateMessageRequest, CreateMessageResponse, ProviderError, StopReason, StreamDelta,
    StreamEvent, Usage,
};

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const API_VERSION: &str = "2024-06-01";

/// Claude Messages API provider.
pub struct ClaudeProvider {
    transport: HttpTransport,
}

impl ClaudeProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_base_url(DEFAULT_BASE_URL, api_key)
    }

    pub fn with_base_url(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        let key = api_key.into();
        let transport = HttpTransport::new(base_url, &key)
            .with_header("anthropic-version", API_VERSION)
            .with_header("x-api-key", &key);
        Self { transport }
    }

    fn serialize_request(&self, request: &CreateMessageRequest) -> Result<String, ProviderError> {
        serde_json::to_string(request)
            .map_err(|e| ProviderError::non_retryable(format!("Failed to serialize request: {e}")))
    }

    /// Inject cache_control on the last system block and last tool definition.
    /// This enables prompt caching for static content (system prompt + tool schemas).
    fn inject_cache_control(request: &mut CreateMessageRequest) {
        if let Some(last) = request.system.last_mut() {
            last.cache_control = Some(CacheControl::ephemeral());
        }
        if let Some(last) = request.tools.last_mut() {
            last.cache_control = Some(CacheControl::ephemeral());
        }
    }
}

impl Provider for ClaudeProvider {
    fn create_message(
        &self,
        request: &CreateMessageRequest,
    ) -> Result<CreateMessageResponse, ProviderError> {
        let mut req = request.clone();
        req.stream = false;
        Self::inject_cache_control(&mut req);
        let body = self.serialize_request(&req)?;

        let response_text = with_retry(3, || self.transport.post_json("/v1/messages", &body))?;

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
        Self::inject_cache_control(&mut req);
        let body = self.serialize_request(&req)?;

        let reader = with_retry(3, || self.transport.post_json_stream("/v1/messages", &body))?;

        parse_sse_stream(reader, on_event)
    }

    fn verify_key(&self) -> Result<String, ProviderError> {
        // Send a minimal request to verify the key
        let body = serde_json::json!({
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 1,
            "messages": [{"role": "user", "content": [{"type": "text", "text": "hi"}]}]
        });
        match self.transport.post_json("/v1/messages", &body.to_string()) {
            Ok(_) => Ok("Claude API key valid".to_string()),
            Err(e) if e.kind == crate::provider::types::ErrorKind::Auth => {
                Err(ProviderError::with_kind(
                    "Invalid or expired Anthropic API key",
                    crate::provider::types::ErrorKind::Auth,
                ))
            }
            Err(e) => {
                // Any non-auth error means the key is likely valid
                // (could be rate limit, overloaded, etc.)
                if e.kind == crate::provider::types::ErrorKind::InvalidRequest {
                    Ok("Claude API key valid (model may differ)".to_string())
                } else {
                    Ok(format!("Claude API key accepted ({:?})", e.kind))
                }
            }
        }
    }
}

/// Parse an SSE stream from the Claude Messages API into events,
/// accumulating the final `CreateMessageResponse`.
/// Uses `SseReader` for unified SSE parsing with stall detection.
pub(crate) fn parse_sse_stream(
    reader: impl std::io::Read,
    on_event: &mut dyn FnMut(StreamEvent),
) -> Result<CreateMessageResponse, ProviderError> {
    let mut sse = SseReader::new(reader);

    let mut response_id = String::new();
    let mut model = String::new();
    let mut content_blocks: Vec<ContentBlock> = Vec::new();
    let mut stop_reason = StopReason::EndTurn;
    let mut usage = Usage::default();

    // Accumulator for tool_use input JSON fragments
    let mut tool_input_bufs: std::collections::HashMap<u32, String> =
        std::collections::HashMap::new();

    while let Some(frame) = sse.next_frame()? {
        let json: serde_json::Value = match serde_json::from_str(&frame.data) {
            Ok(v) => v,
            Err(_) => continue,
        };

        match frame.event_type.as_str() {
            "message_start" => {
                if let Some(msg) = json.get("message") {
                    response_id = msg["id"].as_str().unwrap_or_default().to_string();
                    model = msg["model"].as_str().unwrap_or_default().to_string();
                    if let Ok(u) = serde_json::from_value::<Usage>(msg["usage"].clone()) {
                        usage = u;
                    }
                }
            }
            "content_block_start" => {
                let index = json["index"].as_u64().unwrap_or(0) as u32;
                let block = &json["content_block"];
                let block_type = block["type"].as_str().unwrap_or("");

                let content_block = match block_type {
                    "text" => ContentBlock::text(block["text"].as_str().unwrap_or("")),
                    "tool_use" => {
                        tool_input_bufs.insert(index, String::new());
                        ContentBlock::tool_use(
                            block["id"].as_str().unwrap_or(""),
                            block["name"].as_str().unwrap_or(""),
                            serde_json::Value::Null,
                        )
                    }
                    "thinking" => ContentBlock::Thinking {
                        thinking: String::new(),
                    },
                    _ => continue,
                };

                while content_blocks.len() <= index as usize {
                    content_blocks.push(ContentBlock::text(""));
                }
                content_blocks[index as usize] = content_block.clone();

                on_event(StreamEvent::ContentBlockStart {
                    index,
                    content_block,
                });
            }
            "content_block_delta" => {
                let index = json["index"].as_u64().unwrap_or(0) as u32;
                let delta = &json["delta"];
                let delta_type = delta["type"].as_str().unwrap_or("");

                let stream_delta = match delta_type {
                    "text_delta" => {
                        let text = delta["text"].as_str().unwrap_or("").to_string();
                        if let Some(ContentBlock::Text { text: t }) =
                            content_blocks.get_mut(index as usize)
                        {
                            t.push_str(&text);
                        }
                        StreamDelta::TextDelta { text }
                    }
                    "input_json_delta" => {
                        let partial = delta["partial_json"].as_str().unwrap_or("").to_string();
                        if let Some(buf) = tool_input_bufs.get_mut(&index) {
                            buf.push_str(&partial);
                        }
                        StreamDelta::InputJsonDelta {
                            partial_json: partial,
                        }
                    }
                    "thinking_delta" => {
                        let thinking = delta["thinking"].as_str().unwrap_or("").to_string();
                        if let Some(ContentBlock::Thinking { thinking: t }) =
                            content_blocks.get_mut(index as usize)
                        {
                            t.push_str(&thinking);
                        }
                        StreamDelta::ThinkingDelta { thinking }
                    }
                    _ => continue,
                };

                on_event(StreamEvent::ContentBlockDelta {
                    index,
                    delta: stream_delta,
                });
            }
            "content_block_stop" => {
                let index = json["index"].as_u64().unwrap_or(0) as u32;

                if let Some(json_buf) = tool_input_bufs.remove(&index)
                    && let Some(ContentBlock::ToolUse { input, .. }) =
                        content_blocks.get_mut(index as usize)
                {
                    *input = serde_json::from_str(&json_buf).unwrap_or(serde_json::Value::Null);
                }

                on_event(StreamEvent::ContentBlockStop { index });
            }
            "message_delta" => {
                let delta = &json["delta"];
                if let Ok(sr) = serde_json::from_value::<StopReason>(delta["stop_reason"].clone()) {
                    stop_reason = sr;
                }
                if let Ok(u) = serde_json::from_value::<Usage>(json["usage"].clone()) {
                    usage.output_tokens = u.output_tokens;
                }
                on_event(StreamEvent::MessageDelta {
                    stop_reason,
                    usage: usage.clone(),
                });
            }
            "message_stop" => {
                on_event(StreamEvent::MessageStop);
            }
            "ping" => {
                on_event(StreamEvent::Ping);
            }
            _ => {}
        }
    }

    Ok(CreateMessageResponse {
        id: response_id,
        content: content_blocks,
        stop_reason,
        usage,
        model,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn make_sse(events: &[(&str, &str)]) -> String {
        let mut out = String::new();
        for (event_type, data) in events {
            out.push_str(&format!("event: {event_type}\ndata: {data}\n\n"));
        }
        out
    }

    #[test]
    fn parse_simple_text_stream() {
        let sse = make_sse(&[
            (
                "message_start",
                r#"{"message":{"id":"msg-1","model":"claude-opus-4-20250514","usage":{"input_tokens":10,"output_tokens":0}}}"#,
            ),
            (
                "content_block_start",
                r#"{"index":0,"content_block":{"type":"text","text":""}}"#,
            ),
            (
                "content_block_delta",
                r#"{"index":0,"delta":{"type":"text_delta","text":"Hello"}}"#,
            ),
            (
                "content_block_delta",
                r#"{"index":0,"delta":{"type":"text_delta","text":" world"}}"#,
            ),
            ("content_block_stop", r#"{"index":0}"#),
            (
                "message_delta",
                r#"{"delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":5}}"#,
            ),
            ("message_stop", "{}"),
        ]);

        let mut events = Vec::new();
        let resp = parse_sse_stream(Cursor::new(sse.as_bytes()), &mut |e| events.push(e)).unwrap();

        assert_eq!(resp.id, "msg-1");
        assert_eq!(resp.model, "claude-opus-4-20250514");
        assert_eq!(resp.stop_reason, StopReason::EndTurn);
        assert_eq!(resp.usage.input_tokens, 10);
        assert_eq!(resp.usage.output_tokens, 5);
        assert_eq!(resp.text_content(), "Hello world");

        let delta_count = events
            .iter()
            .filter(|e| matches!(e, StreamEvent::ContentBlockDelta { .. }))
            .count();
        assert_eq!(delta_count, 2);
    }

    #[test]
    fn parse_tool_use_stream() {
        let sse = make_sse(&[
            (
                "message_start",
                r#"{"message":{"id":"msg-2","model":"claude-opus-4-20250514","usage":{"input_tokens":20,"output_tokens":0}}}"#,
            ),
            (
                "content_block_start",
                r#"{"index":0,"content_block":{"type":"tool_use","id":"tu-1","name":"Bash","input":{}}}"#,
            ),
            (
                "content_block_delta",
                r#"{"index":0,"delta":{"type":"input_json_delta","partial_json":"{\"comma"}}"#,
            ),
            (
                "content_block_delta",
                r#"{"index":0,"delta":{"type":"input_json_delta","partial_json":"nd\":\"ls\"}"}}"#,
            ),
            ("content_block_stop", r#"{"index":0}"#),
            (
                "message_delta",
                r#"{"delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":10}}"#,
            ),
            ("message_stop", "{}"),
        ]);

        let mut events = Vec::new();
        let resp = parse_sse_stream(Cursor::new(sse.as_bytes()), &mut |e| events.push(e)).unwrap();

        assert_eq!(resp.stop_reason, StopReason::ToolUse);
        assert!(resp.has_tool_use());
        let tool_uses = resp.tool_uses();
        assert_eq!(tool_uses.len(), 1);
        if let ContentBlock::ToolUse { id, name, .. } = &tool_uses[0] {
            assert_eq!(id, "tu-1");
            assert_eq!(name, "Bash");
        } else {
            panic!("expected ToolUse block");
        }
    }

    #[test]
    fn parse_thinking_stream() {
        let sse = make_sse(&[
            (
                "message_start",
                r#"{"message":{"id":"msg-3","model":"claude-opus-4-20250514","usage":{"input_tokens":5,"output_tokens":0}}}"#,
            ),
            (
                "content_block_start",
                r#"{"index":0,"content_block":{"type":"thinking","thinking":""}}"#,
            ),
            (
                "content_block_delta",
                r#"{"index":0,"delta":{"type":"thinking_delta","thinking":"Let me think..."}}"#,
            ),
            ("content_block_stop", r#"{"index":0}"#),
            (
                "content_block_start",
                r#"{"index":1,"content_block":{"type":"text","text":""}}"#,
            ),
            (
                "content_block_delta",
                r#"{"index":1,"delta":{"type":"text_delta","text":"Done."}}"#,
            ),
            ("content_block_stop", r#"{"index":1}"#),
            (
                "message_delta",
                r#"{"delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":8}}"#,
            ),
            ("message_stop", "{}"),
        ]);

        let mut events = Vec::new();
        let resp = parse_sse_stream(Cursor::new(sse.as_bytes()), &mut |e| events.push(e)).unwrap();

        assert_eq!(resp.text_content(), "Done.");
        let thinking_count = events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    StreamEvent::ContentBlockDelta {
                        delta: StreamDelta::ThinkingDelta { .. },
                        ..
                    }
                )
            })
            .count();
        assert_eq!(thinking_count, 1);
    }

    #[test]
    fn parse_ping_events() {
        let sse = make_sse(&[
            ("ping", "{}"),
            (
                "message_start",
                r#"{"message":{"id":"msg-4","model":"test","usage":{"input_tokens":1,"output_tokens":0}}}"#,
            ),
            ("ping", "{}"),
            (
                "content_block_start",
                r#"{"index":0,"content_block":{"type":"text","text":""}}"#,
            ),
            (
                "content_block_delta",
                r#"{"index":0,"delta":{"type":"text_delta","text":"Hi"}}"#,
            ),
            ("content_block_stop", r#"{"index":0}"#),
            (
                "message_delta",
                r#"{"delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}}"#,
            ),
            ("message_stop", "{}"),
        ]);

        let mut events = Vec::new();
        let resp = parse_sse_stream(Cursor::new(sse.as_bytes()), &mut |e| events.push(e)).unwrap();

        assert_eq!(resp.text_content(), "Hi");
        let ping_count = events
            .iter()
            .filter(|e| matches!(e, StreamEvent::Ping))
            .count();
        assert_eq!(ping_count, 2);
    }

    #[test]
    fn parse_empty_stream() {
        let sse = make_sse(&[
            (
                "message_start",
                r#"{"message":{"id":"msg-5","model":"test","usage":{"input_tokens":0,"output_tokens":0}}}"#,
            ),
            (
                "message_delta",
                r#"{"delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":0}}"#,
            ),
            ("message_stop", "{}"),
        ]);

        let resp = parse_sse_stream(Cursor::new(sse.as_bytes()), &mut |_| {}).unwrap();
        assert_eq!(resp.text_content(), "");
        assert_eq!(resp.stop_reason, StopReason::EndTurn);
    }
}
