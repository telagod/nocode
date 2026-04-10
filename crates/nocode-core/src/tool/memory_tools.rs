//! Memory tools — MemorySave, MemoryList, MemorySearch, MemoryDelete.

use crate::storage::memory::{MemoryEntry, MemoryStore, MemoryType};
use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};

fn default_memory_dir() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    format!("{home}/.nocode/memory")
}

pub struct MemorySaveTool;

impl Tool for MemorySaveTool {
    fn name(&self) -> &str {
        "MemorySave"
    }
    fn description(&self) -> &str {
        "Save a memory entry with YAML frontmatter."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "name":{"type":"string"},"description":{"type":"string"},
            "type":{"type":"string","enum":["user","feedback","project","reference"]},
            "content":{"type":"string"},"file_name":{"type":"string"}
        },"required":["name","description","type","content","file_name"]})
    }
    fn execute(&self, input: &Value) -> ToolOutput {
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
}

pub struct MemoryListTool;

impl Tool for MemoryListTool {
    fn name(&self) -> &str {
        "MemoryList"
    }
    fn description(&self) -> &str {
        "List all saved memory entries."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{}})
    }
    fn execute(&self, _input: &Value) -> ToolOutput {
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
}

pub struct MemorySearchTool;

impl Tool for MemorySearchTool {
    fn name(&self) -> &str {
        "MemorySearch"
    }
    fn description(&self) -> &str {
        "Search memory entries by keyword."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"]})
    }
    fn execute(&self, input: &Value) -> ToolOutput {
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
}

pub struct MemoryDeleteTool;

impl Tool for MemoryDeleteTool {
    fn name(&self) -> &str {
        "MemoryDelete"
    }
    fn description(&self) -> &str {
        "Delete a memory entry by file name."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"file_name":{"type":"string"}},"required":["file_name"]})
    }
    fn execute(&self, input: &Value) -> ToolOutput {
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
        let tool = MemorySaveTool;
        assert!(tool.execute(&json!({})).is_error);
        assert!(tool.execute(&json!({"name": "x"})).is_error);
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
        let tool = MemorySearchTool;
        assert!(tool.execute(&json!({})).is_error);
    }

    #[test]
    fn memory_search_succeeds() {
        let tool = MemorySearchTool;
        let result = tool.execute(&json!({"query": "nonexistent_xyz"}));
        assert!(!result.is_error);
    }

    #[test]
    fn memory_delete_missing_file() {
        let tool = MemoryDeleteTool;
        assert!(tool.execute(&json!({})).is_error);
    }
}
