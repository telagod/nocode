use serde_json::{json, Value};

/// Return the JSON Schema for a known tool, or `None` for unknown tools.
pub fn get_tool_schema(tool_name: &str) -> Option<Value> {
    match tool_name {
        "Read" => Some(json!({
            "type": "object",
            "required": ["file_path"],
            "properties": {
                "file_path": { "type": "string" }
            }
        })),
        "Edit" => Some(json!({
            "type": "object",
            "required": ["file_path", "old_string", "new_string"],
            "properties": {
                "file_path": { "type": "string" },
                "old_string": { "type": "string" },
                "new_string": { "type": "string" }
            }
        })),
        "Write" => Some(json!({
            "type": "object",
            "required": ["file_path", "content"],
            "properties": {
                "file_path": { "type": "string" },
                "content": { "type": "string" }
            }
        })),
        "Bash" => Some(json!({
            "type": "object",
            "required": ["command"],
            "properties": {
                "command": { "type": "string" },
                "description": { "type": "string" }
            }
        })),
        "Glob" => Some(json!({
            "type": "object",
            "required": ["pattern"],
            "properties": {
                "pattern": { "type": "string" },
                "path": { "type": "string" }
            }
        })),
        "Grep" => Some(json!({
            "type": "object",
            "required": ["pattern"],
            "properties": {
                "pattern": { "type": "string" },
                "path": { "type": "string" },
                "glob": { "type": "string" }
            }
        })),
        "WebFetch" => Some(json!({
            "type": "object",
            "required": ["url"],
            "properties": {
                "url": { "type": "string" }
            }
        })),
        "WebSearch" => Some(json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "query": { "type": "string" }
            }
        })),
        "Agent" => Some(json!({
            "type": "object",
            "required": ["agent_id", "prompt"],
            "properties": {
                "agent_id": { "type": "string" },
                "prompt": { "type": "string" }
            }
        })),
        "ToolSearch" => Some(json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "query": { "type": "string" },
                "max_results": { "type": "string" }
            }
        })),
        "Lsp" => Some(json!({
            "type": "object",
            "required": ["action", "file_path"],
            "properties": {
                "action": { "type": "string" },
                "file_path": { "type": "string" },
                "line": { "type": "string" },
                "column": { "type": "string" }
            }
        })),
        _ => None,
    }
}

/// Validate tool input arguments against the tool's JSON Schema.
///
/// Returns `Ok(())` if the tool is unknown (no schema) or validation passes.
/// Returns `Err(message)` with a human-readable error if validation fails.
pub fn validate_tool_input(tool_name: &str, arguments: &[(String, String)]) -> Result<(), String> {
    let schema_value = match get_tool_schema(tool_name) {
        Some(s) => s,
        None => return Ok(()),
    };

    let mut obj = serde_json::Map::new();
    for (key, value) in arguments {
        obj.insert(key.clone(), Value::String(value.clone()));
    }
    let instance = Value::Object(obj);

    let compiled = jsonschema::JSONSchema::compile(&schema_value)
        .map_err(|e| format!("invalid tool schema for {tool_name}: {e}"))?;

    if let Err(errors) = compiled.validate(&instance) {
        let messages: Vec<String> = errors.map(|e| e.to_string()).collect();
        return Err(format!(
            "input validation failed for {tool_name}: {}",
            messages.join("; ")
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_read_input_passes() {
        let args = vec![("file_path".to_string(), "src/main.rs".to_string())];
        assert!(validate_tool_input("Read", &args).is_ok());
    }

    #[test]
    fn missing_required_field_fails() {
        let args: Vec<(String, String)> = vec![];
        let result = validate_tool_input("Read", &args);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("file_path"));
    }

    #[test]
    fn unknown_tool_skips_validation() {
        let args: Vec<(String, String)> = vec![];
        assert!(validate_tool_input("UnknownTool", &args).is_ok());
    }

    #[test]
    fn extra_fields_are_allowed() {
        let args = vec![
            ("file_path".to_string(), "src/main.rs".to_string()),
            ("extra_field".to_string(), "extra_value".to_string()),
        ];
        assert!(validate_tool_input("Read", &args).is_ok());
    }
}