use crate::global_registry::{GlobalToolRegistry, ToolSource};
use crate::message::QueryMessage;

use super::model::{
    ToolCallInput, ToolCallOutput, ToolCallResult, ToolExecutionTrace, ToolPermissionDecision,
    ToolProgressUpdate,
};

// ---------------------------------------------------------------------------
// DeferredTool + DeferredToolRegistry
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct DeferredTool {
    pub name: String,
    pub description: String,
    pub source: ToolSource,
    pub loaded: bool,
}

#[derive(Debug, Clone, Default)]
pub struct DeferredToolRegistry {
    deferred: Vec<DeferredTool>,
}

impl DeferredToolRegistry {
    pub fn new() -> Self {
        Self {
            deferred: Vec::new(),
        }
    }

    pub fn register_deferred(&mut self, name: &str, description: &str, source: ToolSource) {
        // Replace if already exists.
        if let Some(existing) = self.deferred.iter_mut().find(|t| t.name == name) {
            existing.description = description.to_string();
            existing.source = source;
            existing.loaded = false;
        } else {
            self.deferred.push(DeferredTool {
                name: name.to_string(),
                description: description.to_string(),
                source,
                loaded: false,
            });
        }
    }
    pub fn mark_loaded(&mut self, name: &str) {
        if let Some(tool) = self.deferred.iter_mut().find(|t| t.name == name) {
            tool.loaded = true;
        }
    }

    pub fn list_deferred(&self) -> Vec<&DeferredTool> {
        self.deferred.iter().collect()
    }

    pub fn list_unloaded(&self) -> Vec<&DeferredTool> {
        self.deferred.iter().filter(|t| !t.loaded).collect()
    }
}

// ---------------------------------------------------------------------------
// Search result + ranking
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ToolSearchResult {
    pub name: String,
    pub description: String,
    pub source: String,
    pub loaded: bool,
    pub relevance: u8, // higher = better match
}

fn source_label(source: ToolSource) -> &'static str {
    match source {
        ToolSource::Base => "base",
        ToolSource::Plugin => "plugin",
        ToolSource::Mcp => "mcp",
        ToolSource::Runtime => "runtime",
    }
}

/// Compute relevance score for a query against a tool name + description.
/// 3 = exact name match, 2 = name contains, 1 = description contains, 0 = no match.
fn relevance(query: &str, name: &str, description: &str) -> u8 {
    let q = query.to_lowercase();
    let n = name.to_lowercase();
    let d = description.to_lowercase();
    if n == q {
        3
    } else if n.contains(&q) {
        2
    } else if d.contains(&q) {
        1
    } else {
        0
    }
}

/// Search both the GlobalToolRegistry and a DeferredToolRegistry, merge and rank results.
pub fn search_tools(
    global: &GlobalToolRegistry,
    deferred: &DeferredToolRegistry,
    query: &str,
    max_results: usize,
) -> Vec<ToolSearchResult> {
    let mut results: Vec<ToolSearchResult> = Vec::new();

    // Search registered tools.
    for tool in global.list() {
        let score = relevance(query, &tool.name, &tool.description);
        if score > 0 {
            results.push(ToolSearchResult {
                name: tool.name.clone(),
                description: tool.description.clone(),
                source: source_label(tool.source).to_string(),
                loaded: true,
                relevance: score,
            });
        }
    }

    // Search deferred tools.
    for tool in &deferred.deferred {
        let score = relevance(query, &tool.name, &tool.description);
        if score > 0 {
            results.push(ToolSearchResult {
                name: tool.name.clone(),
                description: tool.description.clone(),
                source: format!("{} (deferred)", source_label(tool.source)),
                loaded: tool.loaded,
                relevance: score,
            });
        }
    }

    // Sort by relevance descending, then name ascending for stability.
    results.sort_by(|a, b| b.relevance.cmp(&a.relevance).then(a.name.cmp(&b.name)));
    results.truncate(max_results);
    results
}

// ---------------------------------------------------------------------------
// Tool executor entry point
// ---------------------------------------------------------------------------

