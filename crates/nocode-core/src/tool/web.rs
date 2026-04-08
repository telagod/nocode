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
        let prompt = input["prompt"].as_str().unwrap_or("");
        let max_len: usize = 50_000;
        match reqwest::blocking::get(url) {
            Ok(resp) => match resp.text() {
                Ok(body) => {
                    // Strip HTML tags for cleaner content
                    let cleaned = strip_html_tags_simple(&body);
                    let truncated = if cleaned.len() > max_len {
                        format!("{}...(truncated)", &cleaned[..max_len])
                    } else {
                        cleaned
                    };
                    // Include prompt context in output for model to process
                    if prompt.is_empty() {
                        ToolOutput::success(truncated)
                    } else {
                        ToolOutput::success(format!(
                            "Content from {url} (prompt: {prompt}):\n\n{truncated}"
                        ))
                    }
                }
                Err(e) => ToolOutput::error(format!("Failed to read response: {e}")),
            },
            Err(e) => ToolOutput::error(format!("Fetch failed: {e}")),
        }
    }
}

/// Simple HTML tag stripping for web content.
fn strip_html_tags_simple(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut in_script = false;
    let mut in_style = false;

    let lower = html.to_lowercase();
    let bytes = html.as_bytes();
    let lower_bytes = lower.as_bytes();

    let mut i = 0;
    while i < bytes.len() {
        if !in_tag && bytes[i] == b'<' {
            // Check for script/style start
            if i + 7 < lower_bytes.len() && &lower_bytes[i..i + 7] == b"<script" {
                in_script = true;
            }
            if i + 6 < lower_bytes.len() && &lower_bytes[i..i + 6] == b"<style" {
                in_style = true;
            }
            in_tag = true;
        } else if in_tag && bytes[i] == b'>' {
            // Check for script/style end
            if i >= 8 && &lower_bytes[i - 8..=i] == b"</script>" {
                in_script = false;
            }
            if i >= 7 && &lower_bytes[i - 7..=i] == b"</style>" {
                in_style = false;
            }
            in_tag = false;
        } else if !in_tag && !in_script && !in_style {
            result.push(bytes[i] as char);
        }
        i += 1;
    }

    // Collapse whitespace
    let mut collapsed = String::with_capacity(result.len());
    let mut prev_ws = false;
    for ch in result.chars() {
        if ch.is_whitespace() {
            if !prev_ws {
                collapsed.push(' ');
            }
            prev_ws = true;
        } else {
            collapsed.push(ch);
            prev_ws = false;
        }
    }
    collapsed.trim().to_string()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_html_tags_basic() {
        assert_eq!(strip_html_tags("<b>hello</b>"), "hello");
        assert_eq!(strip_html_tags("<a href=\"x\">link</a>"), "link");
        assert_eq!(strip_html_tags("no tags"), "no tags");
    }

    #[test]
    fn strip_html_tags_entities() {
        assert_eq!(strip_html_tags("a &amp; b"), "a & b");
        assert_eq!(strip_html_tags("&lt;code&gt;"), "<code>");
    }

    #[test]
    fn strip_html_tags_simple_removes_script_style() {
        let html = "<html><script>alert(1)</script><style>.x{}</style><p>text</p></html>";
        let result = strip_html_tags_simple(html);
        assert!(!result.contains("alert"));
        assert!(!result.contains(".x{}"));
        assert!(result.contains("text"));
    }

    #[test]
    fn strip_html_tags_simple_collapses_whitespace() {
        let html = "<p>hello</p>   \n\n   <p>world</p>";
        let result = strip_html_tags_simple(html);
        assert!(!result.contains("  "));
    }

    #[test]
    fn extract_href_direct() {
        let frag = r#"<a href="https://example.com" class="result__a">"#;
        assert_eq!(extract_href(frag), Some("https://example.com".to_string()));
    }

    #[test]
    fn extract_href_ddg_redirect() {
        let frag = r#"<a href="/l/?uddg=https%3A%2F%2Fexample.com&rut=abc" class="result__a">"#;
        assert_eq!(extract_href(frag), Some("https://example.com".to_string()));
    }

    #[test]
    fn parse_ddg_results_empty() {
        let results = parse_ddg_results("<html></html>", 10);
        assert!(results.is_empty());
    }
}
