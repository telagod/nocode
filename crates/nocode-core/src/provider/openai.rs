//! OpenAI Chat Completions API provider.

use crate::message::{ContentBlock, Message, Role};
use crate::provider::Provider;
use crate::provider::transport::HttpTransport;
use crate::provider::types::{
    CreateMessageRequest, CreateMessageResponse, ProviderError, StopReason, StreamDelta,
    StreamEvent, Usage,
};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader};

pub struct OpenAiProvider {
    transport: HttpTransport,
}

impl OpenAiProvider {
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
        let messages = self.convert_messages(&request.system, &request.messages);
        let tools = self.convert_tools(request);

        let mut body = json!({
            "model": request.model,
            "max_tokens": request.max_tokens,
            "messages": messages,
            "stream": stream,
        });

        if !tools.is_empty() {
            body["tools"] = Value::Array(tools);
        }

        body.to_string()
    }

    fn convert_messages(
        &self,
        system: &[crate::message::SystemBlock],
        messages: &[Message],
    ) -> Vec<Value> {
        let mut result = Vec::new();

        // System messages
        for s in system {
            result.push(json!({"role": "system", "content": s.text}));
        }

        // Conversation messages
        for msg in messages {
            let role = match msg.role {
                Role::User => "user",
                Role::Assistant => "assistant",
            };

            let mut content_parts: Vec<Value> = Vec::new();
            let mut tool_calls: Vec<Value> = Vec::new();
            let mut tool_results: Vec<Value> = Vec::new();

            for block in &msg.content {
                match block {
                    ContentBlock::Text { text } => {
                        content_parts.push(json!({"type": "text", "text": text}));
                    }
                    ContentBlock::ToolUse { id, name, input } => {
                        tool_calls.push(json!({
                            "id": id,
                            "type": "function",
                            "function": {"name": name, "arguments": input.to_string()}
                        }));
                    }
                    ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        ..
                    } => {
                        tool_results.push(json!({
                            "role": "tool",
                            "tool_call_id": tool_use_id,
                            "content": content
                        }));
                    }
                    ContentBlock::Thinking { .. } => {}
                    ContentBlock::Image { source } => {
                        content_parts.push(json!({
                            "type": "image_url",
                            "image_url": {
                                "url": format!("data:{};base64,{}", source.media_type, source.data)
                            }
                        }));
                    }
                }
            }

            if !tool_calls.is_empty() {
                let text = content_parts
                    .iter()
                    .filter_map(|p| p["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("");
                let mut m = json!({"role": "assistant", "tool_calls": tool_calls});
                if !text.is_empty() {
                    m["content"] = Value::String(text);
                }
                result.push(m);
            } else if !tool_results.is_empty() {
                for tr in tool_results {
                    result.push(tr);
                }
            } else {
                let text = content_parts
                    .iter()
                    .filter_map(|p| p["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("");
                result.push(json!({"role": role, "content": text}));
            }
        }

        result
    }

    fn convert_tools(&self, request: &CreateMessageRequest) -> Vec<Value> {
        request
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema
                    }
                })
            })
            .collect()
    }

    fn parse_response(&self, body: &str) -> Result<CreateMessageResponse, ProviderError> {
        let json: Value = serde_json::from_str(body)
            .map_err(|e| ProviderError::non_retryable(format!("Invalid JSON: {e}")))?;

        let choice = &json["choices"][0];
        let message = &choice["message"];

        let mut content = Vec::new();

        if let Some(text) = message["content"].as_str()
            && !text.is_empty()
        {
            content.push(ContentBlock::Text {
                text: text.to_string(),
            });
        }

        if let Some(tool_calls) = message["tool_calls"].as_array() {
            for tc in tool_calls {
                let id = tc["id"].as_str().unwrap_or("").to_string();
                let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
                let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
                let input = serde_json::from_str(args_str).unwrap_or(Value::Null);
                content.push(ContentBlock::ToolUse { id, name, input });
            }
        }

        let stop_reason = match choice["finish_reason"].as_str() {
            Some("tool_calls") => StopReason::ToolUse,
            Some("length") => StopReason::MaxTokens,
            _ => StopReason::EndTurn,
        };

        let usage = Usage {
            input_tokens: json["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
            output_tokens: json["usage"]["completion_tokens"].as_u64().unwrap_or(0),
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

impl Provider for OpenAiProvider {
    fn create_message(
        &self,
        request: &CreateMessageRequest,
    ) -> Result<CreateMessageResponse, ProviderError> {
        let body = self.build_request_body(request, false);
        let response = self.transport.post_json("/v1/chat/completions", &body)?;
        self.parse_response(&response)
    }

    fn create_message_stream(
        &self,
        request: &CreateMessageRequest,
        on_event: &mut dyn FnMut(StreamEvent),
    ) -> Result<CreateMessageResponse, ProviderError> {
        let body = self.build_request_body(request, true);
        let reader = self
            .transport
            .post_json_stream("/v1/chat/completions", &body)?;
        let buf = BufReader::new(reader);

        let mut content_blocks: Vec<ContentBlock> = Vec::new();
        let mut tool_call_bufs: std::collections::HashMap<u32, (String, String, String)> =
            std::collections::HashMap::new(); // index -> (id, name, args_buf)
        let mut stop_reason = StopReason::EndTurn;
        let mut usage = Usage::default();

        for line in buf.lines() {
            let line =
                line.map_err(|e| ProviderError::retryable(format!("Stream read error: {e}")))?;

            if !line.starts_with("data: ") {
                continue;
            }
            let data = &line[6..];
            if data == "[DONE]" {
                break;
            }

            let json: Value = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let delta = &json["choices"][0]["delta"];

            // Text content
            if let Some(text) = delta["content"].as_str()
                && !text.is_empty()
            {
                on_event(StreamEvent::ContentBlockDelta {
                    index: 0,
                    delta: StreamDelta::TextDelta {
                        text: text.to_string(),
                    },
                });
                // Accumulate text
                if let Some(ContentBlock::Text { text: t }) = content_blocks.first_mut() {
                    t.push_str(text);
                } else {
                    content_blocks.push(ContentBlock::Text {
                        text: text.to_string(),
                    });
                }
            }

            // Tool calls
            if let Some(tool_calls) = delta["tool_calls"].as_array() {
                for tc in tool_calls {
                    let idx = tc["index"].as_u64().unwrap_or(0) as u32;
                    let entry = tool_call_bufs.entry(idx).or_insert_with(|| {
                        let id = tc["id"].as_str().unwrap_or("").to_string();
                        let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
                        (id, name, String::new())
                    });
                    if let Some(args) = tc["function"]["arguments"].as_str() {
                        entry.2.push_str(args);
                    }
                }
            }

            // Finish reason
            if let Some(reason) = json["choices"][0]["finish_reason"].as_str() {
                stop_reason = match reason {
                    "tool_calls" => StopReason::ToolUse,
                    "length" => StopReason::MaxTokens,
                    _ => StopReason::EndTurn,
                };
            }

            // Usage (some providers send it in stream)
            if let Some(u) = json.get("usage") {
                usage.input_tokens = u["prompt_tokens"].as_u64().unwrap_or(usage.input_tokens);
                usage.output_tokens = u["completion_tokens"]
                    .as_u64()
                    .unwrap_or(usage.output_tokens);
            }
        }

        // Finalize tool calls
        for (_idx, (id, name, args_buf)) in tool_call_bufs {
            let input = serde_json::from_str(&args_buf).unwrap_or(Value::Null);
            content_blocks.push(ContentBlock::ToolUse { id, name, input });
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
                let json: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                let count = json["data"].as_array().map_or(0, Vec::len);
                Ok(format!("OpenAI API key valid ({count} models available)"))
            }
            Err(e) if e.kind == crate::provider::types::ErrorKind::Auth => {
                Err(ProviderError::with_kind(
                    "Invalid or expired OpenAI API key",
                    crate::provider::types::ErrorKind::Auth,
                ))
            }
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::SystemBlock;

    fn make_request() -> CreateMessageRequest {
        CreateMessageRequest {
            model: "gpt-4o".to_string(),
            max_tokens: 1024,
            system: vec![SystemBlock::text("You are helpful.")],
            messages: vec![Message::user_text("Hello")],
            tools: vec![],
            stream: false,
            thinking: None,
            response_format: None,
        }
    }

    #[test]
    fn parse_non_streaming_response() {
        let provider = OpenAiProvider::new("test-key");
        let body = r#"{
            "id": "chatcmpl-1",
            "model": "gpt-4o",
            "choices": [{
                "message": {"role": "assistant", "content": "Hi there!"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5}
        }"#;
        let resp = provider.parse_response(body).unwrap();
        assert_eq!(resp.text_content(), "Hi there!");
        assert_eq!(resp.stop_reason, StopReason::EndTurn);
        assert_eq!(resp.usage.input_tokens, 10);
        assert_eq!(resp.usage.output_tokens, 5);
    }

    #[test]
    fn parse_tool_call_response() {
        let provider = OpenAiProvider::new("test-key");
        let body = r#"{
            "id": "chatcmpl-2",
            "model": "gpt-4o",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call-1",
                        "type": "function",
                        "function": {"name": "Bash", "arguments": "{\"command\":\"ls\"}"}
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 20, "completion_tokens": 10}
        }"#;
        let resp = provider.parse_response(body).unwrap();
        assert_eq!(resp.stop_reason, StopReason::ToolUse);
        assert!(resp.has_tool_use());
        let tools = resp.tool_uses();
        assert_eq!(tools.len(), 1);
        if let ContentBlock::ToolUse { name, .. } = &tools[0] {
            assert_eq!(name, "Bash");
        }
    }

    #[test]
    fn convert_messages_includes_system() {
        let provider = OpenAiProvider::new("test-key");
        let req = make_request();
        let msgs = provider.convert_messages(&req.system, &req.messages);
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "You are helpful.");
        assert_eq!(msgs[1]["role"], "user");
    }

    #[test]
    fn convert_tools_format() {
        let provider = OpenAiProvider::new("test-key");
        let mut req = make_request();
        req.tools.push(crate::provider::types::ToolDefinition {
            name: "Bash".to_string(),
            description: "Run a command".to_string(),
            input_schema: json!({"type": "object", "properties": {"command": {"type": "string"}}}),
            cache_control: None,
        });
        let tools = provider.convert_tools(&req);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["function"]["name"], "Bash");
    }

    #[test]
    fn build_request_body_valid_json() {
        let provider = OpenAiProvider::new("test-key");
        let req = make_request();
        let body = provider.build_request_body(&req, true);
        let parsed: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["model"], "gpt-4o");
        assert_eq!(parsed["stream"], true);
    }
}
