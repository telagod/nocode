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
        // In non-interactive mode, return the questions for the caller to handle
        let questions = &input["questions"];
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
            "additionalProperties": true
        })
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        // Return current config state or apply changes
        ToolOutput::success(format!(
            "Config operation: {}",
            serde_json::to_string(input).unwrap_or_default()
        ))
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
