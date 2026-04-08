use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};
use std::process::Command;

pub struct GrepTool;

impl Tool for GrepTool {
    fn name(&self) -> &str {
        "Grep"
    }

    fn description(&self) -> &str {
        "Search file contents using regex patterns. Uses ripgrep if available, falls back to grep."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Regex pattern to search for" },
                "path": { "type": "string", "description": "File or directory to search in" },
                "glob": { "type": "string", "description": "Glob filter for files (e.g. *.rs)" },
                "output_mode": { "type": "string", "enum": ["content", "files_with_matches", "count"], "description": "Output mode (default: files_with_matches)" },
                "context": { "type": "integer", "description": "Lines of context before and after match (-C)" },
                "before_context": { "type": "integer", "description": "Lines before match (-B)" },
                "after_context": { "type": "integer", "description": "Lines after match (-A)" },
                "case_insensitive": { "type": "boolean", "description": "Case insensitive search (-i)" },
                "line_numbers": { "type": "boolean", "description": "Show line numbers (-n), default true" },
                "head_limit": { "type": "integer", "description": "Limit output to first N entries (default 250)" },
                "multiline": { "type": "boolean", "description": "Enable multiline matching (-U)" }
            },
            "required": ["pattern"]
        })
    }

    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(pattern) = input["pattern"].as_str() else {
            return ToolOutput::error("Missing required parameter: pattern");
        };

        let path = input["path"].as_str().unwrap_or(".");
        let glob_filter = input["glob"].as_str();
        let output_mode = input["output_mode"]
            .as_str()
            .unwrap_or("files_with_matches");
        let context = input["context"].as_u64();
        let before_context = input["before_context"].as_u64();
        let after_context = input["after_context"].as_u64();
        let case_insensitive = input["case_insensitive"].as_bool().unwrap_or(false);
        let line_numbers = input["line_numbers"].as_bool().unwrap_or(true);
        let head_limit = input["head_limit"].as_u64().unwrap_or(250) as usize;
        let multiline = input["multiline"].as_bool().unwrap_or(false);

        let use_rg = which_exists("rg");

        let mut cmd = if use_rg {
            let mut c = Command::new("rg");
            c.arg("--no-heading");

            // Output mode
            match output_mode {
                "files_with_matches" => {
                    c.arg("--files-with-matches");
                }
                "count" => {
                    c.arg("--count");
                }
                _ => {} // "content" — default rg behavior
            }

            // Context flags
            if let Some(ctx) = context {
                c.arg(format!("-C{ctx}"));
            }
            if let Some(b) = before_context {
                c.arg(format!("-B{b}"));
            }
            if let Some(a) = after_context {
                c.arg(format!("-A{a}"));
            }

            if case_insensitive {
                c.arg("-i");
            }
            if line_numbers && output_mode == "content" {
                c.arg("-n");
            }
            if multiline {
                c.arg("-U").arg("--multiline-dotall");
            }

            if let Some(g) = glob_filter {
                c.arg("--glob").arg(g);
            }
            c.arg(pattern).arg(path);
            c
        } else {
            let mut c = Command::new("grep");
            c.arg("-r");

            match output_mode {
                "files_with_matches" => {
                    c.arg("-l");
                }
                "count" => {
                    c.arg("-c");
                }
                _ => {}
            }

            if let Some(ctx) = context {
                c.arg(format!("-C{ctx}"));
            }
            if let Some(b) = before_context {
                c.arg(format!("-B{b}"));
            }
            if let Some(a) = after_context {
                c.arg(format!("-A{a}"));
            }

            if case_insensitive {
                c.arg("-i");
            }
            if line_numbers && output_mode == "content" {
                c.arg("-n");
            }

            if let Some(g) = glob_filter {
                c.arg("--include").arg(g);
            }
            c.arg(pattern).arg(path);
            c
        };

        match cmd.output() {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if stdout.is_empty() {
                    ToolOutput::success("No matches found")
                } else {
                    // Apply head_limit
                    let limited: String = if head_limit > 0 {
                        stdout
                            .lines()
                            .take(head_limit)
                            .collect::<Vec<_>>()
                            .join("\n")
                    } else {
                        stdout.to_string()
                    };
                    ToolOutput::success(limited)
                }
            }
            Err(e) => ToolOutput::error(format!("Search failed: {e}")),
        }
    }
}

fn which_exists(cmd: &str) -> bool {
    Command::new("which")
        .arg(cmd)
        .output()
        .is_ok_and(|o| o.status.success())
}
