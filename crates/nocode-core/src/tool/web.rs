//! Web tools — WebFetch, WebSearch.

use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};

pub struct WebFetchTool;

impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "WebFetch"
    }
    fn description(&self) -> &str {
        "Fetch content from a URL."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "url":{"type":"string","description":"The URL to fetch"},
            "max_length":{"type":"integer","description":"Max response length in chars"}
        },"required":["url"]})
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(url) = input["url"].as_str() else {
            return ToolOutput::error("Missing required parameter: url");
        };
        let max_len = input["max_length"].as_u64().unwrap_or(50_000) as usize;
        match reqwest::blocking::get(url) {
            Ok(resp) => match resp.text() {
                Ok(body) => {
                    let truncated = if body.len() > max_len {
                        format!("{}...(truncated)", &body[..max_len])
                    } else {
                        body
                    };
                    ToolOutput::success(truncated)
                }
                Err(e) => ToolOutput::error(format!("Failed to read response: {e}")),
            },
            Err(e) => ToolOutput::error(format!("Fetch failed: {e}")),
        }
    }
}

pub struct WebSearchTool;

impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "WebSearch"
    }
    fn description(&self) -> &str {
        "Search the web for information."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "query":{"type":"string","description":"Search query"},
            "max_results":{"type":"integer","description":"Max number of results"}
        },"required":["query"]})
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(query) = input["query"].as_str() else {
            return ToolOutput::error("Missing required parameter: query");
        };
        // Web search requires an external API — return guidance
        ToolOutput::error(format!(
            "Web search for '{query}' requires an external search API (not yet configured). \
             Use WebFetch with a known URL instead."
        ))
    }
}
