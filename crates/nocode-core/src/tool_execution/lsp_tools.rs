use super::model::{
    ToolCallInput, ToolCallOutput, ToolCallResult, ToolExecutionTrace, ToolPermissionDecision,
    ToolProgressUpdate,
};
use crate::lsp_client::{LspAction, LspResult, global_lsp_registry};
use crate::message::QueryMessage;

fn missing_argument(call: ToolCallInput, key: &str) -> ToolExecutionTrace {
    ToolExecutionTrace {
        progress_updates: Vec::new(),
        result: ToolCallResult::failed(call, format!("missing required argument: {key}")),
        permission_denial: None,
    }
}

fn format_lsp_result(result: &LspResult) -> String {
    match result {
        LspResult::Diagnostics(diags) => {
            if diags.is_empty() {
                return "no diagnostics".to_string();
            }
            diags
                .iter()
                .map(|d| {
                    format!(
                        "{}:{}:{} [{}] {}",
                        d.file, d.line, d.column, d.severity, d.message
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        LspResult::Hover(hover) => match hover {
            Some(h) => h.content.clone(),
            None => "no hover information".to_string(),
        },
        LspResult::Definition(locs) | LspResult::References(locs) => {
            if locs.is_empty() {
                return "no locations found".to_string();
            }
            locs.iter()
                .map(|l| format!("{}:{}:{}", l.file, l.line, l.column))
                .collect::<Vec<_>>()
                .join("\n")
        }
        LspResult::Completion(items) => {
            if items.is_empty() {
                return "no completions".to_string();
            }
            items
                .iter()
                .map(|c| {
                    let detail = c.detail.as_deref().unwrap_or("");
                    format!("{} ({}) {}", c.label, c.kind, detail)
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        LspResult::Symbols(syms) => {
            if syms.is_empty() {
                return "no symbols found".to_string();
            }
            syms.iter()
                .map(|s| {
                    format!(
                        "{} ({}) {}:{}:{}",
                        s.name, s.kind, s.location.file, s.location.line, s.location.column
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
    }
}

pub fn execute_lsp(call: ToolCallInput) -> ToolExecutionTrace {
    let Some(action_str) = call.argument("action") else {
        return missing_argument(call, "action");
    };
    let Some(file_path) = call.argument("file_path") else {
        return missing_argument(call, "file_path");
    };
    let action_str = action_str.to_string();
    let file_path = file_path.to_string();
    let line: u32 = call
        .argument("line")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let column: u32 = call
        .argument("column")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let progress = ToolProgressUpdate::new(
        call.tool_use_id.clone(),
        format!("lsp {action_str} on {file_path}"),
    );

    let action = match LspAction::parse(&action_str) {
        Ok(a) => a,
        Err(e) => {
            return ToolExecutionTrace {
                progress_updates: vec![progress],
                result: ToolCallResult::failed(call, e),
                permission_denial: None,
            };
        }
    };

    // Detect language from file extension and find a server.
    let ext = file_path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_string();
    let lang = match ext.as_str() {
        "rs" => "rust",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" => "javascript",
        "py" => "python",
        "go" => "go",
        other => other,
    };

    let registry = global_lsp_registry();
    let guard = registry.lock().expect("lock poisoned");

    let server_name = match guard.find_for_language(lang) {
        Some(s) => s.name.clone(),
        None => {
            return ToolExecutionTrace {
                progress_updates: vec![progress],
                result: ToolCallResult::failed(
                    call,
                    format!("no connected LSP server for language: {lang}"),
                ),
                permission_denial: None,
            };
        }
    };

    match guard.execute(&server_name, action, &file_path, line, column) {
        Ok(result) => {
            let formatted = format_lsp_result(&result);
            let summary = format!("lsp {action_str} on {file_path} via {server_name}");
            ToolExecutionTrace {
                progress_updates: vec![progress],
                result: ToolPermissionDecision::allow(false).settle(
                    call.clone(),
                    ToolCallOutput {
                        summary: summary.clone(),
                        generated_messages: vec![QueryMessage::assistant(format!(
                            "tool-message: {summary}\n{formatted}"
                        ))],
                        context_label: Some(call.context_label.clone()),
                        progress_updates: vec![ToolProgressUpdate::new(
                            call.tool_use_id,
                            format!("lsp {action_str} complete"),
                        )],
                    },
                ),
                permission_denial: None,
            }
        }
        Err(e) => ToolExecutionTrace {
            progress_updates: vec![progress],
            result: ToolCallResult::failed(call, e),
            permission_denial: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp_client::LspRegistry;
    use std::sync::{Arc, Mutex};

    fn setup_registry() -> Arc<Mutex<LspRegistry>> {
        let reg = global_lsp_registry();
        let mut guard = reg.lock().unwrap();
        guard.register("rust-analyzer", vec!["rust".into()]);
        guard.connect("rust-analyzer").unwrap();
        drop(guard);
        reg
    }

    #[test]
    fn execute_lsp_tool_missing_action_fails() {
        let call = ToolCallInput::new("Lsp", "toolu-lsp-1")
            .with_argument("file_path", "main.rs")
            .with_context_label("test");
        let trace = execute_lsp(call);
        assert_eq!(trace.result.status_label(), "failed");
        assert!(trace.result.message().contains("missing required argument: action"));
    }

    #[test]
    fn execute_lsp_tool_returns_formatted_result() {
        let _reg = setup_registry();
        let call = ToolCallInput::new("Lsp", "toolu-lsp-2")
            .with_argument("action", "diagnostics")
            .with_argument("file_path", "main.rs")
            .with_argument("line", "10")
            .with_argument("column", "5")
            .with_context_label("test");
        let trace = execute_lsp(call);
        assert_eq!(trace.result.status_label(), "completed");
        assert!(trace.result.message().contains("lsp diagnostics on main.rs"));
    }
}
