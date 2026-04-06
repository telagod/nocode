use serde::{Deserialize, Serialize};
use serde_json::json;

/// A single mock scenario: when a request body contains `trigger`, return `response`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockScenario {
    pub name: String,
    pub trigger: String,
    pub response: MockResponse,
}

/// The HTTP-like response a scenario produces.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockResponse {
    pub status: u16,
    pub body: serde_json::Value,
}

/// A request captured by the mock service for later inspection.
#[derive(Debug, Clone)]
pub struct CapturedRequest {
    pub method: String,
    pub path: String,
    pub body: String,
    pub matched_scenario: Option<String>,
}

/// Deterministic Anthropic-compatible mock service for offline integration / parity tests.
pub struct MockAnthropicService {
    scenarios: Vec<MockScenario>,
    captured: Vec<CapturedRequest>,
}

impl Default for MockAnthropicService {
    fn default() -> Self {
        Self::new()
    }
}

impl MockAnthropicService {
    pub fn new() -> Self {
        Self {
            scenarios: Vec::new(),
            captured: Vec::new(),
        }
    }

    pub fn with_default_scenarios() -> Self {
        let mut svc = Self::new();
        for s in default_scenarios() {
            svc.scenarios.push(s);
        }
        svc
    }

    pub fn add_scenario(&mut self, scenario: MockScenario) {
        self.scenarios.push(scenario);
    }

    pub fn handle_request(&mut self, method: &str, path: &str, body: &str) -> MockResponse {
        let matched = self.scenarios.iter().find(|s| body.contains(&s.trigger));
        let matched_name = matched.map(|s| s.name.clone());
        let response = match matched {
            Some(s) => s.response.clone(),
            None => default_response(),
        };
        self.captured.push(CapturedRequest {
            method: method.to_string(),
            path: path.to_string(),
            body: body.to_string(),
            matched_scenario: matched_name,
        });
        response
    }

    pub fn captured_requests(&self) -> &[CapturedRequest] {
        &self.captured
    }

    pub fn clear_captured(&mut self) {
        self.captured.clear();
    }
}

fn default_response() -> MockResponse {
    MockResponse {
        status: 200,
        body: msg_envelope(
            "default",
            vec![json!({"type": "text", "text": "no scenario matched"})],
            None,
        ),
    }
}

fn msg_envelope(
    id_suffix: &str,
    content: Vec<serde_json::Value>,
    usage: Option<serde_json::Value>,
) -> serde_json::Value {
    let usage = usage.unwrap_or_else(|| json!({"input_tokens": 100, "output_tokens": 50}));
    json!({
        "id": format!("msg-mock-{id_suffix}"),
        "type": "message",
        "role": "assistant",
        "content": content,
        "model": "mock-model",
        "stop_reason": "end_turn",
        "usage": usage
    })
}

fn tool_use_block(id: &str, name: &str, input: serde_json::Value) -> serde_json::Value {
    json!({"type": "tool_use", "id": id, "name": name, "input": input})
}

