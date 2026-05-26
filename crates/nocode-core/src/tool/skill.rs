//! `Skill` tool — invoke a skill discovered by [`crate::skill::SkillRegistry`].
//!
//! This file is the thin tool-side wrapper. The actual skill discovery, parsing
//! and indexing lives in [`crate::skill`], which is also wired into prompt
//! assembly so the model can see the available skills before deciding to call
//! this tool.

use crate::skill::SkillRegistry;
use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};
use std::path::PathBuf;

/// Skill tool. By default it resolves the search roots from `std::env::current_dir()`
/// at call time. Use [`SkillTool::with_cwd`] to pin a fixed root (tests, embedded
/// usage, multi-workspace setups).
pub struct SkillTool {
    cwd: Option<String>,
}

impl SkillTool {
    /// Construct a tool that resolves cwd lazily on each call.
    pub const fn new() -> Self {
        Self { cwd: None }
    }

    /// Construct a tool pinned to a specific working directory.
    pub fn with_cwd(cwd: impl Into<String>) -> Self {
        Self {
            cwd: Some(cwd.into()),
        }
    }

    fn resolved_cwd(&self) -> String {
        self.cwd.clone().unwrap_or_else(|| {
            std::env::current_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| ".".to_owned())
        })
    }
}

impl Default for SkillTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for SkillTool {
    fn name(&self) -> &str {
        "Skill"
    }

    fn description(&self) -> &str {
        "Invoke a registered skill by name. Skills are discovered from .nocode/skills/ \
         and .claude/skills/ (project + user-global). Their index is part of the system \
         prompt; pick a name from there."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "skill": {
                    "type": "string",
                    "description": "Skill name (e.g. 'commit', 'review-pr', 'ns:name')."
                },
                "args": {
                    "type": "string",
                    "description": "Optional arguments substituted into $ARGUMENTS placeholders."
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

        let registry = SkillRegistry::load(&self.resolved_cwd());

        let Some(def) = registry.get(skill_name) else {
            let available: Vec<&str> = registry.iter().map(|(n, _)| n.as_str()).collect();
            let hint = if available.is_empty() {
                "No skills discovered. Place a markdown file under .nocode/skills/ \
                 or .claude/skills/ (project or ~/) with optional YAML frontmatter."
                    .to_owned()
            } else {
                format!("Available: {}", available.join(", "))
            };
            return ToolOutput::error(format!("Skill '{skill_name}' not found. {hint}"));
        };

        let prompt = def.render(args);
        ToolOutput::success(
            json!({
                "skill": def.name,
                "path": def.path.to_string_lossy(),
                "description": def.description,
                "prompt": prompt,
            })
            .to_string(),
        )
    }
}

/// Backwards-compatible helper that returns `(name, path)` pairs.
///
/// **Deprecated**: nothing in-tree calls this any more — the TUI and REPL
/// now hold a [`SkillRegistry`] directly and read `def.description`
/// alongside `def.path`. Kept for external callers (plugins, scripts) that
/// haven't migrated yet.
#[deprecated(
    note = "use SkillRegistry::load(cwd).iter() — gives names plus descriptions"
)]
pub fn list_skills() -> Vec<(String, PathBuf)> {
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| ".".to_owned());
    SkillRegistry::load(&cwd)
        .iter()
        .map(|(name, def)| (name.clone(), def.path.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::env_mutex;
    use crate::tool::Tool;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn missing_skill_param_errors() {
        let out = SkillTool::new().execute(&json!({}));
        assert!(out.is_error);
        assert!(out.content.contains("skill"));
    }

    #[test]
    fn unknown_skill_lists_available_or_hints_empty() {
        let _guard = env_mutex().lock().unwrap();
        let tmp = TempDir::new().unwrap();
        // Pin HOME so we don't pick up the developer's real ~/.claude/skills.
        let saved_home = std::env::var("HOME").ok();
        // SAFETY: env mutations serialized via env_mutex, restored below
        unsafe { std::env::set_var("HOME", tmp.path()) };

        let tool = SkillTool::with_cwd(tmp.path().to_string_lossy().into_owned());
        let out = tool.execute(&json!({ "skill": "__definitely_missing__" }));

        match saved_home {
            Some(h) => unsafe { std::env::set_var("HOME", h) },
            None => unsafe { std::env::remove_var("HOME") },
        }

        assert!(out.is_error);
        assert!(out.content.contains("not found"));
    }

    #[test]
    fn known_skill_returns_rendered_prompt() {
        let _guard = env_mutex().lock().unwrap();
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join(".nocode/skills");
        fs::create_dir_all(&skills_dir).unwrap();
        fs::write(
            skills_dir.join("greet.md"),
            "---\ndescription: greet caller\n---\nHello $ARGUMENTS\n",
        )
        .unwrap();

        let saved_home = std::env::var("HOME").ok();
        // SAFETY: env mutations serialized via env_mutex, restored below
        unsafe { std::env::set_var("HOME", tmp.path()) };

        let tool = SkillTool::with_cwd(tmp.path().to_string_lossy().into_owned());
        let out = tool.execute(&json!({ "skill": "greet", "args": "world" }));

        match saved_home {
            Some(h) => unsafe { std::env::set_var("HOME", h) },
            None => unsafe { std::env::remove_var("HOME") },
        }

        assert!(!out.is_error, "expected success, got: {}", out.content);
        assert!(out.content.contains("Hello world"));
        assert!(out.content.contains("greet"));
    }
}
