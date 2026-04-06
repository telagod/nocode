use super::model::{
    ToolCallInput, ToolCallOutput, ToolCallResult, ToolExecutionTrace, ToolPermissionDecision,
    ToolProgressUpdate,
};
use crate::memory_store::{MemoryEntry, MemoryStore, MemoryType};
use crate::message::QueryMessage;

fn missing_argument(call: ToolCallInput, key: &str) -> ToolExecutionTrace {
    ToolExecutionTrace {
        progress_updates: Vec::new(),
        result: ToolCallResult::failed(call, format!("missing required argument: {key}")),
        permission_denial: None,
    }
}

fn ok_response(call: &ToolCallInput, tool_label: &str, summary: &str) -> ToolExecutionTrace {
    let progress = ToolProgressUpdate::new(call.tool_use_id.clone(), tool_label.to_string());
    ToolExecutionTrace {
        progress_updates: vec![progress],
        result: ToolPermissionDecision::allow(false).settle(
            call.clone(),
            ToolCallOutput {
                summary: summary.to_string(),
                generated_messages: vec![QueryMessage::assistant(format!(
                    "tool-message: {tool_label} — {summary}"
                ))],
                context_label: Some(call.context_label.clone()),
                progress_updates: vec![ToolProgressUpdate::new(
                    call.tool_use_id.clone(),
                    format!("{tool_label} complete"),
                )],
            },
        ),
        permission_denial: None,
    }
}

fn err_response(call: ToolCallInput, tool_label: &str, error: &str) -> ToolExecutionTrace {
    let progress = ToolProgressUpdate::new(call.tool_use_id.clone(), tool_label.to_string());
    ToolExecutionTrace {
        progress_updates: vec![progress],
        result: ToolCallResult::failed(call, error.to_string()),
        permission_denial: None,
    }
}

fn get_memory_store() -> MemoryStore {
    let base_dir = std::env::var("NOCODE_MEMORY_DIR").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        format!("{home}/.nocode/memory")
    });
    MemoryStore::new(&base_dir)
}

/// Save a memory entry to the persistent store.
pub fn execute_memory_save(call: ToolCallInput) -> ToolExecutionTrace {
    execute_memory_save_with_store(call, &get_memory_store())
}

/// Save — testable variant that accepts an explicit store.
pub fn execute_memory_save_with_store(
    call: ToolCallInput,
    store: &MemoryStore,
) -> ToolExecutionTrace {
    let Some(name) = call.argument("name") else {
        return missing_argument(call, "name");
    };
    let Some(description) = call.argument("description") else {
        return missing_argument(call, "description");
    };
    let Some(memory_type) = call.argument("memory_type") else {
        return missing_argument(call, "memory_type");
    };
    let Some(content) = call.argument("content") else {
        return missing_argument(call, "content");
    };
    let Some(file_name) = call.argument("file_name") else {
        return missing_argument(call, "file_name");
    };
    let name = name.to_string();
    let description = description.to_string();
    let memory_type_str = memory_type.to_string();
    let content = content.to_string();
    let file_name = file_name.to_string();

    let Some(mt) = MemoryType::parse(&memory_type_str) else {
        return err_response(
            call,
            "MemorySave",
            &format!("invalid memory_type: {memory_type_str}"),
        );
    };

    let entry = MemoryEntry {
        name: name.clone(),
        description,
        memory_type: mt,
        content,
        file_name: file_name.clone(),
    };

    if let Err(e) = store.save(&entry) {
        return err_response(call, "MemorySave", &e);
    }
    if let Err(e) = store.add_to_index(&entry) {
        return err_response(
            call,
            "MemorySave",
            &format!("saved but index update failed: {e}"),
        );
    }

    let summary = format!("memory saved: {name} -> {file_name}");
    ok_response(&call, "MemorySave", &summary)
}

/// List all memories from the persistent store.
pub fn execute_memory_list(call: ToolCallInput) -> ToolExecutionTrace {
    execute_memory_list_with_store(call, &get_memory_store())
}

