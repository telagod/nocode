//! Interactive tools — AskUserQuestion, Config, NotebookEdit.

use crate::config::settings::{Settings, SettingsTier};
use crate::tool::permission::{AutoFirstOptionPrompter, QuestionPrompter};
use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// AskUserQuestion
// ---------------------------------------------------------------------------

/// AskUserQuestion tool — presents questions to the user and waits for responses.
///
/// Uses a `QuestionPrompter` bridge (injected via `set_question_prompter`)
/// to block the tool execution thread until the user answers in TUI/REPL.
pub struct AskUserQuestionTool {
    prompter: Mutex<Option<Box<dyn QuestionPrompter>>>,
}

impl AskUserQuestionTool {
    pub fn new() -> Self {
        Self {
            prompter: Mutex::new(None),
        }
    }

    /// Inject a question prompter (e.g. TUI bridge or REPL stdin bridge).
    /// Falls back to `AutoFirstOptionPrompter` if none is set.
    pub fn set_prompter(&self, prompter: Box<dyn QuestionPrompter>) {
        if let Ok(mut p) = self.prompter.lock() {
            *p = Some(prompter);
        }
    }
}

impl Default for AskUserQuestionTool {
    fn default() -> Self {
        Self::new()
    }
}

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

        // Use injected prompter, or fall back to auto-first-option
        let result = if let Ok(guard) = self.prompter.lock() {
            if let Some(ref prompter) = *guard {
                prompter.prompt_questions(questions)
            } else {
                AutoFirstOptionPrompter.prompt_questions(questions)
            }
        } else {
            AutoFirstOptionPrompter.prompt_questions(questions)
        };

        match result {
            Ok(answer) => {
                // Build structured response matching Claude Code's expected format
                let response: Vec<Value> = answer
                    .selections
                    .iter()
                    .enumerate()
                    .map(|(i, sel)| {
                        json!({
                            "question_index": i,
                            "answer": sel
                        })
                    })
                    .collect();
                ToolOutput::success(json!({"answers": response}).to_string())
            }
            Err(e) => ToolOutput::error(format!("Question cancelled: {e}")),
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

pub struct ConfigTool;

fn parse_settings_tier(value: Option<&str>) -> Result<SettingsTier, String> {
    match value.unwrap_or("project") {
        "user" => Ok(SettingsTier::User),
        "project" => Ok(SettingsTier::Project),
        "local" => Ok(SettingsTier::Local),
        other => Err(format!(
            "Unknown config tier: {other}. Use user, project, or local."
        )),
    }
}

fn stringify_config_value(value: &Value) -> Result<String, String> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Bool(flag) => Ok(flag.to_string()),
        Value::Number(num) => Ok(num.to_string()),
        _ => Err("Config value must be a string, number, or boolean".to_string()),
    }
}

