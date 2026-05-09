//! Unified Memory tool — save, list, search, delete memory entries.

use crate::storage::memory::{MemoryEntry, MemoryStore, MemoryType};
use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};

fn default_memory_dir() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    format!("{home}/.nocode/memory")
}

pub struct MemoryTool;

impl Tool for MemoryTool {
    fn name(&self) -> &str {
        "Memory"
    }
    fn description(&self) -> &str {
        "Manage persistent memory entries. Actions: save (create/update a memory entry), \
         list (show all entries), search (find by keyword), delete (remove by file name)."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["save", "list", "delete", "search"],
                    "description": "The memory operation to perform"
                },
                "name": {
                    "type": "string",
                    "description": "Memory entry name (required for save)"
                },
                "description": {
                    "type": "string",
                    "description": "Memory entry description (required for save)"
                },
                "type": {
                    "type": "string",
                    "enum": ["user", "feedback", "project", "reference"],
                    "description": "Memory type (required for save)"
                },
                "content": {
                    "type": "string",
                    "description": "Memory content to save (required for save)"
                },
                "file_name": {
                    "type": "string",
                    "description": "File name for the memory entry (required for save and delete)"
                },
                "query": {
                    "type": "string",
                    "description": "Search query keyword (required for search)"
                }
            },
            "required": ["action"]
        })
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(action) = input["action"].as_str() else {
            return ToolOutput::error("Missing required parameter: action");
        };
        match action {
            "save" => self.execute_save(input),
            "list" => self.execute_list(),
            "search" => self.execute_search(input),
            "delete" => self.execute_delete(input),
            _ => ToolOutput::error(format!(
                "Unknown action: {action}. Use save, list, search, or delete."
            )),
        }
    }
}

impl MemoryTool {
    fn execute_save(&self, input: &Value) -> ToolOutput {
        let name = input["name"].as_str().unwrap_or("");
        let desc = input["description"].as_str().unwrap_or("");
        let mtype = input["type"].as_str().unwrap_or("user");
        let content = input["content"].as_str().unwrap_or("");
        let file_name = input["file_name"].as_str().unwrap_or("");
        let Some(memory_type) = MemoryType::parse(mtype) else {
            return ToolOutput::error(format!("Invalid memory type: {mtype}"));
        };
        let entry = MemoryEntry {
            name: name.to_string(),
            description: desc.to_string(),
            memory_type,
            content: content.to_string(),
            file_name: file_name.to_string(),
        };
        let store = MemoryStore::new(&default_memory_dir());
        if let Err(e) = store.save(&entry) {
            return ToolOutput::error(e);
        }
        if let Err(e) = store.add_to_index(&entry) {
            return ToolOutput::error(e);
        }
        ToolOutput::success(format!("Saved memory: {file_name}"))
    }

    fn execute_list(&self) -> ToolOutput {
        let store = MemoryStore::new(&default_memory_dir());
        match store.list() {
            Ok(entries) => {
                let list: Vec<Value> = entries
                    .iter()
                    .map(|e| {
                        json!({
                            "name": e.name, "type": e.memory_type.as_str(),
                            "file": e.file_name, "description": e.description
                        })
                    })
                    .collect();
                ToolOutput::success(serde_json::to_string(&list).unwrap_or_default())
            }
            Err(e) => ToolOutput::error(e),
        }
    }

    fn execute_search(&self, input: &Value) -> ToolOutput {
        let Some(query) = input["query"].as_str() else {
            return ToolOutput::error("Missing required parameter: query");
        };
        let store = MemoryStore::new(&default_memory_dir());
        match store.search(query) {
            Ok(entries) => {
                let list: Vec<Value> = entries
                    .iter()
                    .map(|e| {
                        json!({
                            "name": e.name, "type": e.memory_type.as_str(),
                            "file": e.file_name, "description": e.description,
                            "content": e.content
                        })
                    })
                    .collect();
                ToolOutput::success(serde_json::to_string(&list).unwrap_or_default())
            }
            Err(e) => ToolOutput::error(e),
        }
    }

    fn execute_delete(&self, input: &Value) -> ToolOutput {
        let Some(file_name) = input["file_name"].as_str() else {
            return ToolOutput::error("Missing required parameter: file_name");
        };
        let store = MemoryStore::new(&default_memory_dir());
        if let Err(e) = store.delete(file_name) {
            return ToolOutput::error(e);
        }
        if let Err(e) = store.remove_from_index(file_name) {
            return ToolOutput::error(e);
        }
        ToolOutput::success(format!("Deleted memory: {file_name}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn memory_save_missing_params() {
        let tool = MemoryTool;
        // Missing action
        assert!(tool.execute(&json!({})).is_error);
        // Save with no other params — invalid type triggers error
        let result = tool.execute(&json!({"action": "save", "type": "bad"}));
        assert!(result.is_error);
    }

    #[test]
    fn memory_list_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(dir.path().to_str().unwrap());
        let result = match store.list() {
            Ok(entries) => {
                let list: Vec<Value> = entries
                    .iter()
                    .map(|e| {
                        json!({
                            "name": e.name, "type": e.memory_type.as_str(),
                            "file": e.file_name, "description": e.description
                        })
                    })
                    .collect();
                ToolOutput::success(serde_json::to_string(&list).unwrap_or_default())
            }
            Err(e) => ToolOutput::error(e),
        };
        assert!(!result.is_error);
    }

    #[test]
    fn memory_search_missing_query() {
        let tool = MemoryTool;
        assert!(tool.execute(&json!({"action": "search"})).is_error);
    }

    #[test]
    fn memory_search_succeeds() {
        let tool = MemoryTool;
        let result = tool.execute(&json!({"action": "search", "query": "nonexistent_xyz"}));
        assert!(!result.is_error);
    }

    #[test]
    fn memory_delete_missing_file() {
        let tool = MemoryTool;
        assert!(tool.execute(&json!({"action": "delete"})).is_error);
    }

    #[test]
    fn memory_unknown_action() {
        let tool = MemoryTool;
        let result = tool.execute(&json!({"action": "frobnicate"}));
        assert!(result.is_error);
        assert!(result.content.contains("Unknown action"));
    }
}