/// List — testable variant that accepts an explicit store.
pub fn execute_memory_list_with_store(
    call: ToolCallInput,
    store: &MemoryStore,
) -> ToolExecutionTrace {
    let filter_type = call.argument("memory_type").map(ToString::to_string);

    let entries = match store.list() {
        Ok(entries) => entries,
        Err(e) => return err_response(call, "MemoryList", &e),
    };

    let filtered: Vec<&MemoryEntry> = if let Some(ref ft) = filter_type {
        if let Some(mt) = MemoryType::parse(ft) {
            entries.iter().filter(|e| e.memory_type == mt).collect()
        } else {
            return err_response(
                call,
                "MemoryList",
                &format!("invalid memory_type filter: {ft}"),
            );
        }
    } else {
        entries.iter().collect()
    };

    let count = filtered.len();
    let listing = if filtered.is_empty() {
        String::from("(no entries)")
    } else {
        filtered
            .iter()
            .map(|e| {
                format!(
                    "- {} [{}] ({})",
                    e.name,
                    e.memory_type.as_str(),
                    e.file_name
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    let summary = format!("memory list: {count} entries\n{listing}");
    ok_response(&call, "MemoryList", &summary)
}

/// Search memories by query.
pub fn execute_memory_search(call: ToolCallInput) -> ToolExecutionTrace {
    execute_memory_search_with_store(call, &get_memory_store())
}

/// Search — testable variant that accepts an explicit store.
pub fn execute_memory_search_with_store(
    call: ToolCallInput,
    store: &MemoryStore,
) -> ToolExecutionTrace {
    let Some(query) = call.argument("query") else {
        return missing_argument(call, "query");
    };
    let query = query.to_string();

    let results = match store.search(&query) {
        Ok(r) => r,
        Err(e) => return err_response(call, "MemorySearch", &e),
    };

    let count = results.len();
    let listing = if results.is_empty() {
        String::from("(no matches)")
    } else {
        results
            .iter()
            .map(|e| {
                format!(
                    "- {} [{}] ({}): {}",
                    e.name,
                    e.memory_type.as_str(),
                    e.file_name,
                    e.description
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    let summary = format!("memory search: {count} matches for '{query}'\n{listing}");
    ok_response(&call, "MemorySearch", &summary)
}

/// Delete a memory entry.
pub fn execute_memory_delete(call: ToolCallInput) -> ToolExecutionTrace {
    execute_memory_delete_with_store(call, &get_memory_store())
}

/// Delete — testable variant that accepts an explicit store.
pub fn execute_memory_delete_with_store(
    call: ToolCallInput,
    store: &MemoryStore,
) -> ToolExecutionTrace {
    let Some(file_name) = call.argument("file_name") else {
        return missing_argument(call, "file_name");
    };
    let file_name = file_name.to_string();

    if let Err(e) = store.delete(&file_name) {
        return err_response(call, "MemoryDelete", &e);
    }
    if let Err(e) = store.remove_from_index(&file_name) {
        return err_response(
            call,
            "MemoryDelete",
            &format!("deleted but index update failed: {e}"),
        );
    }

    let summary = format!("memory deleted: {file_name}");
    ok_response(&call, "MemoryDelete", &summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_execution::ToolCallInput;

    fn temp_store() -> (tempfile::TempDir, MemoryStore) {
        let tmp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(tmp.path().to_str().unwrap());
        (tmp, store)
    }

    #[test]
    fn memory_save_returns_confirmation() {
        let (_tmp, store) = temp_store();
        let call = ToolCallInput::new("MemorySave", "ms1")
            .with_argument("name", "test-memory")
            .with_argument("description", "a test memory")
            .with_argument("memory_type", "user")
            .with_argument("content", "some content")
            .with_argument("file_name", "test.md")
            .with_context_label("test");
        let trace = execute_memory_save_with_store(call, &store);
        assert_eq!(trace.result.status_label(), "completed");
        assert!(
            trace
                .result
                .message()
                .contains("memory saved: test-memory -> test.md")
        );
    }

    #[test]
    fn memory_list_returns_saved_entries() {
        let (_tmp, store) = temp_store();
        let save_call = ToolCallInput::new("MemorySave", "ms-l1")
            .with_argument("name", "list-test")
            .with_argument("description", "for listing")
            .with_argument("memory_type", "feedback")
            .with_argument("content", "feedback content")
            .with_argument("file_name", "list_test.md")
            .with_context_label("test");
        execute_memory_save_with_store(save_call, &store);

        let list_call = ToolCallInput::new("MemoryList", "ml1").with_context_label("test");
        let trace = execute_memory_list_with_store(list_call, &store);
        assert_eq!(trace.result.status_label(), "completed");
        assert!(trace.result.message().contains("1 entries"));
        assert!(trace.result.message().contains("list-test"));
    }

    #[test]
    fn memory_list_filters_by_type() {
        let (_tmp, store) = temp_store();
        let save1 = ToolCallInput::new("MemorySave", "ms-f1")
            .with_argument("name", "user-entry")
            .with_argument("description", "user desc")
            .with_argument("memory_type", "user")
            .with_argument("content", "user content")
            .with_argument("file_name", "user_entry.md")
            .with_context_label("test");
        execute_memory_save_with_store(save1, &store);

        let save2 = ToolCallInput::new("MemorySave", "ms-f2")
            .with_argument("name", "feedback-entry")
            .with_argument("description", "feedback desc")
            .with_argument("memory_type", "feedback")
            .with_argument("content", "feedback content")
            .with_argument("file_name", "feedback_entry.md")
            .with_context_label("test");
        execute_memory_save_with_store(save2, &store);

        let list_call = ToolCallInput::new("MemoryList", "ml-f1")
            .with_argument("memory_type", "user")
            .with_context_label("test");
        let trace = execute_memory_list_with_store(list_call, &store);
        assert_eq!(trace.result.status_label(), "completed");
        assert!(trace.result.message().contains("1 entries"));
        assert!(trace.result.message().contains("user-entry"));
    }

    #[test]
    fn memory_search_finds_match() {
        let (_tmp, store) = temp_store();
        let save_call = ToolCallInput::new("MemorySave", "ms-s1")
            .with_argument("name", "search-target")
            .with_argument("description", "searchable desc")
            .with_argument("memory_type", "project")
            .with_argument("content", "unique-keyword-xyz")
            .with_argument("file_name", "search_target.md")
            .with_context_label("test");
        execute_memory_save_with_store(save_call, &store);

        let search_call = ToolCallInput::new("MemorySearch", "ms2")
            .with_argument("query", "unique-keyword")
            .with_context_label("test");
        let trace = execute_memory_search_with_store(search_call, &store);
        assert_eq!(trace.result.status_label(), "completed");
        assert!(trace.result.message().contains("1 matches"));
        assert!(trace.result.message().contains("search-target"));
    }

    #[test]
    fn memory_search_returns_empty_on_no_match() {
        let (_tmp, store) = temp_store();
        let search_call = ToolCallInput::new("MemorySearch", "ms-e1")
            .with_argument("query", "nonexistent-query")
            .with_context_label("test");
        let trace = execute_memory_search_with_store(search_call, &store);
        assert_eq!(trace.result.status_label(), "completed");
        assert!(trace.result.message().contains("0 matches"));
    }

    #[test]
    fn memory_delete_returns_confirmation() {
        let (_tmp, store) = temp_store();
        let save_call = ToolCallInput::new("MemorySave", "ms-d1")
            .with_argument("name", "to-delete")
            .with_argument("description", "will be deleted")
            .with_argument("memory_type", "reference")
            .with_argument("content", "delete me")
            .with_argument("file_name", "to_delete.md")
            .with_context_label("test");
        execute_memory_save_with_store(save_call, &store);

        let delete_call = ToolCallInput::new("MemoryDelete", "md1")
            .with_argument("file_name", "to_delete.md")
            .with_context_label("test");
        let trace = execute_memory_delete_with_store(delete_call, &store);
        assert_eq!(trace.result.status_label(), "completed");
        assert!(
            trace
                .result
                .message()
                .contains("memory deleted: to_delete.md")
        );
    }

    #[test]
    fn memory_save_missing_name_fails() {
        let (_tmp, store) = temp_store();
        let call = ToolCallInput::new("MemorySave", "ms3")
            .with_argument("description", "desc")
            .with_argument("memory_type", "user")
            .with_argument("content", "content")
            .with_argument("file_name", "f.md")
            .with_context_label("test");
        let trace = execute_memory_save_with_store(call, &store);
        assert_eq!(trace.result.status_label(), "failed");
        assert!(
            trace
                .result
                .message()
                .contains("missing required argument: name")
        );
    }

    #[test]
    fn memory_search_missing_query_fails() {
        let (_tmp, store) = temp_store();
        let call = ToolCallInput::new("MemorySearch", "ms4").with_context_label("test");
        let trace = execute_memory_search_with_store(call, &store);
        assert_eq!(trace.result.status_label(), "failed");
        assert!(
            trace
                .result
                .message()
                .contains("missing required argument: query")
        );
    }

    #[test]
    fn memory_save_invalid_type_fails() {
        let (_tmp, store) = temp_store();
        let call = ToolCallInput::new("MemorySave", "ms-it")
            .with_argument("name", "bad-type")
            .with_argument("description", "desc")
            .with_argument("memory_type", "bogus")
            .with_argument("content", "content")
            .with_argument("file_name", "bad.md")
            .with_context_label("test");
        let trace = execute_memory_save_with_store(call, &store);
        assert_eq!(trace.result.status_label(), "failed");
        assert!(trace.result.message().contains("invalid memory_type"));
    }
}
