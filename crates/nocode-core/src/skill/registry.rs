//! `SkillRegistry` — discovers skill `.md` files under `.claude/skills/` and
//! `.nocode/skills/` (project + user-global), parses optional YAML frontmatter,
//! and exposes an index suitable for inclusion in the system prompt.
//!
//! ## Search precedence (first wins on name collision)
//!
//! 1. `{cwd}/.nocode/skills/{name}.md`
//! 2. `{cwd}/.claude/skills/{name}.md`
//! 3. `~/.nocode/skills/{name}.md`
//! 4. `~/.claude/skills/{name}.md`
//!
//! Plus namespaced form `ns:name` → `{ns}/{name}.md` in each root.
//!
//! ## Frontmatter (all optional)
//!
//! ```yaml
//! ---
//! name: commit-and-push
//! description: Stage, commit with conventional message, push to origin.
//! triggers: ["commit", "git push"]
//! ---
//! ```
//!
//! Only `description` is required to be useful — the model picks skills by
//! reading the index. If `description` is missing, the first non-empty line of
//! the body is used as a fallback (truncated to 200 chars).

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Optional YAML frontmatter parsed from a skill file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillFrontmatter {
    pub name: Option<String>,
    pub description: Option<String>,
    pub triggers: Vec<String>,
}

/// A discovered skill — the name is the lookup key, `body` is the prompt
/// material handed to the model when the skill is invoked.
#[derive(Debug, Clone)]
pub struct SkillDef {
    /// Canonical name (frontmatter `name`, else file stem).
    pub name: String,
    /// One-line description for the prompt index.
    pub description: String,
    /// Full parsed frontmatter.
    pub frontmatter: SkillFrontmatter,
    /// Skill body (markdown after frontmatter), used as prompt material.
    pub body: String,
    /// Source path — for debugging and TUI display.
    pub path: PathBuf,
}

impl SkillDef {
    /// Substitute `$ARGUMENTS` / `${ARGUMENTS}` in the body.
    pub fn render(&self, args: &str) -> String {
        self.body
            .replace("${ARGUMENTS}", args)
            .replace("$ARGUMENTS", args)
    }
}

/// First-class skill registry. Loaded once, then queried throughout the session.
#[derive(Debug, Clone, Default)]
pub struct SkillRegistry {
    skills: BTreeMap<String, SkillDef>,
}

impl SkillRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Discover skills from the conventional search roots given the working
    /// directory. Use [`SkillRegistry::load_from_dirs`] for custom roots.
    pub fn load(cwd: &str) -> Self {
        let cwd_path = PathBuf::from(cwd);
        let home = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."));

        let roots = [
            cwd_path.join(".nocode/skills"),
            cwd_path.join(".claude/skills"),
            home.join(".nocode/skills"),
            home.join(".claude/skills"),
        ];
        Self::load_from_dirs(&roots)
    }

    /// Load skills from an explicit list of root directories. Earlier roots win
    /// on name collision.
    pub fn load_from_dirs(roots: &[PathBuf]) -> Self {
        let mut reg = Self::new();
        let mut seen: HashSet<String> = HashSet::new();
        for root in roots {
            if !root.is_dir() {
                continue;
            }
            collect_skills(root, root, &mut seen, &mut reg);
        }
        reg
    }

    /// Insert a skill — used by tests and by `load_from_dirs`. Returns true if
    /// it was added (false if a skill of that name already existed).
    pub fn insert(&mut self, def: SkillDef) -> bool {
        if self.skills.contains_key(&def.name) {
            return false;
        }
        self.skills.insert(def.name.clone(), def);
        true
    }

    pub fn get(&self, name: &str) -> Option<&SkillDef> {
        self.skills.get(name)
    }

    pub fn len(&self) -> usize {
        self.skills.len()
    }

    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &SkillDef)> {
        self.skills.iter()
    }

    /// Render the index suitable for injection into the system prompt. Returns
    /// `None` when there are no skills — caller can then skip the block.
    ///
    /// When `max_chars` is `Some`, the index will be **adaptively trimmed** to
    /// fit: skills are ordered by ascending description length (highest
    /// information density first) and any overflow is truncated with a
    /// `...and N more` marker. When `None`, the full index is returned.
    pub fn prompt_index_with_budget(&self, max_chars: Option<usize>) -> Option<String> {
        if self.skills.is_empty() {
            return None;
        }

        let header = "# Available Skills\n\n\
             Reusable workflows discovered from `.nocode/skills/` and `.claude/skills/`. \
             Invoke one by calling the `Skill` tool with its name; the full body is \
             only materialized then.\n\n";

        // Sort by description length so the densest entries land first when we
        // run up against a budget.
        let mut entries: Vec<&SkillDef> = self.skills.values().collect();
        entries.sort_by_key(|d| d.description.len());

        let mut out = String::from(header);
        let mut included = 0usize;
        let total = entries.len();

        for def in &entries {
            let line = format!("- **{}** — {}\n", def.name, def.description);
            // Reserve a generous margin for the closing footer so we never
            // overflow when adding it in a moment.
            let projected = out.len() + line.len() + 64;
            if let Some(budget) = max_chars
                && projected > budget
            {
                break;
            }
            out.push_str(&line);
            included += 1;
        }

        if included < total {
            let remaining = total - included;
            out.push_str(&format!(
                "\n_...and {remaining} more skill(s) — call `Skill(skill=\"<name>\")` if you know one by name._\n"
            ));
        }

        Some(out)
    }

    /// Backwards-compatible wrapper for the un-budgeted index.
    pub fn prompt_index(&self) -> Option<String> {
        self.prompt_index_with_budget(None)
    }
}

