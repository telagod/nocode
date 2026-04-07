use crate::message::SystemBlock;

/// Build the base system prompt for nocode.
pub fn base_system_prompt(cwd: &str) -> SystemBlock {
    SystemBlock::text(format!(
        "You are nocode, a terminal-native AI coding assistant.\n\
         \n\
         # Environment\n\
         - Working directory: {cwd}\n\
         - Platform: {os}\n\
         \n\
         # Available tools\n\
         You have access to tools for reading, writing, and editing files, \
         running shell commands, searching code, and finding files.\n\
         \n\
         # Guidelines\n\
         - Be concise and direct\n\
         - Use tools to explore and modify the codebase\n\
         - Read files before editing them\n\
         - Prefer Edit over Write for modifying existing files\n\
         - When running shell commands, prefer non-interactive commands\n\
         - Show your work: explain what you find and what you change",
        os = std::env::consts::OS
    ))
}

/// Assemble the full system prompt from base + CLAUDE.md + extras.
pub fn assemble_system_prompt(
    cwd: &str,
    claude_md: Option<&str>,
    extra: Option<&str>,
) -> Vec<SystemBlock> {
    let mut blocks = vec![base_system_prompt(cwd)];

    if let Some(md) = claude_md {
        blocks.push(SystemBlock::text(md));
    }

    if let Some(e) = extra {
        blocks.push(SystemBlock::text(e));
    }

    blocks
}
