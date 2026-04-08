//! Interactive tools — AskUserQuestion, Config, NotebookEdit.

use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// AskUserQuestion
// ---------------------------------------------------------------------------

pub struct AskUserQuestionTool;

impl Tool for AskUserQuestionTool {
    fn name(&self) -> &str {
        "AskUserQuestion"
    }
    fn description(&self) -> &str {
        "Ask the user a question and wait for their response."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "questions": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "question": { "type": "string", "description": "The question to ask" },
                            "header": { "type": "string", "description": "Short label (max 12 chars)" },
                            "options": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "label": { "type": "string" },
                                        "description": { "type": "string" }
                                    },
                                    "required": ["label", "description"]
                                }
                            }
                        },
                        "required": ["question", "header", "options"]
                    },
                    "minItems": 1,
                    "maxItems": 4
                }
            },
            "required": ["questions"]
        })
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        let questions = &input["questions"];
        if questions.is_null() || !questions.is_array() {
            return ToolOutput::error("Missing required parameter: questions");
        }
        ToolOutput::success(format!(
            "Questions pending user response: {}",
            serde_json::to_string_pretty(questions).unwrap_or_default()
        ))
    }
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

pub struct ConfigTool;

impl Tool for ConfigTool {
    fn name(&self) -> &str {
        "Config"
    }
    fn description(&self) -> &str {
        "View or modify configuration settings."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["get", "set", "list"],
                    "description": "Action to perform (default: list)"
                },
                "key": { "type": "string", "description": "Setting key (for get/set)" },
                "value": { "description": "New value (for set)" }
            }
        })
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        let action = input["action"].as_str().unwrap_or("list");
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| String::from("."));

        match action {
            "list" => {
                let settings = crate::config::settings::Settings::load_merged(&cwd);
                let config = crate::config::runtime::RuntimeConfig::from_settings(&settings, &cwd);
                ToolOutput::success(
                    json!({
                        "model": config.model,
                        "permission_mode": config.permission_mode,
                        "max_turns": config.max_turns,
                        "max_tokens": config.max_tokens,
                        "system_prompt": config.system_prompt.is_some(),
                        "reasoning_effort": config.reasoning_effort,
                        "mcp_servers": config.mcp_servers.keys().collect::<Vec<_>>(),
                        "hooks_configured": !config.hooks.pre_tool_use.is_empty()
                            || !config.hooks.post_tool_use.is_empty()
                            || !config.hooks.on_submit.is_empty(),
                        "sandbox_enabled": config.sandbox.enabled,
                    })
                    .to_string(),
                )
            }
            "get" => {
                let Some(key) = input["key"].as_str() else {
                    return ToolOutput::error("Missing required parameter: key");
                };
                let settings = crate::config::settings::Settings::load_merged(&cwd);
                let config = crate::config::runtime::RuntimeConfig::from_settings(&settings, &cwd);
                let value = match key {
                    "model" => json!(config.model),
                    "permission_mode" => json!(config.permission_mode),
                    "max_turns" => json!(config.max_turns),
                    "max_tokens" => json!(config.max_tokens),
                    "system_prompt" => json!(config.system_prompt),
                    "reasoning_effort" => json!(config.reasoning_effort),
                    "sandbox_enabled" => json!(config.sandbox.enabled),
                    _ => return ToolOutput::error(format!("Unknown config key: {key}")),
                };
                ToolOutput::success(json!({"key": key, "value": value}).to_string())
            }
            "set" => {
                let Some(key) = input["key"].as_str() else {
                    return ToolOutput::error("Missing required parameter: key");
                };
                let value = &input["value"];
                if value.is_null() {
                    return ToolOutput::error("Missing required parameter: value");
                }

                let settings_path = format!("{cwd}/.nocode/settings.json");
                let mut settings: serde_json::Map<String, Value> =
                    std::fs::read_to_string(&settings_path)
                        .ok()
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or_default();

                settings.insert(key.to_string(), value.clone());

                // Ensure directory exists
                let _ = std::fs::create_dir_all(format!("{cwd}/.nocode"));
                match std::fs::write(
                    &settings_path,
                    serde_json::to_string_pretty(&settings).unwrap_or_default(),
                ) {
                    Ok(()) => ToolOutput::success(
                        json!({"key": key, "value": value, "written_to": settings_path})
                            .to_string(),
                    ),
                    Err(e) => ToolOutput::error(format!("Failed to write settings: {e}")),
                }
            }
            _ => ToolOutput::error(format!("Unknown action: {action}. Use get, set, or list.")),
        }
    }
}

// ---------------------------------------------------------------------------
// NotebookEdit
// ---------------------------------------------------------------------------

pub struct NotebookEditTool;

