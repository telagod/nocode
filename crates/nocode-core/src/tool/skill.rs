//! Skill tool — invoke user-defined skills from .claude/skills/ or .nocode/skills/.

use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;

pub struct SkillTool;

impl Tool for SkillTool {
    fn name(&self) -> &str {
        "Skill"
    }
    fn description(&self) -> &str {
        "Execute a user-invocable skill by name. Skills are defined in .claude/skills/ or .nocode/skills/ directories."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "skill": {
                    "type": "string",
                    "description": "The skill name to invoke (e.g., 'commit', 'review-pr')"
                },
                "args": {
                    "type": "string",
                    "description": "Optional arguments for the skill"
                }
            },
            "required": ["skill"]
        })
    }
    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(skill_name) = input["skill"].as_str() else {
            return ToolOutput::error("Missing required parameter: skill");
        };
        let args = input["args"].as_str().unwrap_or("");

        // Discover skill file
        let skill_path = match discover_skill(skill_name) {
            Some(p) => p,
            None => {
                return ToolOutput::error(format!(
                    "Skill '{skill_name}' not found. Searched .claude/skills/ and .nocode/skills/"
                ));
            }
        };

        // Read skill content
        let content = match fs::read_to_string(&skill_path) {
            Ok(c) => c,
            Err(e) => {
                return ToolOutput::error(format!(
                    "Failed to read skill '{}': {e}",
                    skill_path.display()
                ));
            }
        };

        // Parse skill — extract prompt from markdown content
        let prompt = parse_skill_prompt(&content, args);

        ToolOutput::success(
            json!({
                "skill": skill_name,
                "path": skill_path.to_string_lossy(),
                "prompt": prompt,
            })
            .to_string(),
        )
    }
}

/// Discover a skill file by name.
///
/// Search order:
/// 1. `{cwd}/.claude/skills/{name}.md`
/// 2. `{cwd}/.nocode/skills/{name}.md`
/// 3. `~/.claude/skills/{name}.md`
/// 4. `~/.nocode/skills/{name}.md`
///
/// Also supports fully qualified names like `namespace:skill` → `{name}.md`
fn discover_skill(name: &str) -> Option<PathBuf> {
    let file_name = if name.contains(':') {
        // "namespace:skill" → look for skill.md in namespace/ subdirectory
        let parts: Vec<&str> = name.splitn(2, ':').collect();
        format!("{}/{}.md", parts[0], parts[1])
    } else {
        format!("{name}.md")
    };

    let cwd = std::env::current_dir()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|_| PathBuf::from("."));
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));

    let candidates = [
        cwd.join(".claude/skills").join(&file_name),
        cwd.join(".nocode/skills").join(&file_name),
        home.join(".claude/skills").join(&file_name),
        home.join(".nocode/skills").join(&file_name),
    ];

    candidates.into_iter().find(|p| p.exists())
}

/// Parse a skill markdown file and extract the prompt.
/// Supports YAML frontmatter (delimited by ---) which is stripped.
/// Replaces `$ARGUMENTS` placeholder with the provided args.
fn parse_skill_prompt(content: &str, args: &str) -> String {
    let body = strip_frontmatter(content);
    body.replace("$ARGUMENTS", args)
        .replace("${ARGUMENTS}", args)
}

/// Strip YAML frontmatter (--- delimited) from markdown content.
fn strip_frontmatter(content: &str) -> &str {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return content;
    }
    // Find the closing ---
    if let Some(end) = trimmed[3..].find("\n---") {
        let after = end + 3 + 4; // skip past "\n---"
        trimmed[after..].trim_start_matches('\n')
    } else {
        content
    }
}

/// List all available skills from search directories.
pub fn list_skills() -> Vec<(String, PathBuf)> {
    let cwd = std::env::current_dir()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|_| PathBuf::from("."));
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));

    let dirs = [
        cwd.join(".claude/skills"),
        cwd.join(".nocode/skills"),
        home.join(".claude/skills"),
        home.join(".nocode/skills"),
    ];

    let mut skills = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for dir in &dirs {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "md")
                    && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                    && seen.insert(stem.to_string())
                {
                    skills.push((stem.to_string(), path));
                }
            }
        }
    }

    skills
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_frontmatter_no_frontmatter() {
        let content = "# Hello\n\nWorld";
        assert_eq!(strip_frontmatter(content), content);
    }

    #[test]
    fn strip_frontmatter_with_frontmatter() {
        let content = "---\nname: test\ntype: skill\n---\n# Hello\n\nWorld";
        assert_eq!(strip_frontmatter(content), "# Hello\n\nWorld");
    }

    #[test]
    fn parse_skill_replaces_arguments() {
        let content = "Run this: $ARGUMENTS\nAlso: ${ARGUMENTS}";
        let result = parse_skill_prompt(content, "my-arg");
        assert_eq!(result, "Run this: my-arg\nAlso: my-arg");
    }

    #[test]
    fn discover_skill_nonexistent() {
        assert!(discover_skill("__nonexistent_skill_xyz__").is_none());
    }
}
