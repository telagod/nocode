use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};
use std::fs;

pub struct EditTool;

/// Generate a unified diff between old and new content.
fn unified_diff(path: &str, old: &str, new: &str) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    let mut diff = String::new();
    diff.push_str(&format!("--- a/{path}\n"));
    diff.push_str(&format!("+++ b/{path}\n"));

    // Simple line-by-line diff: find changed regions
    let max_len = old_lines.len().max(new_lines.len());
    let mut i = 0;
    while i < max_len {
        // Find start of a changed region
        if i < old_lines.len() && i < new_lines.len() && old_lines[i] == new_lines[i] {
            i += 1;
            continue;
        }

        // Found a difference — emit a hunk
        let ctx_start = i.saturating_sub(3);
        let mut old_end = i;
        let mut new_end = i;

        // Scan forward to find end of changed region
        while old_end < old_lines.len() || new_end < new_lines.len() {
            let old_line = old_lines.get(old_end);
            let new_line = new_lines.get(new_end);
            if old_line == new_line {
                // Check if we have 3+ matching lines (end of hunk)
                let mut match_count = 0;
                while old_end + match_count < old_lines.len()
                    && new_end + match_count < new_lines.len()
                    && old_lines[old_end + match_count] == new_lines[new_end + match_count]
                {
                    match_count += 1;
                    if match_count >= 3 {
                        break;
                    }
                }
                if match_count >= 3 {
                    break;
                }
                old_end += 1;
                new_end += 1;
            } else if old_end < old_lines.len()
                && (new_end >= new_lines.len()
                    || !new_lines[new_end..].contains(&old_lines[old_end]))
            {
                old_end += 1;
            } else {
                new_end += 1;
            }
        }

        let ctx_end_old = (old_end + 3).min(old_lines.len());
        let ctx_end_new = (new_end + 3).min(new_lines.len());

        diff.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            ctx_start + 1,
            ctx_end_old - ctx_start,
            ctx_start + 1,
            ctx_end_new - ctx_start,
        ));

        // Context before
        for line in &old_lines[ctx_start..i] {
            diff.push_str(&format!(" {line}\n"));
        }
        // Removed lines
        for line in &old_lines[i..old_end] {
            diff.push_str(&format!("-{line}\n"));
        }
        // Added lines
        for line in &new_lines[i..new_end] {
            diff.push_str(&format!("+{line}\n"));
        }
        // Context after
        let after_start = old_end;
        for line in &old_lines[after_start..ctx_end_old] {
            diff.push_str(&format!(" {line}\n"));
        }

        i = ctx_end_old;
    }

    diff
}

impl Tool for EditTool {
    fn name(&self) -> &str {
        "FileEdit"
    }

