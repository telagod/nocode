//! Discovery tools — ToolSearch, Lsp.

use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};

pub struct ToolSearchTool;

impl Tool for ToolSearchTool {
    fn name(&self) -> &str {
        "ToolSearch"
    }
    fn description(&self) -> &str {
        "Search for available tools by name or keyword."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "query":{"type":"string","description":"Search query to match tool names"},
            "max_results":{"type":"integer","description":"Max results to return"}
        },"required":["query"]})
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(query) = input["query"].as_str() else {
            return ToolOutput::error("Missing required parameter: query");
        };
        // Tool search is handled by the runtime — return available info
        ToolOutput::success(format!(
            "Tool search for '{query}' — use the tool registry to find matching tools"
        ))
    }
}

pub struct LspTool;

impl Tool for LspTool {
    fn name(&self) -> &str {
        "Lsp"
    }
    fn description(&self) -> &str {
        "Query the Language Server Protocol for code intelligence."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "action":{"type":"string","enum":["definition","references","hover","diagnostics"]},
            "file_path":{"type":"string"},
            "line":{"type":"integer"},
            "column":{"type":"integer"}
        },"required":["action","file_path"]})
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        let action = input["action"].as_str().unwrap_or("hover");
        let file_path = input["file_path"].as_str().unwrap_or("");
        // LSP requires a running language server — stub for now
        ToolOutput::error(format!(
            "LSP {action} for {file_path} — no language server connected. \
             Configure LSP servers in settings to enable code intelligence."
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_search_succeeds() {
        let tool = ToolSearchTool;
        let result = tool.execute(&json!({"query": "Bash"}));
        assert!(!result.is_error);
        assert!(result.content.contains("Bash"));
    }

    #[test]
    fn tool_search_missing_query() {
        let tool = ToolSearchTool;
        assert!(tool.execute(&json!({})).is_error);
    }

    #[test]
    fn lsp_returns_error_no_server() {
        let tool = LspTool;
        let result = tool.execute(&json!({"action": "hover", "file_path": "/tmp/test.rs"}));
        assert!(result.is_error);
        assert!(result.content.contains("no language server"));
    }

    #[test]
    fn lsp_missing_action() {
        let tool = LspTool;
        let result = tool.execute(&json!({"file_path": "/tmp/test.rs"}));
        assert!(result.is_error);
    }
}
