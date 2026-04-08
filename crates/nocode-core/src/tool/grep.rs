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
                "path": { "type": "string", "description": "File or directory to search in (rg PATH). Defaults to cwd." },
                "glob": { "type": "string", "description": "Glob pattern to filter files (e.g. \"*.js\", \"*.{ts,tsx}\") — maps to rg --glob" },
                "output_mode": { "type": "string", "enum": ["content", "files_with_matches", "count"], "description": "Output mode. Defaults to files_with_matches." },
                "-B": { "type": "integer", "description": "Lines before match (rg -B). Requires output_mode: content." },
                "-A": { "type": "integer", "description": "Lines after match (rg -A). Requires output_mode: content." },
                "-C": { "type": "integer", "description": "Alias for context." },
                "context": { "type": "integer", "description": "Lines before and after match (rg -C). Requires output_mode: content." },
                "-n": { "type": "boolean", "description": "Show line numbers (rg -n). Requires output_mode: content. Defaults to true." },
                "-i": { "type": "boolean", "description": "Case insensitive search (rg -i)" },
                "type": { "type": "string", "description": "File type to search (rg --type). Common types: js, py, rust, go, java." },
                "head_limit": { "type": "integer", "description": "Limit output to first N entries (default 250). Pass 0 for unlimited." },
                "offset": { "type": "integer", "description": "Skip first N entries before applying head_limit. Defaults to 0." },
                "multiline": { "type": "boolean", "description": "Enable multiline mode (rg -U --multiline-dotall). Default: false." }
            },
            "required": ["pattern"]
        })
    }

    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(pattern) = input["pattern"].as_str() else {
            return ToolOutput::error("Missing required parameter: pattern");
        };

        // PLACEHOLDER_GREP_EXECUTE
        let path = input["path"].as_str().unwrap_or(".");
        let glob_filter = input["glob"].as_str();
        let output_mode = input["output_mode"]
            .as_str()
            .unwrap_or("files_with_matches");
        // Support both Claude Code param names (-B/-A/-C/-n/-i) and long names
        let context = input["context"].as_u64().or_else(|| input["-C"].as_u64());
        let before_context = input["-B"].as_u64();
        let after_context = input["-A"].as_u64();
        let case_insensitive = input["-i"].as_bool().unwrap_or(false);
        let line_numbers = input["-n"].as_bool().unwrap_or(true);
        let file_type = input["type"].as_str();
        let head_limit = input["head_limit"].as_u64().unwrap_or(250) as usize;
        let skip_offset = input["offset"].as_u64().unwrap_or(0) as usize;
        let multiline = input["multiline"].as_bool().unwrap_or(false);

        let use_rg = which_exists("rg");

        let mut cmd = if use_rg {
            let mut c = Command::new("rg");
            c.arg("--no-heading");

            match output_mode {
                "files_with_matches" => {
                    c.arg("--files-with-matches");
                }
                "count" => {
                    c.arg("--count");
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
            if multiline {
                c.arg("-U").arg("--multiline-dotall");
            }
            if let Some(ft) = file_type {
                c.arg("--type").arg(ft);
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
                    // Apply offset then head_limit
                    let lines: Vec<&str> = stdout.lines().collect();
                    let after_skip: Vec<&str> = if skip_offset > 0 {
                        lines.into_iter().skip(skip_offset).collect()
                    } else {
                        lines
                    };
                    let limited: String = if head_limit > 0 {
                        after_skip
                            .into_iter()
                            .take(head_limit)
                            .collect::<Vec<_>>()
                            .join("\n")
                    } else {
                        after_skip.join("\n")
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
