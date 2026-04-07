//! Gemini generateContent API provider.

use crate::message::{ContentBlock, Message, Role};
use crate::provider::Provider;
use crate::provider::transport::HttpTransport;
use crate::provider::types::{
    CreateMessageRequest, CreateMessageResponse, ProviderError, StopReason, StreamDelta,
    StreamEvent, Usage,
};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader};

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
        let body = self.build_request_body(request);
        let path = self.endpoint(&request.model, true);
        let reader = self.transport.post_json_stream(&path, &body)?;
        let buf = BufReader::new(reader);

        let mut all_content: Vec<ContentBlock> = Vec::new();
        let mut accumulated_text = String::new();
        let mut stop_reason = StopReason::EndTurn;
        let mut usage = Usage::default();

        // Gemini streams JSON array chunks, each is a complete candidate response
        let mut json_buf = String::new();
        for line in buf.lines() {
            let line = line.map_err(|e| ProviderError::retryable(format!("Stream error: {e}")))?;
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
}
