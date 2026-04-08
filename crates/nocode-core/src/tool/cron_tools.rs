//! Cron tools — CronCreate, CronDelete, CronList + scheduler.

use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// Parsed cron schedule — supports: `*/N * * * *` (every N minutes),
/// `N * * * *` (at minute N), `* * * * *` (every minute).
#[derive(Debug, Clone)]
pub struct CronSchedule {
    /// Minutes to match (0-59). Empty = every minute.
    pub minutes: Vec<u32>,
}

impl CronSchedule {
    /// Parse a cron expression (minute field only for now).
    pub fn parse(expr: &str) -> Result<Self, String> {
        let parts: Vec<&str> = expr.split_whitespace().collect();
        if parts.len() < 5 {
            return Err(format!("Invalid cron expression (need 5 fields): {expr}"));
        }
        let minute_field = parts[0];
        let minutes = Self::parse_field(minute_field, 0, 59)?;
        Ok(Self { minutes })
    }

    fn parse_field(field: &str, min: u32, max: u32) -> Result<Vec<u32>, String> {
        if field == "*" {
            return Ok(Vec::new()); // empty = match all
        }
        if let Some(step) = field.strip_prefix("*/") {
            let step: u32 = step.parse().map_err(|_| format!("Invalid step: {field}"))?;
            if step == 0 || step > max {
                return Err(format!("Invalid step value: {step}"));
            }
            return Ok((min..=max).step_by(step as usize).collect());
        }
        // Comma-separated values
        let mut values = Vec::new();
        for part in field.split(',') {
            let v: u32 = part
                .trim()
                .parse()
                .map_err(|_| format!("Invalid value: {part}"))?;
            if v < min || v > max {
                return Err(format!("Value {v} out of range {min}-{max}"));
            }
            values.push(v);
        }
        Ok(values)
    }

    /// Check if the given minute matches this schedule.
    pub fn matches_minute(&self, minute: u32) -> bool {
        self.minutes.is_empty() || self.minutes.contains(&minute)
    }
}

#[derive(Debug, Clone)]
pub struct CronEntry {
    pub id: String,
    pub schedule: String,
    pub parsed: CronSchedule,
    pub command: String,
    pub last_run: Option<String>,
    pub run_count: u64,
    pub enabled: bool,
}

pub struct CronRegistry {
    entries: HashMap<String, CronEntry>,
    next_id: u64,
}

impl CronRegistry {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            next_id: 1,
        }
    }

    /// Add a cron entry. Returns the assigned ID.
    pub fn add(&mut self, schedule: &str, command: &str) -> Result<String, String> {
        let parsed = CronSchedule::parse(schedule)?;
        let id = format!("cron-{}", self.next_id);
        self.next_id += 1;
        self.entries.insert(
            id.clone(),
            CronEntry {
                id: id.clone(),
                schedule: schedule.to_string(),
                parsed,
                command: command.to_string(),
                last_run: None,
                run_count: 0,
                enabled: true,
            },
        );
        Ok(id)
    }

    /// Remove a cron entry by ID.
    pub fn remove(&mut self, id: &str) -> Option<CronEntry> {
        self.entries.remove(id)
    }

    /// List all entries.
    pub fn list(&self) -> Vec<&CronEntry> {
        self.entries.values().collect()
    }

    /// Get entries that should fire at the given minute.
    pub fn due_entries(&self, minute: u32) -> Vec<&CronEntry> {
        self.entries
            .values()
            .filter(|e| e.enabled && e.parsed.matches_minute(minute))
            .collect()
    }

    /// Record that an entry was executed.
    pub fn record_run(&mut self, id: &str) {
        if let Some(entry) = self.entries.get_mut(id) {
            entry.run_count += 1;
            entry.last_run = Some(chrono::Utc::now().to_rfc3339());
        }
    }

    /// Execute all due cron jobs for the given minute. Returns (id, command, output).
    pub fn tick(&mut self, minute: u32) -> Vec<(String, String, String)> {
        let due: Vec<(String, String)> = self
            .due_entries(minute)
            .iter()
            .map(|e| (e.id.clone(), e.command.clone()))
            .collect();

        let mut results = Vec::new();
        for (id, command) in due {
            let output = match std::process::Command::new("sh")
                .arg("-c")
                .arg(&command)
                .output()
            {
                Ok(o) => {
                    let stdout = String::from_utf8_lossy(&o.stdout).to_string();
                    let stderr = String::from_utf8_lossy(&o.stderr).to_string();
                    if o.status.success() {
                        stdout
                    } else {
                        format!("exit {}: {stderr}", o.status.code().unwrap_or(-1))
                    }
                }
                Err(e) => format!("exec error: {e}"),
            };
            self.record_run(&id);
            results.push((id, command, output));
        }
        results
    }
}

impl Default for CronRegistry {
    fn default() -> Self {
        Self::new()
    }
}

static GLOBAL_CRON: OnceLock<Arc<Mutex<CronRegistry>>> = OnceLock::new();

