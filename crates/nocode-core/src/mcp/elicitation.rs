//! MCP Elicitation — handles MCP server requests for user input.
//!
//! When an MCP server needs information from the user (e.g., OAuth consent,
//! configuration values), it sends an `elicitation/create` JSON-RPC request.
//! This module defines the protocol types and a handler trait.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// An elicitation request from an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElicitationRequest {
    /// Unique ID for this elicitation.
    pub id: String,
    /// Human-readable message/question to display.
    pub message: String,
    /// Schema describing the expected response shape.
    #[serde(default)]
    pub schema: Option<ElicitationSchema>,
    /// Timeout in seconds (0 = no timeout).
    #[serde(default)]
    pub timeout_secs: u64,
}

/// Schema for elicitation response validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElicitationSchema {
    /// Field definitions the user should fill in.
    pub fields: Vec<ElicitationField>,
}

/// A single field in an elicitation form.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElicitationField {
    pub name: String,
    pub description: String,
    #[serde(default = "default_field_type")]
    pub field_type: ElicitationFieldType,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default_value: Option<String>,
}

fn default_field_type() -> ElicitationFieldType {
    ElicitationFieldType::Text
}
// APPEND_REST

/// Field type for elicitation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ElicitationFieldType {
    Text,
    Password,
    Boolean,
    Select,
    Number,
}

/// User's response to an elicitation request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElicitationResponse {
    /// The elicitation ID this responds to.
    pub id: String,
    /// User's action.
    pub action: ElicitationAction,
    /// Field values provided by the user (name → value).
    #[serde(default)]
    pub values: std::collections::HashMap<String, Value>,
}

/// User action on an elicitation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ElicitationAction {
    /// User provided the requested information.
    Confirm,
    /// User declined / cancelled.
    Deny,
    /// Request timed out.
    Timeout,
}

/// Trait for handling elicitation requests.
/// Implementations display the request to the user and collect their response.
pub trait ElicitationHandler: Send + Sync {
    fn handle(&self, request: &ElicitationRequest) -> ElicitationResponse;
}

/// Auto-deny handler — rejects all elicitation requests.
pub struct AutoDenyHandler;

impl ElicitationHandler for AutoDenyHandler {
    fn handle(&self, request: &ElicitationRequest) -> ElicitationResponse {
        ElicitationResponse {
            id: request.id.clone(),
            action: ElicitationAction::Deny,
            values: std::collections::HashMap::new(),
        }
    }
}

/// Auto-confirm handler — accepts with default values (for testing).
pub struct AutoConfirmHandler;

impl ElicitationHandler for AutoConfirmHandler {
    fn handle(&self, request: &ElicitationRequest) -> ElicitationResponse {
        let mut values = std::collections::HashMap::new();
        if let Some(schema) = &request.schema {
            for field in &schema.fields {
                if let Some(default) = &field.default_value {
                    values.insert(
                        field.name.clone(),
                        Value::String(default.clone()),
                    );
                }
            }
        }
        ElicitationResponse {
            id: request.id.clone(),
            action: ElicitationAction::Confirm,
            values,
        }
    }
}

/// Interactive handler — communicates with a user interface via channels.
///
/// The TUI or REPL sends an `ElicitationRequest` through the `tx` channel,
/// and receives an `ElicitationResponse` back on the `rx` channel.
/// This decouples the MCP protocol layer from the UI layer.
pub struct InteractiveElicitationHandler {
    tx: std::sync::mpsc::Sender<(ElicitationRequest, std::sync::mpsc::Sender<ElicitationResponse>)>,
}

impl InteractiveElicitationHandler {
    /// Create a new interactive handler, returning the handler and a receiver
    /// for the UI to listen on.
    pub fn new() -> (Self, std::sync::mpsc::Receiver<(ElicitationRequest, std::sync::mpsc::Sender<ElicitationResponse>)>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (Self { tx }, rx)
    }
}

impl ElicitationHandler for InteractiveElicitationHandler {
    fn handle(&self, request: &ElicitationRequest) -> ElicitationResponse {
        // Create a one-shot channel for the response
        let (resp_tx, resp_rx) = std::sync::mpsc::channel();

        // Send the request + response sender to the UI
        if self.tx.send((request.clone(), resp_tx)).is_err() {
            return ElicitationResponse {
                id: request.id.clone(),
                action: ElicitationAction::Deny,
                values: std::collections::HashMap::new(),
            };
        }

        // Wait for the UI response (with timeout)
        let timeout = if request.timeout_secs > 0 {
            std::time::Duration::from_secs(request.timeout_secs)
        } else {
            std::time::Duration::from_secs(120) // default 2 min
        };

        match resp_rx.recv_timeout(timeout) {
            Ok(response) => response,
            Err(_) => ElicitationResponse {
                id: request.id.clone(),
                action: ElicitationAction::Timeout,
                values: std::collections::HashMap::new(),
            },
        }
    }
}

