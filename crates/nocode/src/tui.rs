//! TUI entry point — thin wrapper around ratatui-based tui_app.

use crossterm::cursor::{Hide, Show};
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use nocode_core::message::SystemBlock;
use nocode_core::provider::Provider;
use nocode_core::tool::ToolRegistry;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::{self, IsTerminal};

/// Run the TUI with a pre-built provider and config.
pub(crate) fn run_tui(
    provider: Box<dyn Provider>,
    registry: ToolRegistry,
    system: Vec<SystemBlock>,
    model: String,
    max_tokens: u32,
    max_turns: u32,
    warnings: Vec<String>,
) -> io::Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(io::Error::other("nocode tui requires an interactive TTY"));
    }

    // Terminal setup
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableBracketedPaste,
        EnableMouseCapture,
        Hide
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // Run the ratatui app loop
    let result = crate::tui_app::run_app_loop(
        &mut terminal,
        provider,
        registry,
        system,
        &model,
        max_tokens,
        max_turns,
        warnings,
    );

    // Cleanup
    disable_raw_mode()?;
    execute!(
        io::stdout(),
        Show,
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;

    result
}