/// Execute a ToolSearch tool call. Reads `query` (required) and `max_results` (optional, default 10).
///
/// This function creates its own empty registries for the search — in a real integration
/// the caller would pass populated registries. The executor wiring calls this function
/// directly so it matches the same signature pattern as other tool executors.
pub fn execute_tool_search(call: ToolCallInput) -> ToolExecutionTrace {
    let Some(query) = call.argument("query") else {
        return ToolExecutionTrace {
            progress_updates: Vec::new(),
            result: ToolCallResult::failed(call, "missing required argument: query"),
            permission_denial: None,
        };
    };
    let query = query.to_string();
    let max_results: usize = call
        .argument("max_results")
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);

    let progress = ToolProgressUpdate::new(
        call.tool_use_id.clone(),
        format!("searching tools: {query}"),
    );

    // Use the global singleton registry + an empty deferred registry.
    let global_arc = crate::global_registry::global_tool_registry();
    let global = global_arc.lock().unwrap_or_else(|e| e.into_inner());
    let deferred = DeferredToolRegistry::new();

    let results = search_tools(&global, &deferred, &query, max_results);
    let count = results.len();

    let formatted = if results.is_empty() {
        format!("no tools found matching '{query}'")
    } else {
        results
            .iter()
            .map(|r| {
                let status = if r.loaded { "loaded" } else { "deferred" };
                format!(
                    "- {} [{}] ({}): {}",
                    r.name, status, r.source, r.description
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    ToolExecutionTrace {
        progress_updates: vec![progress],
        result: ToolPermissionDecision::allow(false).settle(
            call.clone(),
            ToolCallOutput {
                summary: format!("found {count} tools matching '{query}'"),
                generated_messages: vec![QueryMessage::assistant(format!(
                    "tool-message: ToolSearch '{query}' -> {count} results\n{formatted}"
                ))],
                context_label: Some(call.context_label.clone()),
                progress_updates: vec![ToolProgressUpdate::new(
                    call.tool_use_id,
                    format!("search complete: {count} results"),
                )],
            },
        ),
        permission_denial: None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::global_registry::{GlobalToolRegistry, RegisteredTool, ToolSource};

    fn make_global() -> GlobalToolRegistry {
        let mut reg = GlobalToolRegistry::new();
        reg.register(RegisteredTool {
            name: "Read".to_string(),
            source: ToolSource::Base,
            description: "Read a file from disk".to_string(),
            schema: serde_json::json!({}),
        });
        reg.register(RegisteredTool {
            name: "Grep".to_string(),
            source: ToolSource::Base,
            description: "Search file contents".to_string(),
            schema: serde_json::json!({}),
        });
        reg.register(RegisteredTool {
            name: "WebFetch".to_string(),
            source: ToolSource::Base,
            description: "Fetch a URL".to_string(),
            schema: serde_json::json!({}),
        });
        reg
    }

    fn make_deferred() -> DeferredToolRegistry {
        let mut reg = DeferredToolRegistry::new();
        reg.register_deferred("ReadCode", "Intelligent code reader", ToolSource::Plugin);
        reg.register_deferred("NotebookEdit", "Edit Jupyter notebooks", ToolSource::Mcp);
        reg
    }

    #[test]
    fn search_finds_by_name() {
        let global = make_global();
        let deferred = DeferredToolRegistry::new();
        let results = search_tools(&global, &deferred, "Read", 10);
        assert!(!results.is_empty());
        assert_eq!(results[0].name, "Read");
        assert_eq!(results[0].relevance, 3); // exact match
    }

    #[test]
    fn search_finds_by_description() {
        let global = make_global();
        let deferred = DeferredToolRegistry::new();
        let results = search_tools(&global, &deferred, "URL", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "WebFetch");
        assert_eq!(results[0].relevance, 1); // description match
    }

    #[test]
    fn search_returns_empty_for_no_match() {
        let global = make_global();
        let deferred = DeferredToolRegistry::new();
        let results = search_tools(&global, &deferred, "zzz_nonexistent_zzz", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn deferred_registry_tracks_loaded_status() {
        let mut reg = DeferredToolRegistry::new();
        reg.register_deferred("MyTool", "does stuff", ToolSource::Plugin);
        assert_eq!(reg.list_unloaded().len(), 1);
        assert!(!reg.list_deferred()[0].loaded);

        reg.mark_loaded("MyTool");
        assert!(reg.list_deferred()[0].loaded);
        assert_eq!(reg.list_unloaded().len(), 0);
    }

    #[test]
    fn search_includes_deferred_tools() {
        let global = make_global();
        let deferred = make_deferred();
        // "Read" should match both global "Read" and deferred "ReadCode"
        let results = search_tools(&global, &deferred, "Read", 10);
        assert!(results.len() >= 2);
        let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"Read"));
        assert!(names.contains(&"ReadCode"));
    }

    #[test]
    fn max_results_limits_output() {
        let global = make_global();
        let deferred = make_deferred();
        // Search for something broad that matches multiple tools
        let results = search_tools(&global, &deferred, "e", 2);
        assert!(results.len() <= 2);
    }

    #[test]
    fn execute_tool_search_returns_formatted_results() {
        let call = ToolCallInput::new("ToolSearch", "toolu-ts-1")
            .with_argument("query", "Read")
            .with_argument("max_results", "5")
            .with_context_label("test");
        let trace = execute_tool_search(call);
        // Should succeed (even if global registry is empty in test context)
        assert_eq!(trace.result.status_label(), "completed");
        assert!(trace.result.message().contains("tools matching"));
    }
}