    fn description(&self) -> &str {
        "Replace an exact string in a file with new content. The old_string must be unique in the file."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": { "type": "string", "description": "The absolute path to the file to modify" },
                "old_string": { "type": "string", "description": "The text to replace" },
                "new_string": { "type": "string", "description": "The text to replace it with (must be different from old_string)" },
                "replace_all": { "type": "boolean", "description": "Replace all occurrences of old_string (default false)" }
            },
            "required": ["file_path", "old_string", "new_string"]
        })
    }

    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(path) = input["file_path"].as_str() else {
            return ToolOutput::error("Missing required parameter: file_path");
        };
        let Some(old_string) = input["old_string"].as_str() else {
            return ToolOutput::error("Missing required parameter: old_string");
        };
        let Some(new_string) = input["new_string"].as_str() else {
            return ToolOutput::error("Missing required parameter: new_string");
        };
        let replace_all = input["replace_all"].as_bool().unwrap_or(false);

        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => return ToolOutput::error(format!("Failed to read {path}: {e}")),
        };

        let count = content.matches(old_string).count();
        if count == 0 {
            return ToolOutput::error(format!(
                "old_string not found in {path}. Make sure it matches exactly."
            ));
        }

        let new_content = if replace_all {
            content.replace(old_string, new_string)
        } else {
            if count > 1 {
                return ToolOutput::error(format!(
                    "old_string found {count} times in {path}. It must be unique. Provide more context, or use replace_all."
                ));
            }
            content.replacen(old_string, new_string, 1)
        };

        match fs::write(path, &new_content) {
            Ok(()) => {
                let diff = unified_diff(path, &content, &new_content);
                let msg = if replace_all && count > 1 {
                    format!("Edited {path} ({count} replacements)\n\n{diff}")
                } else {
                    format!("Edited {path}\n\n{diff}")
                };
                ToolOutput::success(msg)
            }
            Err(e) => ToolOutput::error(format!("Failed to write {path}: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn edit_replaces_unique_string() {
        let path = "/tmp/nocode_edit_unit_test.txt";
        std::fs::write(path, "hello world\n").unwrap();
        let tool = EditTool;
        let result = tool.execute(&json!({
            "file_path": path, "old_string": "hello", "new_string": "goodbye"
        }));
        assert!(!result.is_error, "{}", result.content);
        assert!(std::fs::read_to_string(path).unwrap().contains("goodbye"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn edit_fails_on_nonunique() {
        let path = "/tmp/nocode_edit_nonunique.txt";
        std::fs::write(path, "aaa bbb aaa\n").unwrap();
        let tool = EditTool;
        let result = tool.execute(&json!({
            "file_path": path, "old_string": "aaa", "new_string": "ccc"
        }));
        assert!(result.is_error);
        assert!(result.content.contains("2 times"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn edit_replace_all() {
        let path = "/tmp/nocode_edit_replall.txt";
        std::fs::write(path, "aaa bbb aaa\n").unwrap();
        let tool = EditTool;
        let result = tool.execute(&json!({
            "file_path": path, "old_string": "aaa", "new_string": "ccc", "replace_all": true
        }));
        assert!(!result.is_error);
        assert_eq!(std::fs::read_to_string(path).unwrap(), "ccc bbb ccc\n");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn edit_not_found() {
        let path = "/tmp/nocode_edit_notfound.txt";
        std::fs::write(path, "hello\n").unwrap();
        let tool = EditTool;
        let result = tool.execute(&json!({
            "file_path": path, "old_string": "xyz", "new_string": "abc"
        }));
        assert!(result.is_error);
        assert!(result.content.contains("not found"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn edit_missing_params() {
        let tool = EditTool;
        assert!(tool.execute(&json!({})).is_error);
        assert!(tool.execute(&json!({"file_path": "/tmp/x"})).is_error);
    }

    #[test]
    fn edit_output_contains_diff() {
        let path = "/tmp/nocode_edit_diff_test.txt";
        std::fs::write(path, "line1\nline2\nline3\n").unwrap();
        let tool = EditTool;
        let result = tool.execute(&json!({
            "file_path": path, "old_string": "line2", "new_string": "changed"
        }));
        assert!(!result.is_error, "{}", result.content);
        assert!(result.content.contains("--- a/"));
        assert!(result.content.contains("+++ b/"));
        assert!(result.content.contains("-line2"));
        assert!(result.content.contains("+changed"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn unified_diff_basic() {
        let old = "aaa\nbbb\nccc\n";
        let new = "aaa\nxxx\nccc\n";
        let diff = unified_diff("test.txt", old, new);
        assert!(diff.contains("--- a/test.txt"));
        assert!(diff.contains("+++ b/test.txt"));
        assert!(diff.contains("-bbb"));
        assert!(diff.contains("+xxx"));
    }

    #[test]
    fn unified_diff_no_change() {
        let content = "aaa\nbbb\n";
        let diff = unified_diff("test.txt", content, content);
        // No hunks when content is identical
        assert!(!diff.contains("@@"));
    }
}
