use std::fs;
use std::path::Path;

/// Discover and load CLAUDE.md files from the project hierarchy.
pub fn discover_claude_md(cwd: &str) -> Vec<String> {
    let mut contents = Vec::new();
    let home = std::env::var("HOME").unwrap_or_default();

    // User-level CLAUDE.md
    let user_path = Path::new(&home).join(".claude/CLAUDE.md");
    if let Ok(c) = fs::read_to_string(&user_path) {
        contents.push(c);
    }

    // Project-level CLAUDE.md
    let project_path = Path::new(cwd).join("CLAUDE.md");
    if let Ok(c) = fs::read_to_string(&project_path) {
        contents.push(c);
    }

    // .claude/CLAUDE.md in project
    let dot_claude_path = Path::new(cwd).join(".claude/CLAUDE.md");
    if let Ok(c) = fs::read_to_string(&dot_claude_path) {
        contents.push(c);
    }

    contents
}

/// Format discovered CLAUDE.md files into a system prompt section.
pub fn format_claude_md_prompt(contents: &[String]) -> Option<String> {
    if contents.is_empty() {
        return None;
    }
    let mut prompt = String::from(
        "The following are user instructions from CLAUDE.md files. \
         Follow them as part of the user's intent:\n\n",
    );
    for (i, content) in contents.iter().enumerate() {
        if i > 0 {
            prompt.push_str("\n---\n\n");
        }
        prompt.push_str(content);
    }
    Some(prompt)
}
