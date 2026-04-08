//! Tool input validation — JSON Schema validation for tool parameters.

use serde_json::Value;

/// Validate tool input against its JSON Schema.
/// Returns Ok(()) if valid, Err(message) if invalid.
pub fn validate_tool_input(input: &Value, schema: &Value) -> Result<(), String> {
    // Check required fields
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for field in required {
            if let Some(name) = field.as_str()
                && (input.get(name).is_none() || input[name].is_null())
            {
                return Err(format!("Missing required parameter: {name}"));
            }
        }
    }

    // Check property types and constraints
    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        for (name, prop_schema) in properties {
            if let Some(value) = input.get(name) {
                if value.is_null() {
                    continue;
                }
                // Type check
                if let Some(expected_type) = prop_schema.get("type").and_then(Value::as_str)
                    && !type_matches(value, expected_type)
                {
                    return Err(format!(
                        "Parameter '{name}' expected type '{expected_type}', got {}",
                        value_type_name(value)
                    ));
                }
                // Enum check
                if let Some(enum_values) = prop_schema.get("enum").and_then(Value::as_array)
                    && !enum_values.contains(value)
                {
                    return Err(format!(
                        "Parameter '{name}' must be one of: {}",
                        enum_values
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                // Array constraints
                if let Some(arr) = value.as_array() {
                    if let Some(min) = prop_schema.get("minItems").and_then(Value::as_u64)
                        && (arr.len() as u64) < min
                    {
                        return Err(format!(
                            "Parameter '{name}' requires at least {min} items, got {}",
                            arr.len()
                        ));
                    }
                    if let Some(max) = prop_schema.get("maxItems").and_then(Value::as_u64)
                        && (arr.len() as u64) > max
                    {
                        return Err(format!(
                            "Parameter '{name}' allows at most {max} items, got {}",
                            arr.len()
                        ));
                    }
                }
                // Number constraints
                if let Some(n) = value.as_f64() {
                    if let Some(min) = prop_schema.get("minimum").and_then(Value::as_f64)
                        && n < min
                    {
                        return Err(format!("Parameter '{name}' must be >= {min}, got {n}"));
                    }
                    if let Some(max) = prop_schema.get("maximum").and_then(Value::as_f64)
                        && n > max
                    {
                        return Err(format!("Parameter '{name}' must be <= {max}, got {n}"));
                    }
                }
            }
        }
    }

    Ok(())
}

fn type_matches(value: &Value, expected: &str) -> bool {
    match expected {
        "string" => value.is_string(),
        "integer" | "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        _ => true,
    }
}

fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validates_required_fields() {
        let schema =
            json!({"type":"object","properties":{"cmd":{"type":"string"}},"required":["cmd"]});
        assert!(validate_tool_input(&json!({"cmd":"ls"}), &schema).is_ok());
        assert!(validate_tool_input(&json!({}), &schema).is_err());
        assert!(validate_tool_input(&json!({"cmd":null}), &schema).is_err());
    }

    #[test]
    fn validates_types() {
        let schema = json!({"type":"object","properties":{
            "name":{"type":"string"},"count":{"type":"integer"},"flag":{"type":"boolean"}
        }});
        assert!(validate_tool_input(&json!({"name":"x","count":5,"flag":true}), &schema).is_ok());
        assert!(validate_tool_input(&json!({"name":123}), &schema).is_err());
        assert!(validate_tool_input(&json!({"count":"not a number"}), &schema).is_err());
    }

    #[test]
    fn allows_missing_optional_fields() {
        let schema = json!({"type":"object","properties":{"a":{"type":"string"},"b":{"type":"string"}},"required":["a"]});
        assert!(validate_tool_input(&json!({"a":"hello"}), &schema).is_ok());
    }

    #[test]
    fn validates_enum() {
        let schema = json!({"type":"object","properties":{
            "mode":{"type":"string","enum":["fast","slow"]}
        }});
        assert!(validate_tool_input(&json!({"mode":"fast"}), &schema).is_ok());
        assert!(validate_tool_input(&json!({"mode":"invalid"}), &schema).is_err());
    }

    #[test]
    fn validates_array_min_max_items() {
        let schema = json!({"type":"object","properties":{
            "items":{"type":"array","minItems":1,"maxItems":3}
        }});
        assert!(validate_tool_input(&json!({"items":[1]}), &schema).is_ok());
        assert!(validate_tool_input(&json!({"items":[1,2,3]}), &schema).is_ok());
        assert!(validate_tool_input(&json!({"items":[]}), &schema).is_err());
        assert!(validate_tool_input(&json!({"items":[1,2,3,4]}), &schema).is_err());
    }

    #[test]
    fn validates_number_min_max() {
        let schema = json!({"type":"object","properties":{
            "count":{"type":"number","minimum":0,"maximum":100}
        }});
        assert!(validate_tool_input(&json!({"count":50}), &schema).is_ok());
        assert!(validate_tool_input(&json!({"count":0}), &schema).is_ok());
        assert!(validate_tool_input(&json!({"count":100}), &schema).is_ok());
        assert!(validate_tool_input(&json!({"count":-1}), &schema).is_err());
        assert!(validate_tool_input(&json!({"count":101}), &schema).is_err());
    }
}
