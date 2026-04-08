//! SendMessage tool — inter-agent communication.

use crate::agent::worker::global_worker_registry;
use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};

pub struct SendMessageTool;

impl Tool for SendMessageTool {
    fn name(&self) -> &str {
        "SendMessage"
    }
    fn description(&self) -> &str {
        "Send a message to another agent by name or ID. Used for inter-agent coordination."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "to": {
                    "type": "string",
                    "description": "The name or ID of the agent to send the message to"
                },
                "message": {
                    "description": "The message content to send (string or object)",
                }
            },
            "required": ["to", "message"]
        })
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(to) = input["to"].as_str() else {
            return ToolOutput::error("Missing required parameter: to");
        };
        let message = &input["message"];
        if message.is_null() {
            return ToolOutput::error("Missing required parameter: message");
        }

        let registry = global_worker_registry();
        let mut guard = registry.lock().unwrap();

        match guard.send_message(to, "leader", message.clone()) {
            Ok(()) => {
                ToolOutput::success(json!({"sent_to": to, "status": "delivered"}).to_string())
            }
            Err(e) => ToolOutput::error(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::worker::global_worker_registry;

    #[test]
    fn send_message_missing_to() {
        let tool = SendMessageTool;
        let result = tool.execute(&json!({"message": "hello"}));
        assert!(result.is_error);
    }

    #[test]
    fn send_message_missing_message() {
        let tool = SendMessageTool;
        let result = tool.execute(&json!({"to": "worker-1"}));
        assert!(result.is_error);
    }

    #[test]
    fn send_message_to_nonexistent_worker() {
        let tool = SendMessageTool;
        let result = tool.execute(&json!({"to": "ghost", "message": "hello"}));
        assert!(result.is_error);
    }

    #[test]
    fn send_message_delivers_to_worker() {
        let reg = global_worker_registry();
        let id = {
            let mut guard = reg.lock().unwrap();
            guard.register("test-recv", "test prompt")
        };
        let tool = SendMessageTool;
        let result = tool.execute(&json!({"to": &id, "message": "hello agent"}));
        assert!(!result.is_error, "Send failed: {}", result.content);

        // Verify message arrived
        let mut guard = reg.lock().unwrap();
        let msgs = guard.drain_inbox(&id);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, json!("hello agent"));
        guard.remove(&id);
    }
}