pub fn global_cron_registry() -> &'static Arc<Mutex<CronRegistry>> {
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
        let reg = global_cron_registry();
        let mut guard = reg.lock().unwrap();
        match guard.add(schedule, command) {
            Ok(id) => ToolOutput::success(
                json!({"id": id, "schedule": schedule, "command": command}).to_string(),
            ),
            Err(e) => ToolOutput::error(format!("Invalid schedule: {e}")),
        }
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
        let reg = global_cron_registry();
        let mut guard = reg.lock().unwrap();
        match guard.remove(id) {
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
        let reg = global_cron_registry();
        let guard = reg.lock().unwrap();
        let list: Vec<Value> = guard
            .list()
            .iter()
            .map(|e| {
                json!({
                    "id": e.id,
                    "schedule": e.schedule,
                    "command": e.command,
                    "enabled": e.enabled,
                    "run_count": e.run_count,
                    "last_run": e.last_run,
                })
            })
            .collect();
        ToolOutput::success(serde_json::to_string(&list).unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cron_create_and_list() {
        let create = CronCreateTool;
        let result = create.execute(&json!({"schedule": "*/5 * * * *", "command": "echo hi"}));
        assert!(!result.is_error);
        assert!(result.content.contains("cron-"));

        let list = CronListTool;
        let result = list.execute(&json!({}));
        assert!(!result.is_error);
        assert!(result.content.contains("echo hi"));
    }

    #[test]
    fn cron_delete() {
        let create = CronCreateTool;
        let r = create.execute(&json!({"schedule": "0 * * * *", "command": "test"}));
        let v: serde_json::Value = serde_json::from_str(&r.content).unwrap();
        let id = v["id"].as_str().unwrap();

        let delete = CronDeleteTool;
        let result = delete.execute(&json!({"id": id}));
        assert!(!result.is_error);
    }

    #[test]
    fn cron_delete_nonexistent() {
        let delete = CronDeleteTool;
        let result = delete.execute(&json!({"id": "cron-99999"}));
        assert!(result.is_error);
    }

    #[test]
    fn cron_create_missing_params() {
        let tool = CronCreateTool;
        assert!(tool.execute(&json!({})).is_error);
        assert!(tool.execute(&json!({"schedule": "* * * * *"})).is_error);
    }

    #[test]
    fn cron_create_invalid_schedule() {
        let tool = CronCreateTool;
        let result = tool.execute(&json!({"schedule": "bad", "command": "echo hi"}));
        assert!(result.is_error);
        assert!(result.content.contains("Invalid schedule"));
    }

    // --- CronSchedule parsing ---
    #[test]
    fn parse_every_minute() {
        let s = CronSchedule::parse("* * * * *").unwrap();
        assert!(s.minutes.is_empty()); // empty = match all
        assert!(s.matches_minute(0));
        assert!(s.matches_minute(30));
        assert!(s.matches_minute(59));
    }

    #[test]
    fn parse_every_n_minutes() {
        let s = CronSchedule::parse("*/5 * * * *").unwrap();
        assert_eq!(
            s.minutes,
            vec![0, 5, 10, 15, 20, 25, 30, 35, 40, 45, 50, 55]
        );
        assert!(s.matches_minute(0));
        assert!(s.matches_minute(15));
        assert!(!s.matches_minute(3));
    }

    #[test]
    fn parse_specific_minute() {
        let s = CronSchedule::parse("30 * * * *").unwrap();
        assert_eq!(s.minutes, vec![30]);
        assert!(s.matches_minute(30));
        assert!(!s.matches_minute(0));
    }

    #[test]
    fn parse_comma_separated() {
        let s = CronSchedule::parse("0,15,30,45 * * * *").unwrap();
        assert_eq!(s.minutes, vec![0, 15, 30, 45]);
    }

    #[test]
    fn parse_invalid_expression() {
        assert!(CronSchedule::parse("bad").is_err());
        assert!(CronSchedule::parse("*/0 * * * *").is_err());
        assert!(CronSchedule::parse("99 * * * *").is_err());
    }

    // --- tick execution ---
    #[test]
    fn tick_executes_due_entries() {
        let mut reg = CronRegistry::new();
        reg.add("*/5 * * * *", "echo tick_test").unwrap();
        let results = reg.tick(10); // minute 10 matches */5
        assert_eq!(results.len(), 1);
        assert!(results[0].2.contains("tick_test"));
    }

    #[test]
    fn tick_skips_non_due() {
        let mut reg = CronRegistry::new();
        reg.add("30 * * * *", "echo nope").unwrap();
        let results = reg.tick(15); // minute 15 doesn't match 30
        assert!(results.is_empty());
    }

    #[test]
    fn tick_records_run_count() {
        let mut reg = CronRegistry::new();
        let id = reg.add("* * * * *", "echo count").unwrap();
        reg.tick(0);
        reg.tick(1);
        let entry = reg.entries.get(&id).unwrap();
        assert_eq!(entry.run_count, 2);
        assert!(entry.last_run.is_some());
    }
}
