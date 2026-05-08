//! Gemini generateContent API provider.

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

pub struct GeminiProvider {
    transport: HttpTransport,
    api_key: String,
}

impl GeminiProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        let key = api_key.into();
        Self {
            transport: HttpTransport::new("https://generativelanguage.googleapis.com", &key),
            api_key: key,
        }
    }

    fn build_request_body(&self, request: &CreateMessageRequest) -> String {
        let contents = self.convert_messages(&request.system, &request.messages);
        let tools = self.convert_tools(request);

        let mut body = json!({
            "contents": contents,
            "generationConfig": {
                "maxOutputTokens": request.max_tokens,
            }
        });

        // System instruction
        let sys_text: String = request
            .system
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if !sys_text.is_empty() {
            body["systemInstruction"] = json!({"parts": [{"text": sys_text}]});
        }

        if !tools.is_empty() {
            body["tools"] = json!([{"functionDeclarations": tools}]);
        }

        body.to_string()
    }

    fn convert_messages(
        &self,
        _system: &[crate::message::SystemBlock],
        messages: &[Message],
    ) -> Vec<Value> {
        let mut result = Vec::new();

        for msg in messages {
            let role = match msg.role {
                Role::User => "user",
                Role::Assistant => "model",
            };

            let mut parts: Vec<Value> = Vec::new();

            for block in &msg.content {
                match block {
                    ContentBlock::Text { text } => {
                        parts.push(json!({"text": text}));
                    }
                    ContentBlock::ToolUse { name, input, .. } => {
                        parts.push(json!({
                            "functionCall": {"name": name, "args": input}
                        }));
                    }
                    ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        ..
                    } => {
                        parts.push(json!({
                            "functionResponse": {
                                "name": tool_use_id,
                                "response": {"content": content}
                            }
                        }));
                    }
                    ContentBlock::Thinking { .. } => {}
                    ContentBlock::Image { source } => {
                        parts.push(json!({
                            "inlineData": {
                                "mimeType": source.media_type,
                                "data": source.data
                            }
                        }));
                    }
                }
            }

            if !parts.is_empty() {
                result.push(json!({"role": role, "parts": parts}));
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

        let candidate = &json["candidates"][0];
        let parts = candidate["content"]["parts"].as_array();

        let mut content = Vec::new();
        if let Some(parts) = parts {
            for part in parts {
                if let Some(text) = part["text"].as_str() {
                    content.push(ContentBlock::Text {
                        text: text.to_string(),
                    });
                }
                if let Some(fc) = part.get("functionCall") {
                    let name = fc["name"].as_str().unwrap_or("").to_string();
                    let args = fc.get("args").cloned().unwrap_or(Value::Null);
                    let id = format!("gemini-{name}");
                    content.push(ContentBlock::ToolUse {
                        id,
                        name,
                        input: args,
                    });
                }
            }
        }

        let stop_reason = match candidate["finishReason"].as_str() {
            Some("STOP") => StopReason::EndTurn,
            Some("MAX_TOKENS") => StopReason::MaxTokens,
            Some("TOOL_USE") => StopReason::ToolUse,
            _ => {
                // If we have function calls, it's a tool use stop
                if content
                    .iter()
                    .any(|b| matches!(b, ContentBlock::ToolUse { .. }))
                {
                    StopReason::ToolUse
                } else {
                    StopReason::EndTurn
                }
            }
        };

        let usage = Usage {
            input_tokens: json["usageMetadata"]["promptTokenCount"]
                .as_u64()
                .unwrap_or(0),
            output_tokens: json["usageMetadata"]["candidatesTokenCount"]
                .as_u64()
                .unwrap_or(0),
            ..Default::default()
        };

        Ok(CreateMessageResponse {
            id: String::new(),
            model: String::new(),
            content,
            stop_reason,
            usage,
        })
    }

    fn endpoint(&self, model: &str, stream: bool) -> String {
        let method = if stream {
            "streamGenerateContent"
        } else {
            "generateContent"
        };
        format!("/v1beta/models/{model}:{method}?key={}", self.api_key)
    }
}