fn default_scenarios() -> Vec<MockScenario> {
    vec![
        MockScenario {
            name: "streaming_text".into(),
            trigger: "streaming_text".into(),
            response: MockResponse {
                status: 200,
                body: msg_envelope(
                    "streaming_text",
                    vec![json!({"type": "text", "text": "Hello from mock stream."})],
                    None,
                ),
            },
        },
        MockScenario {
            name: "read_file_roundtrip".into(),
            trigger: "read_file".into(),
            response: MockResponse {
                status: 200,
                body: msg_envelope(
                    "read_file_roundtrip",
                    vec![tool_use_block(
                        "tu-read-1",
                        "Read",
                        json!({"file_path": "/tmp/test.txt"}),
                    )],
                    None,
                ),
            },
        },
        MockScenario {
            name: "write_file_allowed".into(),
            trigger: "write_file_allowed".into(),
            response: MockResponse {
                status: 200,
                body: msg_envelope(
                    "write_file_allowed",
                    vec![tool_use_block(
                        "tu-write-1",
                        "Write",
                        json!({"file_path": "/tmp/out.txt", "content": "hello"}),
                    )],
                    None,
                ),
            },
        },
        MockScenario {
            name: "write_file_denied".into(),
            trigger: "write_file_denied".into(),
            response: MockResponse {
                status: 403,
                body: msg_envelope(
                    "write_file_denied",
                    vec![
                        json!({"type": "text", "text": "Permission denied: cannot write to protected path."}),
                    ],
                    None,
                ),
            },
        },
        MockScenario {
            name: "grep_chunk_assembly".into(),
            trigger: "grep_chunk".into(),
            response: MockResponse {
                status: 200,
                body: msg_envelope(
                    "grep_chunk_assembly",
                    vec![tool_use_block(
                        "tu-grep-1",
                        "Grep",
                        json!({"pattern": "TODO", "path": "/tmp/src"}),
                    )],
                    None,
                ),
            },
        },
        MockScenario {
            name: "multi_tool_turn".into(),
            trigger: "multi_tool".into(),
            response: MockResponse {
                status: 200,
                body: msg_envelope(
                    "multi_tool_turn",
                    vec![
                        tool_use_block("tu-mt-1", "Read", json!({"file_path": "/tmp/a.txt"})),
                        tool_use_block(
                            "tu-mt-2",
                            "Grep",
                            json!({"pattern": "error", "path": "/tmp/b"}),
                        ),
                        tool_use_block("tu-mt-3", "Bash", json!({"command": "echo ok"})),
                    ],
                    None,
                ),
            },
        },
        MockScenario {
            name: "bash_stdout_roundtrip".into(),
            trigger: "bash_stdout".into(),
            response: MockResponse {
                status: 200,
                body: msg_envelope(
                    "bash_stdout_roundtrip",
                    vec![tool_use_block(
                        "tu-bash-1",
                        "Bash",
                        json!({"command": "echo hello"}),
                    )],
                    None,
                ),
            },
        },
        MockScenario {
            name: "bash_permission_approved".into(),
            trigger: "bash_approved".into(),
            response: MockResponse {
                status: 200,
                body: msg_envelope(
                    "bash_permission_approved",
                    vec![json!({"type": "text", "text": "Bash execution approved."})],
                    None,
                ),
            },
        },
        MockScenario {
            name: "bash_permission_denied".into(),
            trigger: "bash_denied".into(),
            response: MockResponse {
                status: 403,
                body: msg_envelope(
                    "bash_permission_denied",
                    vec![json!({"type": "text", "text": "Bash execution denied by policy."})],
                    None,
                ),
            },
        },
        MockScenario {
            name: "plugin_tool_roundtrip".into(),
            trigger: "plugin_tool".into(),
            response: MockResponse {
                status: 200,
                body: msg_envelope(
                    "plugin_tool_roundtrip",
                    vec![tool_use_block(
                        "tu-plugin-1",
                        "mcp__test__search",
                        json!({"query": "test"}),
                    )],
                    None,
                ),
            },
        },
        MockScenario {
            name: "auto_compact_triggered".into(),
            trigger: "auto_compact".into(),
            response: MockResponse {
                status: 200,
                body: msg_envelope(
                    "auto_compact_triggered",
                    vec![
                        json!({"type": "text", "text": "This is a very large response that should trigger automatic session compaction. ".repeat(50).trim()}),
                    ],
                    Some(json!({"input_tokens": 95000, "output_tokens": 8000})),
                ),
            },
        },
        MockScenario {
            name: "token_cost_reporting".into(),
            trigger: "token_cost".into(),
            response: MockResponse {
                status: 200,
                body: msg_envelope(
                    "token_cost_reporting",
                    vec![json!({"type": "text", "text": "Cost report generated."})],
                    Some(json!({"input_tokens": 2500, "output_tokens": 350})),
                ),
            },
        },
    ]
}

/// Result of running a single parity scenario.
pub struct ParityResult {
    pub scenario: String,
    pub request_count: usize,
    pub response_matched: bool,
    pub captured: Vec<CapturedRequest>,
}

/// Convenience harness that drives `MockAnthropicService` through named scenarios.
pub struct ParityTestRunner {
    service: MockAnthropicService,
}

impl Default for ParityTestRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl ParityTestRunner {
    pub fn new() -> Self {
        Self {
            service: MockAnthropicService::with_default_scenarios(),
        }
    }

