use nocode_core::{QueryEngine, SubmitMessageOptions};
use serde_json::{Value, json};
use std::io::{self, BufRead, BufReader, Write};

const SERVER_NAME: &str = "nocode";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

// JSON-RPC 2.0 error codes.
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
#[allow(dead_code)]
const INTERNAL_ERROR: i64 = -32603;

/// Run the IDE JSON-RPC stdio server. Blocks until stdin is closed or `shutdown` is received.
pub fn run_ide_server(mut engine: QueryEngine) {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = stdout.lock();

    loop {
        let body = match read_message(&mut reader) {
            Ok(Some(body)) => body,
            Ok(None) => break, // EOF
            Err(err) => {
                let response = error_response(Value::Null, PARSE_ERROR, &err);
                let _ = write_message(&mut writer, &response);
                continue;
            }
        };

        let parsed: Value = match serde_json::from_str(&body) {
            Ok(v) => v,
            Err(err) => {
                let response =
                    error_response(Value::Null, PARSE_ERROR, &format!("invalid JSON: {err}"));
                let _ = write_message(&mut writer, &response);
                continue;
            }
        };

        let id = parsed.get("id").cloned().unwrap_or(Value::Null);
        let method = parsed.get("method").and_then(Value::as_str).unwrap_or("");
        let params = parsed.get("params").cloned().unwrap_or(json!({}));

        if method.is_empty() {
            let response = error_response(id, INVALID_REQUEST, "missing method field");
            let _ = write_message(&mut writer, &response);
            continue;
        }

        let response = dispatch(method, &params, &id, &mut engine);
        let _ = write_message(&mut writer, &response);

        if method == "shutdown" {
            break;
        }
    }
}

fn dispatch(method: &str, params: &Value, id: &Value, engine: &mut QueryEngine) -> Value {
    match method {
        "initialize" => handle_initialize(id),
        "shutdown" => handle_shutdown(id),
        "nocode/submit" => handle_submit(id, params, engine),
        "nocode/status" => handle_status(id, engine),
        _ => error_response(
            id.clone(),
            METHOD_NOT_FOUND,
            &format!("unknown method: {method}"),
        ),
    }
}

fn handle_initialize(id: &Value) -> Value {
    let tools: Vec<&str> = vec!["Read", "Edit", "Write", "Bash", "Glob", "Grep"];
    success_response(
        id.clone(),
        json!({
            "name": SERVER_NAME,
            "version": SERVER_VERSION,
            "capabilities": {
                "tools": tools,
                "methods": ["initialize", "shutdown", "nocode/submit", "nocode/status"],
            }
        }),
    )
}

fn handle_shutdown(id: &Value) -> Value {
    success_response(id.clone(), json!({"status": "shutting_down"}))
}

fn handle_submit(id: &Value, params: &Value, engine: &mut QueryEngine) -> Value {
    let message = match params.get("message").and_then(Value::as_str) {
        Some(m) => m,
        None => {
            return error_response(
                id.clone(),
                INVALID_REQUEST,
                "missing required param: message",
            );
        }
    };

    let plan = engine.submit_message(message, SubmitMessageOptions::default());

    let assistant_text = plan
        .model_response
        .final_assistant_message
        .as_ref()
        .map(|msg| msg.content.as_str())
        .unwrap_or("");

    let tool_count = plan.tool_results.len();

    let mut result = json!({
        "response": assistant_text,
        "tool_uses": tool_count,
        "stop_reason": plan.model_response.stop_reason.as_str(),
    });

    if let Some(err) = &plan.model_error {
        result["error"] = json!({
            "kind": err.kind.as_str(),
            "message": err.message,
            "retryable": err.retryable,
        });
    }

    success_response(id.clone(), result)
}

fn handle_status(id: &Value, engine: &QueryEngine) -> Value {
    let state = engine.state();
    success_response(
        id.clone(),
        json!({
            "turns": state.completed_turns.len(),
            "messages": state.mutable_messages.len(),
            "total_input_tokens": state.total_usage.input_tokens,
            "total_output_tokens": state.total_usage.output_tokens,
        }),
    )
}

// --- LSP-style Content-Length framing ---

