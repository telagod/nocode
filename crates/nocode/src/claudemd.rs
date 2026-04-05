use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ClaudeMdFile {
    pub path: PathBuf,
    pub kind: ClaudeMdKind,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaudeMdKind {
    User,
    Project,
    Rules,
    Local,
}

impl ClaudeMdKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Project => "project",
            Self::Rules => "rules",
            Self::Local => "local",
        }
    }
}

pub fn discover_claude_md_files(cwd: &Path) -> Vec<ClaudeMdFile> {
    let mut files = Vec::new();
    if let Some(home) = home_dir() {
        load(
            &home.join(".claude/CLAUDE.md"),
            ClaudeMdKind::User,
            &mut files,
        );
    }
    let mut ancestors: Vec<PathBuf> = Vec::new();
    let mut cur = cwd.to_path_buf();
    loop {
        ancestors.push(cur.clone());
        if !cur.pop() {
            break;
        }
    }
    ancestors.reverse();
    for dir in &ancestors {
        load(&dir.join("CLAUDE.md"), ClaudeMdKind::Project, &mut files);
        load(
            &dir.join(".claude/CLAUDE.md"),
            ClaudeMdKind::Project,
            &mut files,
        );
        load_rules(&dir.join(".claude/rules"), &mut files);
        load(
            &dir.join("CLAUDE.local.md"),
            ClaudeMdKind::Local,
            &mut files,
        );
    }
    files
}

pub fn format_claude_md_for_prompt(files: &[ClaudeMdFile]) -> Option<String> {
    if files.is_empty() {
        return None;
    }
    let mut out = String::from(
        "Codebase and user instructions are shown below. Be sure to adhere to these instructions. \
         IMPORTANT: These instructions OVERRIDE any default behavior and \
         you MUST follow them exactly as written.",
    );
    for f in files {
        out.push_str(&format!(
            "\n\nContents of {} ({} instructions):\n\n",
            f.path.display(),
            f.kind.label()
        ));
        out.push_str(&strip_html_comments(&f.content));
    }
    Some(out)
}

fn load(path: &Path, kind: ClaudeMdKind, files: &mut Vec<ClaudeMdFile>) {
    if path.is_file()
        && let Ok(content) = fs::read_to_string(path)
        && !content.trim().is_empty()
    {
        files.push(ClaudeMdFile {
            path: path.to_path_buf(),
            kind,
            content,
        });
    }
}

fn load_rules(dir: &Path, files: &mut Vec<ClaudeMdFile>) {
    if !dir.is_dir() {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "md"))
        .collect();
    paths.sort();
    for p in paths {
        load(&p, ClaudeMdKind::Rules, files);
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn strip_html_comments(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(start) = rest.find("<!--") {
        result.push_str(&rest[..start]);
        if let Some(end) = rest[start..].find("-->") {
            rest = &rest[start + end + 3..];
        } else {
            return result;
        }
    }
    result.push_str(rest);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_html_comments_works() {
        assert_eq!(strip_html_comments("a <!-- b --> c"), "a  c");
        assert_eq!(strip_html_comments("no comments"), "no comments");
    }

    #[test]
    fn format_empty_returns_none() {
        assert!(format_claude_md_for_prompt(&[]).is_none());
    }

    #[test]
    fn format_includes_content() {
        let files = vec![ClaudeMdFile {
            path: PathBuf::from("/test/CLAUDE.md"),
            kind: ClaudeMdKind::Project,
            content: String::from("Be concise."),
        }];
        let result = format_claude_md_for_prompt(&files).unwrap();
        assert!(result.contains("IMPORTANT"));
        assert!(result.contains("Be concise."));
    }
}
