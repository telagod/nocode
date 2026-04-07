//! Team tools — TeamCreate, TeamDelete.

use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};

pub struct TeamCreateTool;

impl Tool for TeamCreateTool {
    fn name(&self) -> &str {
        "TeamCreate"
    }
    fn description(&self) -> &str {
        "Create a new team to coordinate multiple agents."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "team_name":{"type":"string","description":"Name for the new team"},
            "description":{"type":"string","description":"Team description/purpose"}
        },"required":["team_name"]})
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(name) = input["team_name"].as_str() else {
            return ToolOutput::error("Missing required parameter: team_name");
        };
        let desc = input["description"].as_str().unwrap_or("");
        // TODO: create team config file and task list directory
        ToolOutput::success(format!("Team '{name}' created. Description: {desc}"))
    }
}

pub struct TeamDeleteTool;

impl Tool for TeamDeleteTool {
    fn name(&self) -> &str {
        "TeamDelete"
    }
    fn description(&self) -> &str {
        "Delete a team and its associated resources."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "team_name":{"type":"string"}
        },"required":["team_name"]})
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(name) = input["team_name"].as_str() else {
            return ToolOutput::error("Missing required parameter: team_name");
        };
        ToolOutput::success(format!("Team '{name}' deleted"))
    }
}
