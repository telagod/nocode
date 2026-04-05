use super::model::{
    ToolCallInput, ToolCallOutput, ToolCallResult, ToolExecutionTrace, ToolPermissionDecision,
    ToolProgressUpdate,
};
use crate::message::QueryMessage;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronEntry {
    pub id: String,
    pub schedule: String,
    pub command: String,
    pub created_at: u64,
}

pub struct CronRegistry {
    entries: Vec<CronEntry>,
    next_id: u64,
}

impl Default for CronRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl CronRegistry {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            next_id: 1,
        }
    }

    pub fn create(&mut self, schedule: &str, command: &str) -> CronEntry {
        let id = format!("cron-{}", self.next_id);
        self.next_id += 1;
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let entry = CronEntry {
            id,
            schedule: schedule.to_string(),
            command: command.to_string(),
            created_at,
        };
        self.entries.push(entry.clone());
        entry
    }

    pub fn delete(&mut self, id: &str) -> Option<CronEntry> {
        if let Some(pos) = self.entries.iter().position(|e| e.id == id) {
            Some(self.entries.remove(pos))
        } else {
            None
        }
    }

    pub fn list(&self) -> &[CronEntry] {
        &self.entries
    }
}

static CRON_REGISTRY: OnceLock<Arc<Mutex<CronRegistry>>> = OnceLock::new();

pub fn global_cron_registry() -> Arc<Mutex<CronRegistry>> {
    CRON_REGISTRY
        .get_or_init(|| Arc::new(Mutex::new(CronRegistry::new())))
        .clone()
}

fn missing_argument(call: ToolCallInput, key: &str) -> ToolExecutionTrace {
    ToolExecutionTrace {
        progress_updates: Vec::new(),
        result: ToolCallResult::failed(call, format!("missing required argument: {key}")),
        permission_denial: None,
    }
}

pub fn execute_cron_create(call: ToolCallInput) -> ToolExecutionTrace {
    let Some(schedule) = call.argument("schedule") else {
        return missing_argument(call, "schedule");
    };
    let Some(command) = call.argument("command") else {
        return missing_argument(call, "command");
    };
    let schedule = schedule.to_string();
    let command = command.to_string();
    let progress = ToolProgressUpdate::new(
        call.tool_use_id.clone(),
        format!("creating cron: {schedule} {command}"),
    );

    let registry = global_cron_registry();
    let entry = registry.lock().expect("lock poisoned").create(&schedule, &command);
    let summary = format!("created cron {} ({} -> {})", entry.id, entry.schedule, entry.command);

    ToolExecutionTrace {
        progress_updates: vec![progress],
        result: ToolPermissionDecision::allow(false).settle(
            call.clone(),
            ToolCallOutput {
                summary: summary.clone(),
                generated_messages: vec![QueryMessage::assistant(format!(
                    "tool-message: {summary}"
                ))],
                context_label: Some(call.context_label.clone()),
                progress_updates: vec![ToolProgressUpdate::new(
                    call.tool_use_id,
                    "cron create complete",
                )],
            },
        ),
        permission_denial: None,
    }
}

pub fn execute_cron_delete(call: ToolCallInput) -> ToolExecutionTrace {
    let Some(cron_id) = call.argument("cron_id") else {
        return missing_argument(call, "cron_id");
    };
    let cron_id = cron_id.to_string();
    let progress = ToolProgressUpdate::new(
        call.tool_use_id.clone(),
        format!("deleting cron: {cron_id}"),
    );

    let registry = global_cron_registry();
    let result = registry.lock().expect("lock poisoned").delete(&cron_id);

    match result {
        Some(entry) => {
            let summary = format!("deleted cron {} ({} -> {})", entry.id, entry.schedule, entry.command);
            ToolExecutionTrace {
                progress_updates: vec![progress],
                result: ToolPermissionDecision::allow(false).settle(
                    call.clone(),
                    ToolCallOutput {
                        summary: summary.clone(),
                        generated_messages: vec![QueryMessage::assistant(format!(
                            "tool-message: {summary}"
                        ))],
                        context_label: Some(call.context_label.clone()),
                        progress_updates: vec![ToolProgressUpdate::new(
                            call.tool_use_id,
                            "cron delete complete",
                        )],
                    },
                ),
                permission_denial: None,
            }
        }
        None => ToolExecutionTrace {
            progress_updates: vec![progress],
            result: ToolCallResult::failed(call, format!("cron entry not found: {cron_id}")),
            permission_denial: None,
        },
    }
}

pub fn execute_cron_list(call: ToolCallInput) -> ToolExecutionTrace {
    let progress = ToolProgressUpdate::new(call.tool_use_id.clone(), "listing cron entries");

    let registry = global_cron_registry();
    let guard = registry.lock().expect("lock poisoned");
    let entries = guard.list();
    let count = entries.len();
    let listing = if entries.is_empty() {
        String::from("no cron entries")
    } else {
        entries
            .iter()
            .map(|e| format!("{}: {} -> {}", e.id, e.schedule, e.command))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let summary = format!("listed {count} cron entries");
    ToolExecutionTrace {
        progress_updates: vec![progress],
        result: ToolPermissionDecision::allow(false).settle(
            call.clone(),
            ToolCallOutput {
                summary: summary.clone(),
                generated_messages: vec![QueryMessage::assistant(format!(
                    "tool-message: {summary}\n{listing}"
                ))],
                context_label: Some(call.context_label.clone()),
                progress_updates: vec![ToolProgressUpdate::new(
                    call.tool_use_id,
                    "cron list complete",
                )],
            },
        ),
        permission_denial: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_registry() -> Arc<Mutex<CronRegistry>> {
        Arc::new(Mutex::new(CronRegistry::new()))
    }

    #[test]
    fn cron_create_and_list() {
        let reg = fresh_registry();
        let mut guard = reg.lock().unwrap();
        let entry = guard.create("*/5 * * * *", "echo hello");
        assert_eq!(entry.schedule, "*/5 * * * *");
        assert_eq!(entry.command, "echo hello");
        assert_eq!(guard.list().len(), 1);
        assert_eq!(guard.list()[0].id, entry.id);
    }

    #[test]
    fn cron_delete_existing() {
        let reg = fresh_registry();
        let mut guard = reg.lock().unwrap();
        let entry = guard.create("0 * * * *", "backup.sh");
        let id = entry.id.clone();
        let deleted = guard.delete(&id);
        assert!(deleted.is_some());
        assert_eq!(deleted.unwrap().id, id);
        assert!(guard.list().is_empty());
    }

    #[test]
    fn cron_delete_nonexistent() {
        let reg = fresh_registry();
        let mut guard = reg.lock().unwrap();
        let deleted = guard.delete("cron-999");
        assert!(deleted.is_none());
    }

    #[test]
    fn cron_create_missing_schedule_fails() {
        let call = ToolCallInput::new("CronCreate", "toolu-cron-1")
            .with_argument("command", "echo hi")
            .with_context_label("test");
        let trace = execute_cron_create(call);
        assert_eq!(trace.result.status_label(), "failed");
        assert!(trace.result.message().contains("missing required argument: schedule"));
    }
}