fn collect_skills(
    root: &Path,
    dir: &Path,
    seen: &mut HashSet<String>,
    reg: &mut SkillRegistry,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // One level of namespacing: {root}/{ns}/{skill}.md → name "ns:skill"
            if path.parent() == Some(root) {
                collect_skills(root, &path, seen, reg);
            }
            continue;
        }
        if path.extension().is_some_and(|e| e == "md")
            && let Some(def) = load_skill_file(root, &path)
            && seen.insert(def.name.clone())
        {
            reg.skills.insert(def.name.clone(), def);
        }
    }
}

fn load_skill_file(root: &Path, path: &Path) -> Option<SkillDef> {
    let content = fs::read_to_string(path).ok()?;
    let (frontmatter, body) = parse_frontmatter(&content);

    let stem = path.file_stem()?.to_str()?.to_owned();
    let parent_ns = path
        .parent()
        .and_then(|p| if p == root { None } else { p.file_name() })
        .and_then(|n| n.to_str())
        .map(str::to_owned);

    let name = frontmatter.name.clone().unwrap_or_else(|| {
        parent_ns
            .as_ref()
            .map_or_else(|| stem.clone(), |ns| format!("{ns}:{stem}"))
    });

    let description = frontmatter.description.clone().unwrap_or_else(|| {
        body.lines()
            .find_map(|l| {
                let t = l.trim();
                if t.is_empty() || t.starts_with('#') {
                    None
                } else {
                    Some(t.to_owned())
                }
            })
            .unwrap_or_else(|| format!("Skill defined in {}", path.display()))
            .chars()
            .take(200)
            .collect()
    });

    Some(SkillDef {
        name,
        description,
        frontmatter,
        body,
        path: path.to_path_buf(),
    })
}

/// Split `---`-delimited YAML frontmatter from the body. The parser is
/// intentionally minimal — only flat scalars and one-level lists. Anything more
/// exotic is ignored (frontmatter stays parsed-best-effort).
fn parse_frontmatter(content: &str) -> (SkillFrontmatter, String) {
    let trimmed = content.trim_start_matches('\u{feff}');
    if !trimmed.starts_with("---") {
        return (SkillFrontmatter::default(), content.to_owned());
    }
    // Find closing "---" on its own line.
    let after_open = &trimmed[3..];
    let Some(end_rel) = after_open.find("\n---") else {
        return (SkillFrontmatter::default(), content.to_owned());
    };
    let raw_fm = after_open[..end_rel].trim_start_matches('\n');
    let after_close = &after_open[end_rel + 4..]; // skip "\n---"
    let body = after_close.trim_start_matches('\n').to_owned();

    let mut fm = SkillFrontmatter::default();
    let mut current_list_key: Option<&str> = None;
    for raw_line in raw_fm.lines() {
        let line = raw_line.trim_end();
        if line.is_empty() {
            current_list_key = None;
            continue;
        }
        if let Some(item) = line.strip_prefix("  - ").or_else(|| line.strip_prefix("- ")) {
            if let Some(key) = current_list_key {
                push_list_item(&mut fm, key, item.trim().trim_matches('"'));
            }
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            let value = value.trim();
            if value.is_empty() {
                current_list_key = match key {
                    "triggers" => Some("triggers"),
                    _ => None,
                };
                continue;
            }
            current_list_key = None;
            let value = value.trim_matches('"');
            match key {
                "name" => fm.name = Some(value.to_owned()),
                "description" => fm.description = Some(value.to_owned()),
                "triggers" => {
                    fm.triggers = parse_inline_list(value);
                }
                _ => {}
            }
        }
    }

    (fm, body)
}

fn push_list_item(fm: &mut SkillFrontmatter, key: &str, item: &str) {
    if key == "triggers" {
        fm.triggers.push(item.to_owned());
    }
}

