//! Team tools — TeamCreate, TeamDelete with file system persistence.

use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;

/// Resolve the base directory for team/task storage: ~/.nocode/
fn nocode_home() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| String::from("."));
    PathBuf::from(home).join(".nocode")
}

fn team_config_dir(team_name: &str) -> PathBuf {
    nocode_home().join("teams").join(team_name)
}

fn task_list_dir(team_name: &str) -> PathBuf {
    nocode_home().join("tasks").join(team_name)
}

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

        // Validate team name (alphanumeric + hyphens only)
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            return ToolOutput::error(
                "Team name must be non-empty and contain only alphanumeric, hyphen, or underscore characters",
            );
        }

        let config_dir = team_config_dir(name);
        let tasks_dir = task_list_dir(name);

        // Check if team already exists
        if config_dir.exists() {
            return ToolOutput::error(format!("Team '{name}' already exists"));
        }

        // Create directories
        if let Err(e) = fs::create_dir_all(&config_dir) {
            return ToolOutput::error(format!("Failed to create team directory: {e}"));
        }
        if let Err(e) = fs::create_dir_all(&tasks_dir) {
            return ToolOutput::error(format!("Failed to create task directory: {e}"));
        }

        // Write config.json
        let config = json!({
            "name": name,
            "description": desc,
            "created_at": chrono::Utc::now().to_rfc3339(),
        });
        let config_path = config_dir.join("config.json");
        if let Err(e) = fs::write(&config_path, serde_json::to_string_pretty(&config).unwrap()) {
            return ToolOutput::error(format!("Failed to write team config: {e}"));
        }

        ToolOutput::success(
            json!({
                "team_name": name,
                "description": desc,
                "config_dir": config_dir.to_string_lossy(),
                "tasks_dir": tasks_dir.to_string_lossy(),
            })
            .to_string(),
        )
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
            "team_name":{"type":"string","description":"Name of the team to delete"}
        },"required":["team_name"]})
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(name) = input["team_name"].as_str() else {
            return ToolOutput::error("Missing required parameter: team_name");
        };

        let config_dir = team_config_dir(name);
        let tasks_dir = task_list_dir(name);

        if !config_dir.exists() {
            return ToolOutput::error(format!("Team '{name}' not found"));
        }

        // Remove team config directory
        if let Err(e) = fs::remove_dir_all(&config_dir) {
            return ToolOutput::error(format!("Failed to remove team config: {e}"));
        }

        // Remove task list directory (if exists)
        if tasks_dir.exists()
            && let Err(e) = fs::remove_dir_all(&tasks_dir)
        {
            return ToolOutput::error(format!("Failed to remove task directory: {e}"));
        }

        ToolOutput::success(format!("Team '{name}' deleted"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_create_validates_name() {
        let tool = TeamCreateTool;
        let result = tool.execute(&json!({"team_name": ""}));
        assert!(result.is_error);

        let result = tool.execute(&json!({"team_name": "bad name!"}));
        assert!(result.is_error);
    }

    #[test]
    fn team_create_and_delete_roundtrip() {
        // Reads $HOME — must serialize with other env-mutating tests.
        let _guard = crate::test_support::env_mutex().lock().unwrap();
        let create = TeamCreateTool;
        let delete = TeamDeleteTool;

        let name = format!("test-team-{}", std::process::id());
        let result = create.execute(&json!({"team_name": name, "description": "test"}));
        assert!(!result.is_error, "Create failed: {}", result.content);

        // Verify directories exist
        assert!(team_config_dir(&name).exists());
        assert!(task_list_dir(&name).exists());
        assert!(team_config_dir(&name).join("config.json").exists());

        // Delete
        let result = delete.execute(&json!({"team_name": name}));
        assert!(!result.is_error, "Delete failed: {}", result.content);

        // Verify cleaned up
        assert!(!team_config_dir(&name).exists());
        assert!(!task_list_dir(&name).exists());
    }

    #[test]
    fn team_delete_nonexistent() {
        let tool = TeamDeleteTool;
        let result = tool.execute(&json!({"team_name": "nonexistent-team-xyz"}));
        assert!(result.is_error);
    }
}
