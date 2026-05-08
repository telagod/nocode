//! OpenAI Responses API provider.
//!
//! Implements the newer `/v1/responses` endpoint with its distinct
//! request/response format (input items, function_call output items,
//! named SSE events).

use crate::message::{ContentBlock, Message, Role};
use crate::provider::Provider;
use crate::provider::transport::{HttpTransport, map_stream_read_error};
use crate::provider::types::{
    CreateMessageRequest, CreateMessageResponse, ProviderError, StopReason, StreamDelta,
    StreamEvent, Usage,
};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader};
use std::sync::{Arc, atomic::AtomicBool};

pub struct OpenAiResponsesProvider {
    transport: HttpTransport,
}

impl OpenAiResponsesProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            transport: HttpTransport::new("https://api.openai.com", api_key),
        }
    }

    pub fn with_base_url(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            transport: HttpTransport::new(base_url, api_key),
        }
    }

    fn build_request_body(&self, request: &CreateMessageRequest, stream: bool) -> String {
        let input = self.convert_input(&request.system, &request.messages);
        let tools = self.convert_tools(request);

        // Extract instructions from system blocks
        let instructions: String = request
            .system
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        let mut body = json!({
            "model": request.model,
            "input": input,
            "stream": stream,
            "store": false,
        });

        if !instructions.is_empty() {
            body["instructions"] = Value::String(instructions);
        }
        if request.max_tokens > 0 {
            body["max_output_tokens"] = json!(request.max_tokens);
        }
        if !tools.is_empty() {
            body["tools"] = Value::Array(tools);
        }

        body.to_string()
    }

    /// Convert system blocks + messages into Responses API `input` array.
    fn convert_input(
        &self,
        _system: &[crate::message::SystemBlock],
        messages: &[Message],
    ) -> Vec<Value> {
        let mut input = Vec::new();

        for msg in messages {
            let mut text_parts: Vec<String> = Vec::new();
            let mut function_calls: Vec<Value> = Vec::new();
            let mut function_results: Vec<Value> = Vec::new();
            let mut image_parts: Vec<Value> = Vec::new();

            for block in &msg.content {
                match block {
                    ContentBlock::Text { text } => {
                        text_parts.push(text.clone());
                    }
                    ContentBlock::ToolUse {
                        id,
                        name,
                        input: args,
                    } => {
                        function_calls.push(json!({
                            "type": "function_call",
                            "call_id": id,
                            "name": name,
                            "arguments": args.to_string()
                        }));
                    }
                    ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        ..
                    } => {
                        function_results.push(json!({
                            "type": "function_call_output",
                            "call_id": tool_use_id,
                            "output": content
                        }));
                    }
                    ContentBlock::Thinking { .. } => {}
                    ContentBlock::Image { source } => {
                        image_parts.push(json!({
                            "type": "input_image",
                            "image_url": format!("data:{};base64,{}", source.media_type, source.data)
                        }));
                    }
                }
            }

            match msg.role {
                Role::User => {
                    // Push function results first (they pair with preceding function_calls)
                    for fr in function_results {
                        input.push(fr);
                    }
                    if !image_parts.is_empty() {
                        // Multi-part content: images + text
                        let mut content_parts = image_parts;
                        if !text_parts.is_empty() {
                            content_parts.push(json!({
                                "type": "input_text",
                                "text": text_parts.join("")
                            }));
                        }
                        input.push(json!({"role": "user", "content": content_parts}));
                    } else if !text_parts.is_empty() {
                        let text = text_parts.join("");
                        input.push(json!({"role": "user", "content": text}));
                    }
                }
                Role::Assistant => {
                    // Push text as message output item
                    if !text_parts.is_empty() {
                        let text = text_parts.join("");
                        input.push(json!({
                            "type": "message",
                            "role": "assistant",
                            "content": [{"type": "output_text", "text": text}]
                        }));
                    }
                    // Push function calls
                    for fc in function_calls {
                        input.push(fc);
                    }
                }
            }
        }

        input
    }

    fn convert_tools(&self, request: &CreateMessageRequest) -> Vec<Value> {
        request
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.input_schema
                })
            })
            .collect()
    }
    fn parse_response(&self, body: &str) -> Result<CreateMessageResponse, ProviderError> {
        let json: Value = serde_json::from_str(body)
            .map_err(|e| ProviderError::non_retryable(format!("Invalid JSON: {e}")))?;

        // Check for API error
        if let Some(err) = json.get("error") {
            let msg = err["message"].as_str().unwrap_or("Unknown error");
            return Err(ProviderError::non_retryable(format!("API error: {msg}")));
        }

        let mut content = Vec::new();
        let mut has_function_call = false;

        if let Some(output) = json["output"].as_array() {
            for item in output {
                match item["type"].as_str() {
                    Some("message") => {
                        if let Some(parts) = item["content"].as_array() {
                            for part in parts {
                                if part["type"].as_str() == Some("output_text")
                                    && let Some(text) = part["text"].as_str()
                                {
                                    content.push(ContentBlock::text(text));
                                }
                            }
                        }
                    }
                    Some("function_call") => {
                        has_function_call = true;
                        let call_id = item["call_id"].as_str().unwrap_or("").to_string();
                        let name = item["name"].as_str().unwrap_or("").to_string();
                        let args_str = item["arguments"].as_str().unwrap_or("{}");
                        let input = serde_json::from_str(args_str).unwrap_or(Value::Null);
                        content.push(ContentBlock::ToolUse {
                            id: call_id,
                            name,
                            input,
                        });
                    }
                    Some("reasoning") => {
                        if let Some(summary) = item["summary"].as_array() {
                            for s in summary {
                                if let Some(text) = s["text"].as_str() {
                                    content.push(ContentBlock::Thinking {
                                        thinking: text.to_string(),
                                    });
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Fallback: use output_text convenience field
        if content.is_empty()
            && let Some(text) = json["output_text"].as_str()
            && !text.is_empty()
        {
            content.push(ContentBlock::text(text));
        }

        let stop_reason = if has_function_call {
            StopReason::ToolUse
        } else {
            match json["status"].as_str() {
                Some("incomplete") => StopReason::MaxTokens,
                _ => StopReason::EndTurn,
            }
        };

        let usage = Usage {
            input_tokens: json["usage"]["input_tokens"].as_u64().unwrap_or(0),
            output_tokens: json["usage"]["output_tokens"].as_u64().unwrap_or(0),
            ..Default::default()
        };

        Ok(CreateMessageResponse {
            id: json["id"].as_str().unwrap_or("").to_string(),
            model: json["model"].as_str().unwrap_or("").to_string(),
            content,
            stop_reason,
            usage,
        })
    }
}
impl Provider for OpenAiResponsesProvider {
    fn create_message(
        &self,
        request: &CreateMessageRequest,
    ) -> Result<CreateMessageResponse, ProviderError> {
        let body = self.build_request_body(request, false);
        let response = self.transport.post_json("/v1/responses", &body)?;
        self.parse_response(&response)
    }

    fn create_message_stream(
        &self,
        request: &CreateMessageRequest,
        on_event: &mut dyn FnMut(StreamEvent),
    ) -> Result<CreateMessageResponse, ProviderError> {
        self.create_message_stream_with_cancel(request, on_event, None)
    }

    fn create_message_stream_with_cancel(
        &self,
        request: &CreateMessageRequest,
        on_event: &mut dyn FnMut(StreamEvent),
        cancel_token: Option<Arc<AtomicBool>>,
    ) -> Result<CreateMessageResponse, ProviderError> {
        let body = self.build_request_body(request, true);
        let reader = self.transport.post_json_stream_cancellable(
            "/v1/responses",
            &body,
            cancel_token.clone(),
        )?;
        let buf = BufReader::new(reader);

        let mut content_blocks: Vec<ContentBlock> = Vec::new();
        let mut fc_bufs: std::collections::HashMap<String, (String, String, String)> =
            std::collections::HashMap::new(); // item_id -> (call_id, name, args_buf)
        let mut stop_reason = StopReason::EndTurn;
        let mut usage = Usage::default();
        let mut has_function_call = false;

        let mut current_event_type = String::new();

        for line in buf.lines() {
            let line = line.map_err(map_stream_read_error)?;

            if cancel_token
                .as_ref()
                .is_some_and(|token| token.load(std::sync::atomic::Ordering::Relaxed))
            {
                return Err(ProviderError::non_retryable("Cancelled by user"));
            }

            // Parse SSE: "event: <type>" and "data: <json>"
            if let Some(event_type) = line.strip_prefix("event: ") {
                current_event_type = event_type.to_string();
                continue;
            }

            if !line.starts_with("data: ") {
                continue;
            }
            let data = &line[6..];

            let json: Value = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(_) => continue,
            };

            match current_event_type.as_str() {
                "response.output_text.delta" => {
                    if let Some(delta) = json["delta"].as_str() {
                        on_event(StreamEvent::ContentBlockDelta {
                            index: 0,
                            delta: StreamDelta::TextDelta {
                                text: delta.to_string(),
                            },
                        });
                        // Accumulate text
                        if let Some(ContentBlock::Text { text: t }) = content_blocks.first_mut() {
                            t.push_str(delta);
                        } else {
                            content_blocks.push(ContentBlock::text(delta));
                        }
                    }
                }
                "response.output_item.added"
                    if json["item"]["type"].as_str() == Some("function_call") =>
                {
                    // Track new function_call items
                    let item_id = json["item"]["id"].as_str().unwrap_or("").to_string();
                    let call_id = json["item"]["call_id"].as_str().unwrap_or("").to_string();
                    let name = json["item"]["name"].as_str().unwrap_or("").to_string();
                    has_function_call = true;
                    fc_bufs.insert(item_id, (call_id, name, String::new()));
                }
                "response.function_call_arguments.delta" => {
                    let item_id = json["item_id"].as_str().unwrap_or("");
                    if let Some(delta) = json["delta"].as_str() {
                        if let Some(entry) = fc_bufs.get_mut(item_id) {
                            entry.2.push_str(delta);
                        }
                        on_event(StreamEvent::ContentBlockDelta {
                            index: 1,
                            delta: StreamDelta::InputJsonDelta {
                                partial_json: delta.to_string(),
                            },
                        });
                    }
                }
                "response.reasoning_summary_text.delta" => {
                    if let Some(delta) = json["delta"].as_str() {
                        on_event(StreamEvent::ContentBlockDelta {
                            index: 0,
                            delta: StreamDelta::ThinkingDelta {
                                thinking: delta.to_string(),
                            },
                        });
                    }
                }
                "response.completed" => {
                    // Extract usage from final response
                    if let Some(resp) = json.get("response") {
                        usage.input_tokens = resp["usage"]["input_tokens"].as_u64().unwrap_or(0);
                        usage.output_tokens = resp["usage"]["output_tokens"].as_u64().unwrap_or(0);
                        if resp["status"].as_str() == Some("incomplete") {
                            stop_reason = StopReason::MaxTokens;
                        }
                    }
                    on_event(StreamEvent::MessageStop);
                }
                "response.failed" => {
                    let msg = json["response"]["error"]["message"]
                        .as_str()
                        .unwrap_or("Unknown error");
                    return Err(ProviderError::non_retryable(format!(
                        "Response failed: {msg}"
                    )));
                }
                _ => {}
            }

            current_event_type.clear();
        }

        // Finalize function calls
        for (_item_id, (call_id, name, args_buf)) in fc_bufs {
            let input = serde_json::from_str(&args_buf).unwrap_or(Value::Null);
            content_blocks.push(ContentBlock::ToolUse {
                id: call_id,
                name,
                input,
            });
        }

        if has_function_call {
            stop_reason = StopReason::ToolUse;
        }

        Ok(CreateMessageResponse {
            id: String::new(),
            model: request.model.clone(),
            content: content_blocks,
            stop_reason,
            usage,
        })
    }

    fn verify_key(&self) -> Result<String, ProviderError> {
        match self.transport.get("/v1/models") {
            Ok(body) => {
                let json: Value = serde_json::from_str(&body).unwrap_or_default();
                let count = json["data"].as_array().map_or(0, Vec::len);
                Ok(format!("OpenAI Responses API key valid ({count} models)"))
            }
            Err(e) if e.kind == crate::provider::types::ErrorKind::Auth => {
                Err(ProviderError::with_kind(
                    "Invalid or expired API key",
                    crate::provider::types::ErrorKind::Auth,
                ))
            }
            Err(e) => Err(e),
        }
    }
}
