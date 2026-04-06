use nocode_core::{
    DefaultToolExecutor, ToolCallInput, ToolExecutionRequest, ToolExecutor, ToolRegistry,
};
use serde_json::{Value, json};
use std::io::{self, BufRead, BufReader, Write};

const SERVER_NAME: &str = "nocode";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
const PROTOCOL_VERSION: &str = "2024-11-05";

const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// Run the MCP JSON-RPC stdio server. Reads one JSON object per line from stdin,
/// writes one JSON object per line to stdout. Blocks until stdin is closed.
pub fn run_mcp_server(cwd: String) {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let reader = BufReader::new(stdin.lock());
    let mut writer = stdout.lock();
    let executor = DefaultToolExecutor::new(&cwd);

    process_lines(reader, &mut writer, &executor);
}

fn process_lines(reader: impl BufRead, writer: &mut impl Write, executor: &impl ToolExecutor) {
    for line_result in reader.lines() {
        let line = match line_result {
            Ok(l) => l,
            Err(_) => break,
        };
        let trimmed = line.trim().to_string();
        if trimmed.is_empty() {
            continue;
        }

        let parsed: Value = match serde_json::from_str(&trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Notifications have no "id" field — no response needed.
        let id = match parsed.get("id") {
            Some(id) => id.clone(),
            None => continue,
        };

        let method = parsed.get("method").and_then(Value::as_str).unwrap_or("");
        let params = parsed.get("params").cloned().unwrap_or(json!({}));

        let response = dispatch(method, &params, &id, executor);
        let _ = write_response(writer, &response);
    }
}

fn dispatch(method: &str, params: &Value, id: &Value, executor: &impl ToolExecutor) -> Value {
    match method {
        "initialize" => handle_initialize(id),
        "tools/list" => handle_tools_list(id),
        "tools/call" => handle_tools_call(id, params, executor),
        _ => error_response(
            id.clone(),
            METHOD_NOT_FOUND,
            &format!("unknown method: {method}"),
        ),
    }
}

fn handle_initialize(id: &Value) -> Value {
    success_response(
        id.clone(),
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": SERVER_NAME,
                "version": SERVER_VERSION
            }
        }),
    )
}

fn handle_tools_list(id: &Value) -> Value {
    let registry = ToolRegistry::default();
    let tools: Vec<Value> = registry
        .base_tools
        .iter()
        .filter_map(|tool| {
            let schema = nocode_core::get_tool_schema(&tool.name)?;
            Some(json!({
                "name": tool.name,
                "description": format!("{} tool", tool.name),
                "inputSchema": schema
            }))
        })
        .collect();

    success_response(id.clone(), json!({ "tools": tools }))
}

fn handle_tools_call(id: &Value, params: &Value, executor: &impl ToolExecutor) -> Value {
    let tool_name = match params.get("name").and_then(Value::as_str) {
        Some(n) => n,
        None => {
            return error_response(id.clone(), INVALID_PARAMS, "missing required param: name");
        }
    };

    let arguments = params
        .get("arguments")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let mut call = ToolCallInput::new(tool_name, "mcp-call-1");
    for (key, value) in &arguments {
        let str_value = match value {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        call = call.with_argument(key, str_value);
    }

    let trace = executor.execute(ToolExecutionRequest::allowed(call));

    let (text, is_error) = match &trace.result {
        nocode_core::ToolCallResult::Completed { output, .. } => (output.summary.clone(), false),
        nocode_core::ToolCallResult::Failed { error, .. } => (error.clone(), true),
        nocode_core::ToolCallResult::Denied { reason, .. } => (reason.clone(), true),
    };

    success_response(
        id.clone(),
        json!({
            "content": [{ "type": "text", "text": text }],
            "isError": is_error
        }),
    )
}

fn success_response(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
        },
    })
}

fn write_response(writer: &mut impl Write, response: &Value) -> io::Result<()> {
    let line = serde_json::to_string(response).expect("response serialization should not fail");
    writeln!(writer, "{line}")?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocode_core::{
        ToolCallOutput, ToolExecutionRequest, ToolExecutionTrace, ToolExecutor,
        ToolPermissionDecision,
    };

    /// A mock executor that always returns a fixed success message.
    #[derive(Debug)]
    struct MockToolExecutor;

    impl ToolExecutor for MockToolExecutor {
        fn execute(&self, request: ToolExecutionRequest) -> ToolExecutionTrace {
            ToolExecutionTrace {
                progress_updates: Vec::new(),
                result: ToolPermissionDecision::allow(false).settle(
                    request.call,
                    ToolCallOutput {
                        summary: String::from("mock tool executed"),
                        generated_messages: Vec::new(),
                        context_label: None,
                        progress_updates: Vec::new(),
                    },
                ),
                permission_denial: None,
            }
        }
    }

    fn parse_response(output: &[u8]) -> Value {
        let text = String::from_utf8_lossy(output);
        let line = text.lines().next().expect("should have at least one line");
        serde_json::from_str(line).expect("should parse as JSON")
    }

    fn run_single_request(request: &Value) -> Value {
        let input = format!("{}\n", serde_json::to_string(request).unwrap());
        let mut output = Vec::new();
        let executor = MockToolExecutor;
        process_lines(input.as_bytes(), &mut output, &executor);
        parse_response(&output)
    }

    #[test]
    fn initialize_returns_server_info() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "0.1.0" }
            }
        });

        let response = run_single_request(&request);

        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 1);
        let result = &response["result"];
        assert_eq!(result["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(result["serverInfo"]["name"], SERVER_NAME);
        assert_eq!(result["serverInfo"]["version"], SERVER_VERSION);
        assert!(result["capabilities"]["tools"].is_object());
    }

    #[test]
    fn tools_list_returns_tool_array() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        });

        let response = run_single_request(&request);

        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 2);
        let tools = response["result"]["tools"]
            .as_array()
            .expect("tools should be an array");
        assert!(!tools.is_empty());

        // Every tool should have name, description, and inputSchema.
        for tool in tools {
            assert!(tool["name"].is_string());
            assert!(tool["description"].is_string());
            assert!(tool["inputSchema"].is_object());
        }

        // Verify a known tool is present.
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        assert!(names.contains(&"Read"));
        assert!(names.contains(&"Bash"));
        assert!(names.contains(&"Grep"));
    }

    #[test]
    fn tools_call_executes_and_returns_result() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "Read",
                "arguments": { "file_path": "/tmp/test.txt" }
            }
        });

        let response = run_single_request(&request);

        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 3);
        let result = &response["result"];
        let content = result["content"]
            .as_array()
            .expect("content should be an array");
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "mock tool executed");
        assert_eq!(result["isError"], false);
    }

    #[test]
    fn tools_call_without_name_returns_error() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {}
        });

        let response = run_single_request(&request);

        assert_eq!(response["error"]["code"], INVALID_PARAMS);
        assert!(
            response["error"]["message"]
                .as_str()
                .unwrap()
                .contains("name")
        );
    }

    #[test]
    fn unknown_method_returns_error() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "bogus/method",
            "params": {}
        });

        let response = run_single_request(&request);

        assert_eq!(response["error"]["code"], METHOD_NOT_FOUND);
    }

    #[test]
    fn notification_without_id_produces_no_output() {
        let notification = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        });

        let input = format!("{}\n", serde_json::to_string(&notification).unwrap());
        let mut output = Vec::new();
        let executor = MockToolExecutor;
        process_lines(input.as_bytes(), &mut output, &executor);

        assert!(output.is_empty(), "notifications should produce no output");
    }
}