/// Helper for the TUI/REPL side: process a pending elicitation request
/// by auto-filling defaults and returning a confirm response.
/// Useful as a fallback when no interactive UI is available.
pub fn auto_fill_defaults(request: &ElicitationRequest) -> ElicitationResponse {
    let mut values = std::collections::HashMap::new();
    if let Some(schema) = &request.schema {
        for field in &schema.fields {
            let value = match field.field_type {
                ElicitationFieldType::Boolean => Value::Bool(false),
                ElicitationFieldType::Number => Value::Number(0.into()),
                _ => Value::String(field.default_value.clone().unwrap_or_default()),
            };
            values.insert(field.name.clone(), value);
        }
    }
    ElicitationResponse {
        id: request.id.clone(),
        action: ElicitationAction::Confirm,
        values,
    }
}

/// Parse an elicitation/create JSON-RPC params into an ElicitationRequest.
pub fn parse_elicitation_request(params: &Value) -> Result<ElicitationRequest, String> {
    serde_json::from_value(params.clone())
        .map_err(|e| format!("invalid elicitation request: {e}"))
}

/// Build a JSON-RPC result from an ElicitationResponse.
pub fn build_elicitation_result(response: &ElicitationResponse) -> Value {
    serde_json::to_value(response).unwrap_or(serde_json::json!({"error": "serialize failed"}))
}

/// Validate that all required fields have values in the response.
pub fn validate_response(
    schema: &ElicitationSchema,
    response: &ElicitationResponse,
) -> Result<(), Vec<String>> {
    let mut missing = Vec::new();
    for field in &schema.fields {
        if field.required && !response.values.contains_key(&field.name) {
            missing.push(field.name.clone());
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(missing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_request() -> ElicitationRequest {
        ElicitationRequest {
            id: "elicit-1".to_string(),
            message: "Please provide your API key".to_string(),
            schema: Some(ElicitationSchema {
                fields: vec![
                    ElicitationField {
                        name: "api_key".to_string(),
                        description: "Your API key".to_string(),
                        field_type: ElicitationFieldType::Password,
                        required: true,
                        default_value: None,
                    },
                    ElicitationField {
                        name: "region".to_string(),
                        description: "AWS region".to_string(),
                        field_type: ElicitationFieldType::Text,
                        required: false,
                        default_value: Some("us-east-1".to_string()),
                    },
                ],
            }),
            timeout_secs: 60,
        }
    }

    #[test]
    fn request_serialization_roundtrip() {
        let req = sample_request();
        let json = serde_json::to_string(&req).unwrap();
        let parsed: ElicitationRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "elicit-1");
        assert_eq!(parsed.schema.unwrap().fields.len(), 2);
    }

    #[test]
    fn parse_from_json_value() {
        let val = json!({
            "id": "e-2",
            "message": "Confirm OAuth",
            "timeout_secs": 30
        });
        let req = parse_elicitation_request(&val).unwrap();
        assert_eq!(req.id, "e-2");
        assert!(req.schema.is_none());
        assert_eq!(req.timeout_secs, 30);
    }

    #[test]
    fn auto_deny_handler() {
        let handler = AutoDenyHandler;
        let req = sample_request();
        let resp = handler.handle(&req);
        assert_eq!(resp.action, ElicitationAction::Deny);
        assert!(resp.values.is_empty());
    }

    #[test]
    fn auto_confirm_handler_uses_defaults() {
        let handler = AutoConfirmHandler;
        let req = sample_request();
        let resp = handler.handle(&req);
        assert_eq!(resp.action, ElicitationAction::Confirm);
        // region has default, api_key does not
        assert_eq!(resp.values.len(), 1);
        assert_eq!(resp.values["region"], "us-east-1");
    }

    #[test]
    fn validate_response_missing_required() {
        let schema = sample_request().schema.unwrap();
        let resp = ElicitationResponse {
            id: "e-1".to_string(),
            action: ElicitationAction::Confirm,
            values: std::collections::HashMap::new(),
        };
        let result = validate_response(&schema, &resp);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), vec!["api_key"]);
    }

    #[test]
    fn validate_response_all_required_present() {
        let schema = sample_request().schema.unwrap();
        let mut values = std::collections::HashMap::new();
        values.insert("api_key".to_string(), json!("sk-xxx"));
        let resp = ElicitationResponse {
            id: "e-1".to_string(),
            action: ElicitationAction::Confirm,
            values,
        };
        assert!(validate_response(&schema, &resp).is_ok());
    }

    #[test]
    fn build_result_json() {
        let resp = ElicitationResponse {
            id: "e-1".to_string(),
            action: ElicitationAction::Confirm,
            values: std::collections::HashMap::new(),
        };
        let val = build_elicitation_result(&resp);
        assert_eq!(val["action"], "confirm");
        assert_eq!(val["id"], "e-1");
    }

    #[test]
    fn field_type_variants() {
        let types = [
            ElicitationFieldType::Text,
            ElicitationFieldType::Password,
            ElicitationFieldType::Boolean,
            ElicitationFieldType::Select,
            ElicitationFieldType::Number,
        ];
        for t in &types {
            let json = serde_json::to_string(t).unwrap();
            let parsed: ElicitationFieldType = serde_json::from_str(&json).unwrap();
            assert_eq!(&parsed, t);
        }
    }

    #[test]
    fn action_variants() {
        let actions = [
            ElicitationAction::Confirm,
            ElicitationAction::Deny,
            ElicitationAction::Timeout,
        ];
        for a in &actions {
            let json = serde_json::to_string(a).unwrap();
            let parsed: ElicitationAction = serde_json::from_str(&json).unwrap();
            assert_eq!(&parsed, a);
        }
    }
}
