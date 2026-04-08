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
            "url":{"type":"string","description":"The URL to fetch content from"},
            "prompt":{"type":"string","description":"The prompt to run on the fetched content"}
        },"required":["url","prompt"]})
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(url) = input["url"].as_str() else {
            return ToolOutput::error("Missing required parameter: url");
        };
        let _prompt = input["prompt"].as_str().unwrap_or("");
        let max_len: usize = 50_000;
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
        "Search the web for information using DuckDuckGo."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "query":{"type":"string","description":"The search query to use"},
            "allowed_domains":{"type":"array","items":{"type":"string"},"description":"Only include search results from these domains"},
            "blocked_domains":{"type":"array","items":{"type":"string"},"description":"Never include search results from these domains"}
        },"required":["query"]})
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(query) = input["query"].as_str() else {
            return ToolOutput::error("Missing required parameter: query");
        };
        let max_results = 10usize; // fetch more, filter down

        let allowed_domains: Vec<String> = input["allowed_domains"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let blocked_domains: Vec<String> = input["blocked_domains"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        // Use DuckDuckGo HTML lite — no API key required
        let encoded = urlencoding::encode(query);
        let url = format!("https://html.duckduckgo.com/html/?q={encoded}");

        let client = reqwest::blocking::Client::builder()
            .user_agent("Mozilla/5.0 (compatible; nocode/1.0)")
            .timeout(std::time::Duration::from_secs(15))
            .build();

        let client = match client {
            Ok(c) => c,
            Err(e) => return ToolOutput::error(format!("Failed to create HTTP client: {e}")),
        };

        let resp = match client.get(&url).send() {
            Ok(r) => r,
            Err(e) => return ToolOutput::error(format!("Search request failed: {e}")),
        };

        let body = match resp.text() {
            Ok(b) => b,
            Err(e) => return ToolOutput::error(format!("Failed to read search response: {e}")),
        };

        // Parse results from DuckDuckGo HTML lite
        let results = parse_ddg_results(&body, max_results);

        // Apply domain filtering
        let filtered: Vec<&SearchResult> = results
            .iter()
            .filter(|r| {
                if !allowed_domains.is_empty()
                    && !allowed_domains.iter().any(|d| r.url.contains(d.as_str()))
                {
                    return false;
                }
                if blocked_domains.iter().any(|d| r.url.contains(d.as_str())) {
                    return false;
                }
                true
            })
            .take(5)
            .collect();

        if filtered.is_empty() {
            return ToolOutput::success(format!("No results found for: {query}"));
        }

        let formatted: Vec<String> = filtered
            .iter()
            .enumerate()
            .map(|(i, r)| format!("{}. {}\n   {}\n   {}", i + 1, r.title, r.url, r.snippet))
            .collect();

        ToolOutput::success(formatted.join("\n\n"))
    }
}

struct SearchResult {
    title: String,
    url: String,
    snippet: String,
}

/// Parse search results from DuckDuckGo HTML lite response.
fn parse_ddg_results(html: &str, max: usize) -> Vec<SearchResult> {
    let mut results = Vec::new();

    // DuckDuckGo HTML lite uses <a class="result__a" href="...">title</a>
    // and <a class="result__snippet" ...>snippet</a>
    let mut pos = 0;
    while results.len() < max {
        // Find result link
        let link_marker = "class=\"result__a\"";
        let Some(link_start) = html[pos..].find(link_marker) else {
            break;
        };
        let link_start = pos + link_start;

        // Extract href
        let href_search_start = link_start.saturating_sub(100);
        let href = extract_href(&html[href_search_start..link_start + link_marker.len() + 50]);

        // Extract title (text between > and </a>)
        let title_start = html[link_start..].find('>').map(|i| link_start + i + 1);
        let title_end = title_start.and_then(|s| html[s..].find("</a>").map(|i| s + i));
        let title = match (title_start, title_end) {
            (Some(s), Some(e)) => strip_html_tags(&html[s..e]).trim().to_string(),
            _ => String::new(),
        };

        // Find snippet
        let snippet_marker = "class=\"result__snippet\"";
        let snippet = if let Some(snip_start) = html[link_start..].find(snippet_marker) {
            let snip_abs = link_start + snip_start;
            let text_start = html[snip_abs..].find('>').map(|i| snip_abs + i + 1);
            let text_end = text_start.and_then(|s| html[s..].find("</").map(|i| s + i));
            match (text_start, text_end) {
                (Some(s), Some(e)) => strip_html_tags(&html[s..e]).trim().to_string(),
                _ => String::new(),
            }
        } else {
            String::new()
        };

        pos = title_end.unwrap_or(link_start + link_marker.len() + 1);

        if !title.is_empty() {
            results.push(SearchResult {
                title,
                url: href.unwrap_or_default(),
                snippet,
            });
        }
    }

    results
}

fn extract_href(fragment: &str) -> Option<String> {
    let href_pos = fragment.find("href=\"")?;
    let start = href_pos + 6;
    let end = fragment[start..].find('"').map(|i| start + i)?;
    let raw = &fragment[start..end];
    // DuckDuckGo wraps URLs in redirect — extract actual URL
    if let Some(u_pos) = raw.find("uddg=") {
        let u_start = u_pos + 5;
        let u_end = raw[u_start..].find('&').map_or(raw.len(), |i| u_start + i);
        Some(
            urlencoding::decode(&raw[u_start..u_end])
                .unwrap_or_default()
                .to_string(),
        )
    } else {
        Some(raw.to_string())
    }
}

fn strip_html_tags(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }
    // Decode common HTML entities
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ")
}