impl Tool for NotebookEditTool {
    fn name(&self) -> &str {
        "NotebookEdit"
    }
    fn description(&self) -> &str {
        "Edit a Jupyter notebook cell. Supports replace, insert, and delete operations."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "notebook_path": { "type": "string", "description": "Absolute path to the Jupyter notebook file" },
                "cell_id": { "type": "string", "description": "The ID of the cell to edit" },
                "new_source": { "type": "string", "description": "The new source for the cell" },
                "cell_type": { "type": "string", "enum": ["code", "markdown"], "description": "Cell type (code or markdown)" },
                "edit_mode": { "type": "string", "enum": ["replace", "insert", "delete"], "description": "Edit mode. Defaults to replace." }
            },
            "required": ["notebook_path", "new_source"]
        })
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(path) = input["notebook_path"].as_str() else {
            return ToolOutput::error("Missing required parameter: notebook_path");
        };
        let Some(new_source) = input["new_source"].as_str() else {
            return ToolOutput::error("Missing required parameter: new_source");
        };
        let cell_id = input["cell_id"].as_str();
        let edit_mode = input["edit_mode"].as_str().unwrap_or("replace");
        let cell_type = input["cell_type"].as_str().unwrap_or("code");

        // Read notebook JSON
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => return ToolOutput::error(format!("Failed to read notebook: {e}")),
        };

        let mut notebook: Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(e) => return ToolOutput::error(format!("Invalid notebook JSON: {e}")),
        };

        let Some(cells) = notebook.get_mut("cells").and_then(Value::as_array_mut) else {
            return ToolOutput::error("Notebook has no cells array");
        };

        match edit_mode {
            "delete" => {
                if let Some(id) = cell_id {
                    let before = cells.len();
                    cells.retain(|c| c.get("id").and_then(Value::as_str) != Some(id));
                    if cells.len() == before {
                        return ToolOutput::error(format!("Cell {id} not found"));
                    }
                } else {
                    return ToolOutput::error("cell_id required for delete");
                }
            }
            "insert" => {
                let new_cell = json!({
                    "cell_type": cell_type,
                    "source": new_source.lines().map(|l| format!("{l}\n")).collect::<Vec<_>>(),
                    "metadata": {},
                    "outputs": []
                });
                if let Some(id) = cell_id {
                    if let Some(pos) = cells
                        .iter()
                        .position(|c| c.get("id").and_then(Value::as_str) == Some(id))
                    {
                        cells.insert(pos + 1, new_cell);
                    } else {
                        cells.push(new_cell);
                    }
                } else {
                    cells.insert(0, new_cell);
                }
            }
            _ => {
                // replace
                if let Some(id) = cell_id {
                    if let Some(cell) = cells
                        .iter_mut()
                        .find(|c| c.get("id").and_then(Value::as_str) == Some(id))
                    {
                        cell["source"] = json!(
                            new_source
                                .lines()
                                .map(|l| format!("{l}\n"))
                                .collect::<Vec<String>>()
                        );
                        if cell_type == "markdown" || cell_type == "code" {
                            cell["cell_type"] = json!(cell_type);
                        }
                    } else {
                        return ToolOutput::error(format!("Cell {id} not found"));
                    }
                } else if let Some(cell) = cells.first_mut() {
                    cell["source"] = json!(
                        new_source
                            .lines()
                            .map(|l| format!("{l}\n"))
                            .collect::<Vec<String>>()
                    );
                } else {
                    return ToolOutput::error("No cells to replace");
                }
            }
        }

        // Write back
        match serde_json::to_string_pretty(&notebook) {
            Ok(json_str) => match std::fs::write(path, &json_str) {
                Ok(()) => ToolOutput::success(format!("Notebook {path} updated ({edit_mode})")),
                Err(e) => ToolOutput::error(format!("Failed to write notebook: {e}")),
            },
            Err(e) => ToolOutput::error(format!("Failed to serialize notebook: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ask_user_question_missing_questions() {
        let tool = AskUserQuestionTool;
        let result = tool.execute(&json!({}));
        assert!(result.is_error);
    }

    #[test]
    fn ask_user_question_returns_questions() {
        let tool = AskUserQuestionTool;
        let result = tool.execute(&json!({
            "questions": [{"question": "Which?", "header": "Choice", "options": [
                {"label": "A", "description": "Option A"},
                {"label": "B", "description": "Option B"}
            ]}]
        }));
        assert!(!result.is_error);
        assert!(result.content.contains("Which?"));
    }

    #[test]
    fn config_list_returns_model() {
        let tool = ConfigTool;
        let result = tool.execute(&json!({"action": "list"}));
        assert!(!result.is_error);
        assert!(result.content.contains("model"));
    }

    #[test]
    fn config_get_known_key() {
        let tool = ConfigTool;
        let result = tool.execute(&json!({"action": "get", "key": "model"}));
        assert!(!result.is_error);
        assert!(result.content.contains("model"));
    }

    #[test]
    fn config_get_unknown_key() {
        let tool = ConfigTool;
        let result = tool.execute(&json!({"action": "get", "key": "nonexistent_xyz"}));
        assert!(result.is_error);
    }

    #[test]
    fn notebook_edit_missing_path() {
        let tool = NotebookEditTool;
        let result = tool.execute(&json!({"new_source": "print('hi')"}));
        assert!(result.is_error);
    }

    #[test]
    fn notebook_edit_missing_source() {
        let tool = NotebookEditTool;
        let result = tool.execute(&json!({"notebook_path": "/tmp/test.ipynb"}));
        assert!(result.is_error);
    }
}