impl Provider for GeminiProvider {
    fn create_message(
        &self,
        request: &CreateMessageRequest,
    ) -> Result<CreateMessageResponse, ProviderError> {
        let body = self.build_request_body(request);
        let path = self.endpoint(&request.model, false);
        let response = self.transport.post_json(&path, &body)?;
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
        let body = self.build_request_body(request);
        let path = self.endpoint(&request.model, true);
        let reader =
            self.transport
                .post_json_stream_cancellable(&path, &body, cancel_token.clone())?;
        let buf = BufReader::new(reader);

        let mut all_content: Vec<ContentBlock> = Vec::new();
        let mut accumulated_text = String::new();
        let mut stop_reason = StopReason::EndTurn;
        let mut usage = Usage::default();

        // Gemini streams JSON array chunks, each is a complete candidate response
        let mut json_buf = String::new();
        for line in buf.lines() {
            let line = line.map_err(map_stream_read_error)?;

            if cancel_token
                .as_ref()
                .is_some_and(|token| token.load(std::sync::atomic::Ordering::Relaxed))
            {
                return Err(ProviderError::non_retryable("Cancelled by user"));
            }
            let trimmed = line.trim();

            // Skip array delimiters
            if trimmed == "[" || trimmed == "]" || trimmed == "," || trimmed.is_empty() {
                continue;
            }

            json_buf.push_str(trimmed);

            // Try to parse accumulated JSON
            let json: Value = match serde_json::from_str(&json_buf) {
                Ok(v) => {
                    json_buf.clear();
                    v
                }
                Err(_) => continue,
            };

            // Extract text deltas
            if let Some(parts) = json["candidates"][0]["content"]["parts"].as_array() {
                for part in parts {
                    if let Some(text) = part["text"].as_str() {
                        on_event(StreamEvent::ContentBlockDelta {
                            index: 0,
                            delta: StreamDelta::TextDelta {
                                text: text.to_string(),
                            },
                        });
                        accumulated_text.push_str(text);
                    }
                    if let Some(fc) = part.get("functionCall") {
                        let name = fc["name"].as_str().unwrap_or("").to_string();
                        let args = fc.get("args").cloned().unwrap_or(Value::Null);
                        let id = format!("gemini-{name}");
                        all_content.push(ContentBlock::ToolUse {
                            id,
                            name,
                            input: args,
                        });
                        stop_reason = StopReason::ToolUse;
                    }
                }
            }

            // Usage metadata
            if let Some(u) = json.get("usageMetadata") {
                usage.input_tokens = u["promptTokenCount"].as_u64().unwrap_or(usage.input_tokens);
                usage.output_tokens = u["candidatesTokenCount"]
                    .as_u64()
                    .unwrap_or(usage.output_tokens);
            }

            if let Some(reason) = json["candidates"][0]["finishReason"].as_str() {
                stop_reason = match reason {
                    "MAX_TOKENS" => StopReason::MaxTokens,
                    _ => stop_reason,
                };
            }
        }

        if !accumulated_text.is_empty() {
            all_content.insert(
                0,
                ContentBlock::Text {
                    text: accumulated_text,
                },
            );
        }

        Ok(CreateMessageResponse {
            id: String::new(),
            model: request.model.clone(),
            content: all_content,
            stop_reason,
            usage,
        })
    }

    fn verify_key(&self) -> Result<String, ProviderError> {
        let path = format!("/v1beta/models?key={}", self.api_key);
        match self.transport.get(&path) {
            Ok(body) => {
                let json: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                let count = json["models"].as_array().map_or(0, Vec::len);
                Ok(format!("Gemini API key valid ({count} models available)"))
            }
            Err(e) if e.kind == crate::provider::types::ErrorKind::Auth => {
                Err(ProviderError::with_kind(
                    "Invalid or expired Gemini API key",
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
            model: "gemini-2.5-pro".to_string(),
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
    fn parse_text_response() {
        let provider = GeminiProvider::new("test-key");
        let body = r#"{
            "candidates": [{
                "content": {"parts": [{"text": "Hi there!"}]},
                "finishReason": "STOP"
            }],
            "usageMetadata": {"promptTokenCount": 10, "candidatesTokenCount": 5}
        }"#;
        let resp = provider.parse_response(body).unwrap();
        assert_eq!(resp.text_content(), "Hi there!");
        assert_eq!(resp.stop_reason, StopReason::EndTurn);
        assert_eq!(resp.usage.input_tokens, 10);
        assert_eq!(resp.usage.output_tokens, 5);
    }

    #[test]
    fn parse_function_call_response() {
        let provider = GeminiProvider::new("test-key");
        // Gemini omits finishReason or uses non-STOP when returning function calls
        let body = r#"{
            "candidates": [{
                "content": {"parts": [{"functionCall": {"name": "Bash", "args": {"command": "ls"}}}]}
            }],
            "usageMetadata": {"promptTokenCount": 15, "candidatesTokenCount": 8}
        }"#;
        let resp = provider.parse_response(body).unwrap();
        assert_eq!(resp.stop_reason, StopReason::ToolUse);
        let tools = resp.tool_uses();
        assert_eq!(tools.len(), 1);
        if let ContentBlock::ToolUse { name, id, .. } = &tools[0] {
            assert_eq!(name, "Bash");
            assert_eq!(id, "gemini-Bash");
        }
    }

    #[test]
    fn convert_messages_format() {
        let provider = GeminiProvider::new("test-key");
        let req = make_request();
        let contents = provider.convert_messages(&req.system, &req.messages);
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0]["role"], "user");
        assert_eq!(contents[0]["parts"][0]["text"], "Hello");
    }

    #[test]
    fn convert_tools_format() {
        let provider = GeminiProvider::new("test-key");
        let mut req = make_request();
        req.tools.push(crate::provider::types::ToolDefinition {
            name: "Bash".to_string(),
            description: "Run a command".to_string(),
            input_schema: json!({"type": "object"}),
            cache_control: None,
        });
        let tools = provider.convert_tools(&req);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "Bash");
    }

    #[test]
    fn build_request_body_includes_system() {
        let provider = GeminiProvider::new("test-key");
        let req = make_request();
        let body = provider.build_request_body(&req);
        let parsed: Value = serde_json::from_str(&body).unwrap();
        assert!(parsed.get("systemInstruction").is_some());
        assert_eq!(
            parsed["systemInstruction"]["parts"][0]["text"],
            "You are helpful."
        );
    }

    #[test]
    fn endpoint_format() {
        let provider = GeminiProvider::new("my-key");
        let ep = provider.endpoint("gemini-2.5-pro", false);
        assert!(ep.contains("gemini-2.5-pro"));
        assert!(ep.contains("generateContent"));
        assert!(ep.contains("key=my-key"));

        let ep_stream = provider.endpoint("gemini-2.5-pro", true);
        assert!(ep_stream.contains("streamGenerateContent"));
    }
}
