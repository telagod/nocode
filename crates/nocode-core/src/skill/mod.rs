//! Skill registry — first-class skill system, loaded into prompt assembly.
//!
//! A **skill** is a Markdown file with optional YAML frontmatter that describes a
//! repeatable workflow (e.g. "commit", "review-pr", "verify-security"). Skills are
//! first-class in nocode: they are discovered from `.claude/skills/` and
//! `.nocode/skills/` directories and injected into the system prompt as an
//! *index* (name + description), letting the model decide when to invoke one via
//! the `Skill` tool. Only the chosen skill's body is materialized — the index
//! stays cheap.
//!
//! This is the harness-bionics fix for the old `tool/skill.rs` which treated
//! skills as a wrapper tool with no model-visible inventory.

pub mod registry;

pub use registry::{SkillDef, SkillFrontmatter, SkillRegistry};
