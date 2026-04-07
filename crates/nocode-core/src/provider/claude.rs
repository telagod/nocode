use crate::provider::types::{
    CreateMessageRequest, CreateMessageResponse, ProviderError, StopReason, StreamDelta,
    StreamEvent, Usage,
};
use crate::provider::transport::{HttpTransport, with_retry};
use crate::provider::Provider;
use crate::message::ContentBlock;
use std::io::BufRead;

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const API_VERSION: &str = "2023-06-01";

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
}

impl Provider for ClaudeProvider {
    fn create_message(
        &self,
        request: &CreateMessageRequest,
    ) -> Result<CreateMessageResponse, ProviderError> {
        let mut req = request.clone();
        req.stream = false;
        let body = self.serialize_request(&req)?;

        let response_text = with_retry(3, || {
            self.transport.post_json("/v1/messages", &body)
        })?;

        serde_json::from_str::<CreateMessageResponse>(&response_text).map_err(|e| {
            ProviderError::non_retryable(format!("Failed to parse response: {e}"))
        })
    }

    fn create_message_stream(
        &self,
        request: &CreateMessageRequest,
        on_event: &mut dyn FnMut(StreamEvent),
    ) -> Result<CreateMessageResponse, ProviderError> {
        let mut req = request.clone();
        req.stream = true;
        let body = self.serialize_request(&req)?;

        let reader = with_retry(3, || {
            self.transport.post_json_stream("/v1/messages", &body)
        })?;

        parse_sse_stream(reader, on_event)
    }
}

/// Parse an SSE stream from the Claude Messages API into events,
/// accumulating the final `CreateMessageResponse`.
fn parse_sse_stream(
    reader: impl std::io::Read,
    on_event: &mut dyn FnMut(StreamEvent),
) -> Result<CreateMessageResponse, ProviderError> {
    let buf = std::io::BufReader::new(reader);

    let mut response_id = String::new();
    let mut model = String::new();
    let mut content_blocks: Vec<ContentBlock> = Vec::new();
    let mut stop_reason = StopReason::EndTurn;
    let mut usage = Usage::default();

    // Accumulator for tool_use input JSON fragments
    let mut tool_input_bufs: std::collections::HashMap<u32, String> = std::collections::HashMap::new();

    let mut event_type = String::new();

    for line in buf.lines() {
        let line = line.map_err(|e| ProviderError::retryable(format!("Stream read error: {e}")))?;

        if line.starts_with("event: ") {
            event_type = line[7..].to_string();
            continue;
        }

        if !line.starts_with("data: ") {
            continue;
        }

        let data = &line[6..];
        let json: serde_json::Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => continue,
        };

        match event_type.as_str() {
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

                // Ensure content_blocks vec is large enough
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
                        // Accumulate text
                        if let Some(ContentBlock::Text { text: t }) =
                            content_blocks.get_mut(index as usize)
                        {
                            t.push_str(&text);
                        }
                        StreamDelta::TextDelta { text }
                    }
                    "input_json_delta" => {
                        let partial = delta["partial_json"].as_str().unwrap_or("").to_string();
                        // Accumulate tool input JSON
                        if let Some(buf) = tool_input_bufs.get_mut(&index) {
                            buf.push_str(&partial);
                        }
                        StreamDelta::InputJsonDelta {
                            partial_json: partial,
                        }
                    }
                    "thinking_delta" => {
                        let thinking = delta["thinking"].as_str().unwrap_or("").to_string();
                        if let Some(ContentBlock::Thinking {
                            thinking: t,
                        }) = content_blocks.get_mut(index as usize)
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

                // Finalize tool_use input from accumulated JSON
                if let Some(json_buf) = tool_input_bufs.remove(&index) {
                    if let Some(ContentBlock::ToolUse { input, .. }) =
                        content_blocks.get_mut(index as usize)
                    {
                        *input = serde_json::from_str(&json_buf).unwrap_or(serde_json::Value::Null);
                    }
                }

                on_event(StreamEvent::ContentBlockStop { index });
            }
            "message_delta" => {
                let delta = &json["delta"];
                if let Ok(sr) = serde_json::from_value::<StopReason>(delta["stop_reason"].clone())
                {
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