impl ConfigTool {
    fn execute_with_cwd(&self, input: &Value, cwd: &str) -> ToolOutput {
        let action = input["action"].as_str().unwrap_or("list");

        match action {
            "list" => {
                let settings = Settings::load_merged(cwd);
                let config = crate::config::runtime::RuntimeConfig::from_settings(&settings, cwd);
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
                        "set_keys": settings.list_set_keys(),
                    })
                    .to_string(),
                )
            }
            "get" => {
                let Some(key) = input["key"].as_str() else {
                    return ToolOutput::error("Missing required parameter: key");
                };
                let settings = Settings::load_merged(cwd);
                let config = crate::config::runtime::RuntimeConfig::from_settings(&settings, cwd);
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
                let tier = match parse_settings_tier(input["tier"].as_str()) {
                    Ok(tier) => tier,
                    Err(e) => return ToolOutput::error(e),
                };
                let value = match stringify_config_value(value) {
                    Ok(value) => value,
                    Err(e) => return ToolOutput::error(e),
                };

                let path = tier.path_for(cwd);
                let mut settings = Settings::load_from(&path);
                match settings.set_and_persist(key, &value, tier, cwd) {
                    Ok(()) => ToolOutput::success(
                        json!({
                            "key": key,
                            "value": value,
                            "tier": input["tier"].as_str().unwrap_or("project"),
                            "written_to": path,
                        })
                        .to_string(),
                    ),
                    Err(e) => ToolOutput::error(e),
                }
            }
            _ => ToolOutput::error(format!("Unknown action: {action}. Use get, set, or list.")),
        }
    }
}

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
                "value": { "description": "New value (for set)" },
                "tier": {
                    "type": "string",
                    "enum": ["user", "project", "local"],
                    "description": "Settings tier to read/write for set operations (default: project)"
                }
            }
        })
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| String::from("."));
        self.execute_with_cwd(input, &cwd)
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
    use tempfile::tempdir;

    #[test]
    fn ask_user_question_missing_questions() {
        let tool = AskUserQuestionTool::new();
        let result = tool.execute(&json!({}));
        assert!(result.is_error);
    }

    #[test]
    fn ask_user_question_auto_first_option() {
        let tool = AskUserQuestionTool::new();
        let result = tool.execute(&json!({
            "questions": [{"question": "Which?", "header": "Choice", "options": [
                {"label": "A", "description": "Option A"},
                {"label": "B", "description": "Option B"}
            ]}]
        }));
        assert!(!result.is_error);
        // Without injected prompter, auto-selects first option "A"
        assert!(result.content.contains("A"));
        assert!(result.content.contains("answers"));
    }

    #[test]
    fn ask_user_question_with_prompter() {
        use crate::tool::permission::UserAnswer;
        use std::sync::Mutex;

        /// Test prompter that returns predetermined answers.
        struct TestPrompter {
            answers: Mutex<Vec<String>>,
        }
        impl QuestionPrompter for TestPrompter {
            fn prompt_questions(
                &self,
                _questions: &serde_json::Value,
            ) -> Result<UserAnswer, String> {
                let mut guard = self.answers.lock().unwrap();
                let selections: Vec<String> = guard.drain(..).collect();
                Ok(UserAnswer { selections })
            }
        }

        let tool = AskUserQuestionTool::new();
        tool.set_prompter(Box::new(TestPrompter {
            answers: Mutex::new(vec!["B".to_string()]),
        }));

        let result = tool.execute(&json!({
            "questions": [{"question": "Which?", "header": "Choice", "options": [
                {"label": "A", "description": "Option A"},
                {"label": "B", "description": "Option B"}
            ]}]
        }));
        assert!(!result.is_error);
        assert!(result.content.contains("B"));
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
    fn config_set_persists_to_project_tier() {
        let tool = ConfigTool;
        let dir = tempdir().unwrap();
        let result = tool.execute_with_cwd(
            &json!({
                "action": "set",
                "key": "model",
                "value": "gpt-5",
                "tier": "project"
            }),
            dir.path().to_str().unwrap(),
        );
        assert!(!result.is_error);
        let settings = Settings::load_from(&dir.path().join(".nocode/settings.json"));
        assert_eq!(settings.model.as_deref(), Some("gpt-5"));
    }

    #[test]
    fn config_set_rejects_invalid_tier() {
        let tool = ConfigTool;
        let result = tool.execute(&json!({
            "action": "set",
            "key": "model",
            "value": "gpt-5",
            "tier": "unknown"
        }));
        assert!(result.is_error);
    }

    #[test]
    fn config_set_rejects_non_scalar_value() {
        let tool = ConfigTool;
        let result = tool.execute(&json!({
            "action": "set",
            "key": "model",
            "value": {"nested": true}
        }));
        assert!(result.is_error);
    }

    #[test]
    fn config_list_reports_set_keys() {
        let dir = tempdir().unwrap();
        let settings = Settings {
            model: Some("gpt-5".to_string()),
            max_turns: Some(42),
            ..Default::default()
        };
        settings
            .save_to(&dir.path().join(".nocode/settings.json"))
            .unwrap();

        let tool = ConfigTool;
        let result =
            tool.execute_with_cwd(&json!({"action": "list"}), dir.path().to_str().unwrap());

        assert!(!result.is_error);
        assert!(result.content.contains("set_keys"));
        assert!(result.content.contains("gpt-5"));
        assert!(result.content.contains("42"));
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
