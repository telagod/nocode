//! Discovery tools — ToolSearch, Lsp.

use crate::tool::global_registry::global_tool_registry;
use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// ToolSearch
// ---------------------------------------------------------------------------

pub struct ToolSearchTool;

impl Tool for ToolSearchTool {
    fn name(&self) -> &str {
        "ToolSearch"
    }
    fn description(&self) -> &str {
        "Fetches full schema definitions for deferred tools so they can be called. \
         Deferred tools appear by name in the available tool list but their parameter \
         schemas are not loaded until this tool is used to discover them. \
         Use \"select:ToolName\" to fetch a specific tool, or a keyword to search."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Query to find tools. Use \"select:ToolName\" to fetch a specific tool's schema, or a keyword to search by name/description."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default: 5)"
                }
            },
            "required": ["query"]
        })
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(query) = input["query"].as_str() else {
            return ToolOutput::error("Missing required parameter: query");
        };
        let max_results = input["max_results"].as_u64().unwrap_or(5).min(20) as usize;

        // Handle "select:ToolName" exact lookup
        if let Some(tool_name) = query.strip_prefix("select:") {
            return self.lookup_tool(tool_name.trim());
        }

        // Keyword search across all registered tools
        self.search_tools(query, max_results)
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ToolSearchTool {
    /// Look up a single tool by exact name and return its full definition.
    fn lookup_tool(&self, name: &str) -> ToolOutput {
        let global = global_tool_registry();
        let guard = global.lock().unwrap_or_else(|e| e.into_inner());

        // Search base tools
        if let Some(tool) = guard.get_native(name) {
            let def = tool.definition();
            return ToolOutput::success(
                json!({
                    "tools": [{
                        "name": def.name,
                        "description": def.description,
                        "input_schema": def.input_schema,
                    }]
                })
                .to_string(),
            );
        }

        // Search bridged tools (MCP/plugin) by iterating definitions
        let all_defs = guard.definitions();
        if let Some(def) = all_defs.iter().find(|d| d.name == name) {
            return ToolOutput::success(
                json!({
                    "tools": [{
                        "name": def.name,
                        "description": def.description,
                        "input_schema": def.input_schema,
                    }]
                })
                .to_string(),
            );
        }

        // Global registry not initialized with base tools in test context.
        // Fall back to creating a default registry and searching it directly.
        let base = crate::tool::ToolRegistry::with_defaults("/tmp");
        if let Some(tool) = base.get(name) {
            let def = tool.definition();
            return ToolOutput::success(
                json!({
                    "tools": [{
                        "name": def.name,
                        "description": def.description,
                        "input_schema": def.input_schema,
                    }]
                })
                .to_string(),
            );
        }

        ToolOutput::error(format!("Tool '{name}' not found in registry"))
    }

    /// Search tools by keyword matching against name and description.
    fn search_tools(&self, query: &str, max_results: usize) -> ToolOutput {
        let global = global_tool_registry();
        let guard = global.lock().unwrap_or_else(|e| e.into_inner());
        let all_defs = guard.definitions();

        // If global registry is empty (test context), fall back to default registry
        let defs = if all_defs.is_empty() {
            let base = crate::tool::ToolRegistry::with_defaults("/tmp");
            base.definitions()
        } else {
            all_defs
        };

        let query_lower = query.to_lowercase();

        // Score each tool: exact name match > name contains > description contains
        let mut scored: Vec<(usize, &crate::provider::types::ToolDefinition)> = defs
            .iter()
            .filter_map(|def| {
                let name_lower = def.name.to_lowercase();
                let desc_lower = def.description.to_lowercase();

                let score = if name_lower == query_lower {
                    100
                } else if name_lower.contains(&query_lower) {
                    50
                } else if desc_lower.contains(&query_lower) {
                    20
                } else {
                    // Also check if query words appear in name
                    let words: Vec<&str> = query_lower.split_whitespace().collect();
                    let name_matches = words.iter().filter(|w| name_lower.contains(*w)).count();
                    let desc_matches = words.iter().filter(|w| desc_lower.contains(*w)).count();
                    let total = name_matches * 10 + desc_matches * 5;
                    if total > 0 { total } else { return None }
                };
                Some((score, def))
            })
            .collect();

        scored.sort_by_key(|s| std::cmp::Reverse(s.0));

        let results: Vec<Value> = scored
            .into_iter()
            .take(max_results)
            .map(|(_, def)| {
                json!({
                    "name": def.name,
                    "description": def.description,
                    "input_schema": def.input_schema,
                })
            })
            .collect();

        if results.is_empty() {
            return ToolOutput::success(
                json!({
                    "tools": [],
                    "message": format!("No tools matching '{query}' found")
                })
                .to_string(),
            );
        }

        ToolOutput::success(
            json!({
                "tools": results,
                "total": results.len()
            })
            .to_string(),
        )
    }
}

// ---------------------------------------------------------------------------
// Lsp
// ---------------------------------------------------------------------------

pub struct LspTool;

impl Tool for LspTool {
    fn name(&self) -> &str {
        "Lsp"
    }
    fn description(&self) -> &str {
        "Query the Language Server Protocol for code intelligence."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["definition", "references", "hover", "diagnostics"]
                },
                "file_path": { "type": "string" },
                "line": { "type": "integer" },
                "column": { "type": "integer" }
            },
            "required": ["action", "file_path"]
        })
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
    fn tool_search_keyword_match() {
        let tool = ToolSearchTool;
        let result = tool.execute(&json!({"query": "Bash"}));
        assert!(!result.is_error);
        // Should find at least the Bash tool
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        let tools = parsed["tools"].as_array().unwrap();
        assert!(!tools.is_empty());
        assert!(tools.iter().any(|t| t["name"].as_str() == Some("Bash")));
    }

    #[test]
    fn tool_search_select_exact() {
        let tool = ToolSearchTool;
        let result = tool.execute(&json!({"query": "select:Bash"}));
        assert!(!result.is_error);
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        let tools = parsed["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"].as_str(), Some("Bash"));
        // Should include input_schema
        assert!(tools[0]["input_schema"].is_object());
    }

    #[test]
    fn tool_search_select_not_found() {
        let tool = ToolSearchTool;
        let result = tool.execute(&json!({"query": "select:NonExistentTool123"}));
        assert!(result.is_error);
    }

    #[test]
    fn tool_search_no_match() {
        let tool = ToolSearchTool;
        let result = tool.execute(&json!({"query": "zzzznoexistxyz"}));
        assert!(!result.is_error);
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        let tools = parsed["tools"].as_array().unwrap();
        assert!(tools.is_empty());
    }

    #[test]
    fn tool_search_missing_query() {
        let tool = ToolSearchTool;
        assert!(tool.execute(&json!({})).is_error);
    }

    #[test]
    fn tool_search_max_results() {
        let tool = ToolSearchTool;
        let result = tool.execute(&json!({"query": "a", "max_results": 2}));
        assert!(!result.is_error);
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        let tools = parsed["tools"].as_array().unwrap();
        assert!(tools.len() <= 2);
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
