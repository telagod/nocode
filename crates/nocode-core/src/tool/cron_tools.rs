//! Cron tools — CronCreate, CronDelete, CronList.

use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Debug, Clone)]
struct CronEntry {
    id: String,
    schedule: String,
    command: String,
}

struct CronRegistry {
    entries: HashMap<String, CronEntry>,
    next_id: u64,
}

impl CronRegistry {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            next_id: 1,
        }
    }
}

static GLOBAL_CRON: OnceLock<Arc<Mutex<CronRegistry>>> = OnceLock::new();

fn global_cron() -> &'static Arc<Mutex<CronRegistry>> {
    GLOBAL_CRON.get_or_init(|| Arc::new(Mutex::new(CronRegistry::new())))
}

pub struct CronCreateTool;

impl Tool for CronCreateTool {
    fn name(&self) -> &str {
        "CronCreate"
    }
    fn description(&self) -> &str {
        "Create a scheduled cron job."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "schedule":{"type":"string","description":"Cron schedule expression"},
            "command":{"type":"string","description":"Command to execute"}
        },"required":["schedule","command"]})
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(schedule) = input["schedule"].as_str() else {
            return ToolOutput::error("Missing: schedule");
        };
        let Some(command) = input["command"].as_str() else {
            return ToolOutput::error("Missing: command");
        };
        let reg = global_cron();
        let mut guard = reg.lock().unwrap();
        let id = format!("cron-{}", guard.next_id);
        guard.next_id += 1;
        guard.entries.insert(
            id.clone(),
            CronEntry {
                id: id.clone(),
                schedule: schedule.to_string(),
                command: command.to_string(),
            },
        );
        ToolOutput::success(json!({"id": id, "schedule": schedule, "command": command}).to_string())
    }
}

pub struct CronDeleteTool;

impl Tool for CronDeleteTool {
    fn name(&self) -> &str {
        "CronDelete"
    }
    fn description(&self) -> &str {
        "Delete a cron job by ID."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"id":{"type":"string"}},"required":["id"]})
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(id) = input["id"].as_str() else {
            return ToolOutput::error("Missing: id");
        };
        let reg = global_cron();
        let mut guard = reg.lock().unwrap();
        match guard.entries.remove(id) {
            Some(_) => ToolOutput::success(format!("Cron {id} deleted")),
            None => ToolOutput::error(format!("Cron {id} not found")),
        }
    }
}

pub struct CronListTool;

impl Tool for CronListTool {
    fn name(&self) -> &str {
        "CronList"
    }
    fn description(&self) -> &str {
        "List all cron jobs."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{}})
    }
    fn execute(&self, _input: &Value) -> ToolOutput {
        let reg = global_cron();
        let guard = reg.lock().unwrap();
        let list: Vec<Value> = guard
            .entries
            .values()
            .map(|e| {
                json!({
                    "id": e.id, "schedule": e.schedule, "command": e.command
                })
            })
            .collect();
        ToolOutput::success(serde_json::to_string(&list).unwrap_or_default())
    }
}
