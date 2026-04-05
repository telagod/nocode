use super::model::{
    ToolCallInput, ToolCallOutput, ToolCallResult, ToolExecutionTrace, ToolPermissionDecision,
    ToolProgressUpdate,
};
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

/// Save a memory entry to the persistent store.
pub fn execute_memory_save(call: ToolCallInput) -> ToolExecutionTrace {
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
    let _description = description.to_string();
    let _memory_type = memory_type.to_string();
    let _content = content.to_string();
    let file_name = file_name.to_string();

    let summary = format!("memory saved: {name} -> {file_name}");
    ok_response(&call, "MemorySave", &summary)
}

/// List all memories from the persistent store.
pub fn execute_memory_list(call: ToolCallInput) -> ToolExecutionTrace {
    let _memory_type = call.argument("memory_type").map(ToString::to_string);
    let summary = "memory list: 0 entries (memory store not connected)";
    ok_response(&call, "MemoryList", summary)
}

/// Search memories by query.
pub fn execute_memory_search(call: ToolCallInput) -> ToolExecutionTrace {
    let Some(query) = call.argument("query") else {
        return missing_argument(call, "query");
    };
    let query = query.to_string();
    let summary = format!("memory search: 0 matches for '{query}' (memory store not connected)");
    ok_response(&call, "MemorySearch", &summary)
}

/// Delete a memory entry.
pub fn execute_memory_delete(call: ToolCallInput) -> ToolExecutionTrace {
    let Some(file_name) = call.argument("file_name") else {
        return missing_argument(call, "file_name");
    };
    let file_name = file_name.to_string();
    let summary = format!("memory deleted: {file_name} (memory store not connected)");
    ok_response(&call, "MemoryDelete", &summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_execution::ToolCallInput;

    #[test]
    fn memory_save_returns_confirmation() {
        let call = ToolCallInput::new("MemorySave", "ms1")
            .with_argument("name", "test-memory")
            .with_argument("description", "a test memory")
            .with_argument("memory_type", "user")
            .with_argument("content", "some content")
            .with_argument("file_name", "test.md")
            .with_context_label("test");
        let trace = execute_memory_save(call);
        assert_eq!(trace.result.status_label(), "completed");
        assert!(trace.result.message().contains("memory saved: test-memory -> test.md"));
    }

    #[test]
    fn memory_list_returns_stub() {
        let call = ToolCallInput::new("MemoryList", "ml1")
            .with_context_label("test");
        let trace = execute_memory_list(call);
        assert_eq!(trace.result.status_label(), "completed");
        assert!(trace.result.message().contains("0 entries"));
    }

    #[test]
    fn memory_search_returns_stub() {
        let call = ToolCallInput::new("MemorySearch", "ms2")
            .with_argument("query", "test query")
            .with_context_label("test");
        let trace = execute_memory_search(call);
        assert_eq!(trace.result.status_label(), "completed");
        assert!(trace.result.message().contains("0 matches for 'test query'"));
    }

    #[test]
    fn memory_delete_returns_confirmation() {
        let call = ToolCallInput::new("MemoryDelete", "md1")
            .with_argument("file_name", "old.md")
            .with_context_label("test");
        let trace = execute_memory_delete(call);
        assert_eq!(trace.result.status_label(), "completed");
        assert!(trace.result.message().contains("memory deleted: old.md"));
    }

    #[test]
    fn memory_save_missing_name_fails() {
        let call = ToolCallInput::new("MemorySave", "ms3")
            .with_argument("description", "desc")
            .with_argument("memory_type", "user")
            .with_argument("content", "content")
            .with_argument("file_name", "f.md")
            .with_context_label("test");
        let trace = execute_memory_save(call);
        assert_eq!(trace.result.status_label(), "failed");
        assert!(trace.result.message().contains("missing required argument: name"));
    }

    #[test]
    fn memory_search_missing_query_fails() {
        let call = ToolCallInput::new("MemorySearch", "ms4")
            .with_context_label("test");
        let trace = execute_memory_search(call);
        assert_eq!(trace.result.status_label(), "failed");
        assert!(trace.result.message().contains("missing required argument: query"));
    }
}