fn read_message(reader: &mut impl BufRead) -> Result<Option<String>, String> {
    let mut header_line = String::new();
    let bytes_read = reader
        .read_line(&mut header_line)
        .map_err(|e| format!("failed to read header: {e}"))?;
    if bytes_read == 0 {
        return Ok(None);
    }

    let content_length = parse_content_length(header_line.trim())
        .ok_or_else(|| format!("invalid header: {}", header_line.trim()))?;

    // Read the blank separator line.
    let mut separator = String::new();
    reader
        .read_line(&mut separator)
        .map_err(|e| format!("failed to read separator: {e}"))?;

    let mut body = vec![0u8; content_length];
    reader
        .read_exact(&mut body)
        .map_err(|e| format!("failed to read body: {e}"))?;

    String::from_utf8(body)
        .map(Some)
        .map_err(|e| format!("invalid UTF-8 in body: {e}"))
}

fn parse_content_length(header: &str) -> Option<usize> {
    header.strip_prefix("Content-Length: ")?.parse().ok()
}

fn write_message(writer: &mut impl Write, response: &Value) -> io::Result<()> {
    let body = serde_json::to_string(response).expect("response serialization should not fail");
    write!(writer, "Content-Length: {}\r\n\r\n{}", body.len(), body)?;
    writer.flush()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_engine() -> QueryEngine {
        QueryEngine::new(crate::bootstrap_config())
    }

    #[test]
    fn initialize_returns_capabilities() {
        let mut engine = test_engine();
        let response = dispatch("initialize", &json!({}), &json!(1), &mut engine);

        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 1);
        let result = &response["result"];
        assert_eq!(result["name"], SERVER_NAME);
        assert_eq!(result["version"], SERVER_VERSION);
        assert!(result["capabilities"]["tools"].is_array());
        assert!(result["capabilities"]["methods"].is_array());
    }

    #[test]
    fn unknown_method_returns_method_not_found() {
        let mut engine = test_engine();
        let response = dispatch("bogus/method", &json!({}), &json!(2), &mut engine);

        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 2);
        assert_eq!(response["error"]["code"], METHOD_NOT_FOUND);
        assert!(
            response["error"]["message"]
                .as_str()
                .unwrap()
                .contains("bogus/method")
        );
    }

    #[test]
    fn submit_returns_model_response() {
        let mut engine = test_engine();
        let params = json!({"message": "hello from ide test"});
        let response = dispatch("nocode/submit", &params, &json!(3), &mut engine);

        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 3);
        let result = &response["result"];
        assert!(result.get("response").is_some());
        assert!(result.get("tool_uses").is_some());
        assert!(result.get("stop_reason").is_some());
    }

    #[test]
    fn submit_without_message_returns_error() {
        let mut engine = test_engine();
        let response = dispatch("nocode/submit", &json!({}), &json!(4), &mut engine);

        assert_eq!(response["error"]["code"], INVALID_REQUEST);
        assert!(
            response["error"]["message"]
                .as_str()
                .unwrap()
                .contains("message")
        );
    }

    #[test]
    fn status_returns_engine_state() {
        let mut engine = test_engine();
        let response = dispatch("nocode/status", &json!({}), &json!(5), &mut engine);

        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 5);
        let result = &response["result"];
        assert!(result.get("turns").is_some());
        assert!(result.get("messages").is_some());
        assert!(result.get("total_input_tokens").is_some());
        assert!(result.get("total_output_tokens").is_some());
    }

    #[test]
    fn framing_roundtrip() {
        let request_body = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}});
        let framed = format!(
            "Content-Length: {}\r\n\r\n{}",
            request_body.to_string().len(),
            request_body
        );
        let mut cursor = io::Cursor::new(framed.as_bytes());
        let body = read_message(&mut cursor)
            .expect("read should succeed")
            .expect("should have body");
        let parsed: Value = serde_json::from_str(&body).expect("should parse");
        assert_eq!(parsed["method"], "initialize");
    }

    #[test]
    fn shutdown_returns_status() {
        let mut engine = test_engine();
        let response = dispatch("shutdown", &json!({}), &json!(99), &mut engine);

        assert_eq!(response["result"]["status"], "shutting_down");
    }
}
