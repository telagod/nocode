//! TUI entry point — thin wrapper around ratatui-based tui_app.

use crate::repl::ReplSession;
use crossterm::cursor::{Hide, Show};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use nocode_core::QueryEngine;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::{self, IsTerminal};
use std::sync::mpsc;

pub(crate) fn run_tui() -> io::Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(io::Error::other("nocode tui requires an interactive TTY"));
    }

    // Terminal setup
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, Hide)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Session setup
    let config = crate::bootstrap_config();
    crate::wire_task_coordinator(&config.session_id);
    let mut engine: Option<QueryEngine> = Some(QueryEngine::new(config));
    let mut session = ReplSession::new("nocode");
    session.set_tui_mode(true);

    // Permission channel
    let (permission_tx, permission_rx) = mpsc::channel();
    session.set_permission_rx(permission_rx);
    let tui_prompter =
        crate::tui_permission::TuiPermissionPrompter::with_default_timeout(permission_tx);
    session.set_tui_prompter(tui_prompter);

    // Run the ratatui app loop
    let result = crate::tui_app::run_app_loop(&mut terminal, &mut session, &mut engine);

    // Cleanup
    disable_raw_mode()?;
    execute!(io::stdout(), Show, LeaveAlternateScreen)?;

    result
}
