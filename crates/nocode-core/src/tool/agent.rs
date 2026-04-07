//! Agent tool — spawn subagent workers.

use crate::agent::worker::{WorkerState, global_worker_registry};
use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};

pub struct AgentTool;

impl Tool for AgentTool {
    fn name(&self) -> &str {
        "Agent"
    }
    fn description(&self) -> &str {
        "Launch a new agent to handle complex, multi-step tasks autonomously."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "prompt":{"type":"string","description":"The task for the agent to perform"},
            "name":{"type":"string","description":"Name for the spawned agent"},
            "subagent_type":{"type":"string","description":"Type of agent (general-purpose, Explore, Plan)"}
        },"required":["prompt"]})
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(prompt) = input["prompt"].as_str() else {
            return ToolOutput::error("Missing required parameter: prompt");
        };
        let name = input["name"].as_str().unwrap_or("agent");
        let registry = global_worker_registry();
        let mut guard = registry.lock().unwrap();
        let id = guard.register(name, prompt);
        guard.set_state(&id, WorkerState::Running);
        // TODO: actually spawn background thread with agentic loop
        ToolOutput::success(json!({"worker_id": id, "name": name, "status": "spawned"}).to_string())
    }
}