    pub fn run_scenario(&mut self, scenario_name: &str) -> Result<ParityResult, String> {
        let exists = self
            .service
            .scenarios
            .iter()
            .any(|s| s.name == scenario_name);
        if !exists {
            return Err(format!("scenario not found: {scenario_name}"));
        }
        let trigger = self
            .service
            .scenarios
            .iter()
            .find(|s| s.name == scenario_name)
            .unwrap()
            .trigger
            .clone();

        self.service.clear_captured();
        let body = format!(r#"{{"messages":[{{"role":"user","content":"{trigger}"}}]}}"#);
        let resp = self.service.handle_request("POST", "/v1/messages", &body);
        let matched = resp.body["id"]
            .as_str()
            .map(|id| id.contains(scenario_name))
            .unwrap_or(false);

        Ok(ParityResult {
            scenario: scenario_name.to_string(),
            request_count: self.service.captured_requests().len(),
            response_matched: matched,
            captured: self.service.captured_requests().to_vec(),
        })
    }

    pub fn assert_scenario_passed(&self, result: &ParityResult) {
        assert!(
            result.response_matched,
            "scenario '{}' did not match expected response",
            result.scenario
        );
        assert!(
            result.request_count > 0,
            "scenario '{}' produced no captured requests",
            result.scenario
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_scenarios_count() {
        let svc = MockAnthropicService::with_default_scenarios();
        assert_eq!(svc.scenarios.len(), 12);
    }

    #[test]
    fn handle_request_matches_trigger() {
        let mut svc = MockAnthropicService::with_default_scenarios();
        let resp = svc.handle_request("POST", "/v1/messages", r#"{"content":"streaming_text"}"#);
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body["id"], "msg-mock-streaming_text");
    }

    #[test]
    fn handle_request_captures_request() {
        let mut svc = MockAnthropicService::with_default_scenarios();
        svc.handle_request("POST", "/v1/messages", "streaming_text");
        assert_eq!(svc.captured_requests().len(), 1);
        let cap = &svc.captured_requests()[0];
        assert_eq!(cap.method, "POST");
        assert_eq!(cap.path, "/v1/messages");
        assert_eq!(cap.matched_scenario.as_deref(), Some("streaming_text"));
    }

    #[test]
    fn unmatched_request_returns_default() {
        let mut svc = MockAnthropicService::with_default_scenarios();
        let resp = svc.handle_request("POST", "/v1/messages", "no_match_here");
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body["id"], "msg-mock-default");
        assert_eq!(svc.captured_requests()[0].matched_scenario, None);
    }

    #[test]
    fn streaming_text_scenario() {
        let mut svc = MockAnthropicService::with_default_scenarios();
        let resp = svc.handle_request("POST", "/v1/messages", "streaming_text");
        let content = &resp.body["content"][0];
        assert_eq!(content["type"], "text");
        assert_eq!(content["text"], "Hello from mock stream.");
    }

    #[test]
    fn read_file_roundtrip_scenario() {
        let mut svc = MockAnthropicService::with_default_scenarios();
        let resp = svc.handle_request("POST", "/v1/messages", "read_file");
        let tool = &resp.body["content"][0];
        assert_eq!(tool["type"], "tool_use");
        assert_eq!(tool["name"], "Read");
        assert_eq!(tool["input"]["file_path"], "/tmp/test.txt");
    }

    #[test]
    fn multi_tool_turn_scenario() {
        let mut svc = MockAnthropicService::with_default_scenarios();
        let resp = svc.handle_request("POST", "/v1/messages", "multi_tool");
        let content = resp.body["content"].as_array().unwrap();
        assert_eq!(content.len(), 3);
        assert_eq!(content[0]["name"], "Read");
        assert_eq!(content[1]["name"], "Grep");
        assert_eq!(content[2]["name"], "Bash");
    }

    #[test]
    fn parity_runner_basic() {
        let mut runner = ParityTestRunner::new();
        let result = runner.run_scenario("streaming_text").unwrap();
        runner.assert_scenario_passed(&result);
        assert_eq!(result.request_count, 1);
        assert!(result.response_matched);

        let err = runner.run_scenario("nonexistent");
        assert!(err.is_err());
    }
}