fn parse_inline_list(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    if let Some(inside) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        return inside
            .split(',')
            .map(|s| s.trim().trim_matches('"').to_owned())
            .filter(|s| !s.is_empty())
            .collect();
    }
    if trimmed.is_empty() {
        Vec::new()
    } else {
        vec![trimmed.to_owned()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_skill(root: &Path, rel: &str, content: &str) {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    #[test]
    fn parses_frontmatter_with_description_and_triggers() {
        let raw = "---\nname: commit-and-push\ndescription: Stage and push.\ntriggers:\n  - commit\n  - push\n---\nBody here.";
        let (fm, body) = parse_frontmatter(raw);
        assert_eq!(fm.name.as_deref(), Some("commit-and-push"));
        assert_eq!(fm.description.as_deref(), Some("Stage and push."));
        assert_eq!(fm.triggers, vec!["commit".to_owned(), "push".to_owned()]);
        assert_eq!(body, "Body here.");
    }

    #[test]
    fn parses_inline_triggers_list() {
        let raw = "---\ntriggers: [\"a\", \"b\", \"c\"]\n---\nbody";
        let (fm, _) = parse_frontmatter(raw);
        assert_eq!(fm.triggers, vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]);
    }

    #[test]
    fn missing_frontmatter_returns_default() {
        let raw = "# No frontmatter\n\nJust body.";
        let (fm, body) = parse_frontmatter(raw);
        assert_eq!(fm, SkillFrontmatter::default());
        assert_eq!(body, raw);
    }

    #[test]
    fn description_falls_back_to_first_prose_line() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join(".nocode/skills");
        write_skill(&root, "foo.md", "# Heading\n\nDo the thing.\nMore.\n");
        let reg = SkillRegistry::load_from_dirs(std::slice::from_ref(&root));
        let def = reg.get("foo").expect("skill loaded");
        assert_eq!(def.description, "Do the thing.");
    }

    #[test]
    fn earlier_root_wins_on_name_collision() {
        let tmp = TempDir::new().unwrap();
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        write_skill(&a, "same.md", "---\ndescription: from A\n---\nbody");
        write_skill(&b, "same.md", "---\ndescription: from B\n---\nbody");
        let reg = SkillRegistry::load_from_dirs(&[a, b]);
        assert_eq!(reg.get("same").unwrap().description, "from A");
    }

    #[test]
    fn namespaced_subdir_becomes_ns_colon_name() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        write_skill(&root, "git/commit.md", "---\ndescription: Commit.\n---\nbody");
        let reg = SkillRegistry::load_from_dirs(&[root]);
        assert!(reg.get("git:commit").is_some(), "namespaced skill not found");
    }

    #[test]
    fn prompt_index_lists_all_with_descriptions() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        write_skill(&root, "alpha.md", "---\ndescription: A.\n---\nbody");
        write_skill(&root, "beta.md", "---\ndescription: B.\n---\nbody");
        let reg = SkillRegistry::load_from_dirs(&[root]);
        let idx = reg.prompt_index().expect("non-empty");
        assert!(idx.contains("**alpha** — A."));
        assert!(idx.contains("**beta** — B."));
    }

    #[test]
    fn prompt_index_adaptive_trims_to_budget() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        write_skill(&root, "short.md", "---\ndescription: x\n---\n");
        write_skill(
            &root,
            "huge.md",
            &format!("---\ndescription: {}\n---\n", "y".repeat(1000)),
        );
        write_skill(&root, "tiny.md", "---\ndescription: z\n---\n");

        let reg = SkillRegistry::load_from_dirs(&[root]);
        let idx = reg.prompt_index_with_budget(Some(400)).expect("non-empty");
        assert!(idx.contains("short") || idx.contains("tiny"));
        assert!(idx.contains("more skill"), "footer missing: {idx}");
        assert!(!idx.contains(&"y".repeat(500)));
    }

    #[test]
    fn prompt_index_no_footer_when_everything_fits() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        write_skill(&root, "a.md", "---\ndescription: A.\n---\n");
        let reg = SkillRegistry::load_from_dirs(&[root]);
        let idx = reg
            .prompt_index_with_budget(Some(10_000))
            .expect("non-empty");
        assert!(!idx.contains("more skill"));
    }

    #[test]
    fn empty_registry_returns_no_prompt_index() {
        let reg = SkillRegistry::new();
        assert!(reg.prompt_index().is_none());
    }

    #[test]
    fn render_substitutes_arguments() {
        let def = SkillDef {
            name: "x".to_owned(),
            description: "x".to_owned(),
            frontmatter: SkillFrontmatter::default(),
            body: "Run: $ARGUMENTS and ${ARGUMENTS}".to_owned(),
            path: PathBuf::from("x"),
        };
        assert_eq!(def.render("foo"), "Run: foo and foo");
    }
}
