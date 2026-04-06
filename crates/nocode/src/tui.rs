use crate::markdown_render::{LineSegment, render_markdown_to_lines};
use crate::markdown_stream::MarkdownStreamState;
use crate::repl::{ReplIntent, ReplSession};
use crate::spinner::Spinner;
use crate::tool_render;
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::queue;
use crossterm::style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor};
use crossterm::terminal::{
    self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
    enable_raw_mode,
};
use nocode_core::{ChannelModelStreamSink, QueryEngine};
use std::io::{self, IsTerminal, Stdout, Write};
use std::sync::mpsc;
use std::time::Duration;

const LOG_LIMIT: usize = 160;
const MIN_TUI_WIDTH: u16 = 72;
const MIN_TUI_HEIGHT: u16 = 20;
const HEADER_ROWS: u16 = 4;
const FOOTER_ROWS: u16 = 4;

pub(crate) fn run_tui() -> io::Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(io::Error::other("nocode tui requires an interactive TTY"));
    }
    let mut stdout = io::stdout();
    let _guard = TuiTerminalGuard::enter(&mut stdout)?;
    let mut engine: Option<QueryEngine> = Some(QueryEngine::new(crate::bootstrap_config()));
    let mut session = ReplSession::new("nocode");
    session.set_tui_mode(true);

    // W2: Create permission channel and inject into session.
    let (permission_tx, permission_rx) = mpsc::channel();
    session.set_permission_rx(permission_rx);
    // permission_tx is available for bridge/tool transports to send requests.
    let _permission_tx = permission_tx;

    let mut app = TuiApp::new();
    app.push_block("tui ready: Alt-1..4 focus pane, ? help, F2 inspector, F3 permissions");

    loop {
        // W1: If a deferred intent is waiting and engine is available, launch async model call.
        if let Some(intent) = session.take_pending_intent() {
            if let Some(eng) = engine.take() {
                launch_async_submission(&mut session, eng, intent);
                app.thinking_spinner = Some(Spinner::new("\u{1F980} Thinking..."));
            } else {
                // Engine is busy in a background thread — re-queue the intent.
                app.push_block("engine busy — waiting for current submission to complete");
            }
        }

        // Tick tasks only when engine is available.
        if let Some(eng) = engine.as_ref() {
            let mut tick_output = Vec::new();
            session.tick_tasks(eng, &mut tick_output)?;
            app.capture_output(tick_output);
        }

        // Tick the thinking spinner while streaming.
        if let Some(spinner) = app.thinking_spinner.as_mut()
            && !spinner.is_done()
        {
            let frame = spinner.tick();
            app.push_block(frame.display.as_str());
        }

        // W1: Poll pending stream events for live rendering.
        let (stream_lines, returned_engine) = session.poll_pending_stream();
        for line in &stream_lines {
            // Route tool call/result lines through tool_render for formatted output.
            if line.contains("[CALL]") || line.contains("[DONE]") {
                app.push_tool_event(line.as_str());
            } else if line.starts_with("stream delta:") {
                // Route stream deltas through MarkdownStreamState for boundary detection.
                let text = line.strip_prefix("stream delta: ").unwrap_or(line.as_str());
                if let Some(rendered) = app.md_stream.push(text) {
                    app.push_markdown_block(rendered.as_str());
                }
            } else {
                app.push_block(line.as_str());
            }
        }
        if let Some(eng) = returned_engine {
            // Flush remaining markdown buffer when stream ends.
            if let Some(rendered) = app.md_stream.flush() {
                app.push_markdown_block(rendered.as_str());
            }
            if let Some(spinner) = app.thinking_spinner.as_mut() {
                spinner.finish("Done");
            }
            app.thinking_spinner = None;
            engine = Some(eng);
        }

        // W2: Poll incoming permission requests.
        if session.poll_permissions() {
            app.set_overlay_permissions();
        }

        if app.should_render() {
            app.render(&mut stdout, &session)?;
        }

        // W1: Adaptive poll timeout — faster when streaming.
        let poll_ms = if session.is_streaming() || session.has_pending_permissions() {
            16
        } else {
            120
        };
        if !event::poll(Duration::from_millis(poll_ms))? {
            continue;
        }

        match event::read()? {
            Event::Key(key) => {
                if let Some(eng) = engine.as_mut() {
                    if !app.handle_key(key, &mut session, eng)? {
                        break;
                    }
                } else {
                    // Engine is busy — only handle overlay/navigation keys, not commands.
                    if !app.handle_key_no_engine(key, &mut session)? {
                        break;
                    }
                }
            }
            Event::Resize(_, _) => app.mark_dirty(),
            _ => {}
        }
    }

    Ok(())
}

/// Launch an async model call on a background thread.
/// The engine is moved into the thread and returned via the result channel when done.
fn launch_async_submission(
    session: &mut ReplSession,
    engine: QueryEngine,
    intent: ReplIntent,
) -> Option<QueryEngine> {
    use crate::repl::PendingSubmission;

    let (stream_tx, stream_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();

    session.set_pending_submission(PendingSubmission {
        stream_rx,
        result_rx,
        accumulated_text: String::new(),
        started: false,
    });

    std::thread::spawn(move || {
        let mut eng = engine;
        let mut sink = ChannelModelStreamSink::new(stream_tx);
        let plan = eng.submit_message_with_stream(intent.prompt, intent.options, &mut sink);
        let _ = result_tx.send((plan, eng));
    });

    None
}

struct TuiTerminalGuard;

impl TuiTerminalGuard {
    fn enter(stdout: &mut Stdout) -> io::Result<Self> {
        enable_raw_mode()?;
        execute!(stdout, EnterAlternateScreen, Hide)?;
        Ok(Self)
    }
}

impl Drop for TuiTerminalGuard {
    fn drop(&mut self) {
        let mut stdout = io::stdout();
        let _ = execute!(stdout, Show, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

/// A single line in the transcript/events log — either plain text or pre-styled segments.
#[derive(Debug, Clone)]
enum StyledContent {
    Plain(String),
    Styled(Vec<LineSegment>),
}

impl StyledContent {
    /// Return the plain-text representation (for filtering / wrapping).
    fn as_plain(&self) -> String {
        match self {
            Self::Plain(s) => s.clone(),
            Self::Styled(segs) => segs.iter().map(|s| s.text.as_str()).collect(),
        }
    }
}

impl Default for StyledContent {
    fn default() -> Self {
        Self::Plain(String::new())
    }
}

#[derive(Debug, Default)]
struct TuiApp {
    input: String,
    cursor_chars: usize,
    styled_lines: Vec<StyledContent>,
    events_filter: TuiEventsFilter,
    active_pane: TuiPane,
    overlay: TuiOverlay,
    transcript_scroll: usize,
    task_list_scroll: usize,
    task_detail_scroll: usize,
    events_scroll: usize,
    detail_follow_selection: bool,
    dirty: bool,
    thinking_spinner: Option<Spinner>,
    md_stream: MarkdownStreamState,
}

impl TuiApp {
    fn new() -> Self {
        Self {
            active_pane: TuiPane::Transcript,
            detail_follow_selection: true,
            dirty: true,
            ..Self::default()
        }
    }

    fn handle_key(
        &mut self,
        key: KeyEvent,
        session: &mut ReplSession,
        engine: &mut QueryEngine,
    ) -> io::Result<bool> {
        if matches!(
            (key.code, key.modifiers),
            (KeyCode::Char('c'), KeyModifiers::CONTROL)
        ) {
            return Ok(false);
        }

        if self.handle_overlay_toggle(key) {
            return Ok(true);
        }

        if self.overlay.is_open() {
            return match (self.overlay, key.code) {
                (_, KeyCode::Esc) => {
                    self.overlay = TuiOverlay::None;
                    self.mark_dirty();
                    Ok(true)
                }
                // W2: Permission overlay keys
                (TuiOverlay::Permissions, KeyCode::Char('a' | 'A')) => {
                    session.resolve_permission(true);
                    if !session.has_pending_permissions() {
                        self.overlay = TuiOverlay::None;
                    }
                    self.mark_dirty();
                    Ok(true)
                }
                (TuiOverlay::Permissions, KeyCode::Char('d' | 'D')) => {
                    session.resolve_permission(false);
                    if !session.has_pending_permissions() {
                        self.overlay = TuiOverlay::None;
                    }
                    self.mark_dirty();
                    Ok(true)
                }
                (TuiOverlay::Permissions, KeyCode::Up) => {
                    session.permission_cursor_up();
                    self.mark_dirty();
                    Ok(true)
                }
                (TuiOverlay::Permissions, KeyCode::Down) => {
                    session.permission_cursor_down();
                    self.mark_dirty();
                    Ok(true)
                }
                _ => Ok(true),
            };
        }

        match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => Ok(false),
            (KeyCode::Tab, _) => self.rotate_focus(session, engine, 1),
            (KeyCode::BackTab, _) => self.rotate_focus(session, engine, -1),
            (KeyCode::Char('1'), KeyModifiers::ALT) => {
                self.focus_pane(session, engine, TuiPane::Transcript)
            }
            (KeyCode::Char('2'), KeyModifiers::ALT) => {
                self.focus_pane(session, engine, TuiPane::TaskList)
            }
            (KeyCode::Char('3'), KeyModifiers::ALT) => {
                self.focus_pane(session, engine, TuiPane::TaskDetail)
            }
            (KeyCode::Char('4'), KeyModifiers::ALT) => {
                self.focus_pane(session, engine, TuiPane::Events)
            }
            (KeyCode::Up, _) => self.handle_up(session, engine),
            (KeyCode::Down, _) => self.handle_down(session, engine),
            (KeyCode::PageUp, _) => {
                self.scroll_current_pane(8);
                Ok(true)
            }
            (KeyCode::PageDown, _) => {
                self.scroll_current_pane_back(8);
                Ok(true)
            }
            (KeyCode::Home, _) => {
                self.scroll_current_pane_to_top();
                Ok(true)
            }
            (KeyCode::End, _) => {
                self.scroll_current_pane_to_bottom();
                Ok(true)
            }
            (KeyCode::Enter, _) => {
                if self.input.trim().is_empty() {
                    if self.active_pane == TuiPane::Events {
                        self.push_block("events pane: no enter action");
                        return Ok(true);
                    }
                    if self.active_pane == TuiPane::TaskDetail {
                        self.task_detail_scroll = 0;
                    }
                    self.dispatch(session, engine, "/enter")
                } else {
                    let line = std::mem::take(&mut self.input);
                    self.cursor_chars = 0;
                    session.clear_tui_draft();
                    self.mark_dirty();
                    self.dispatch(session, engine, line.as_str())
                }
            }
            (KeyCode::Char('f'), _)
                if self.input.is_empty() && self.active_pane == TuiPane::Events =>
            {
                self.events_filter = self.events_filter.step();
                self.push_block(format!("events filter: {}", self.events_filter.label()).as_str());
                Ok(true)
            }
            (KeyCode::Char('f'), _)
                if self.input.is_empty() && self.active_pane == TuiPane::TaskDetail =>
            {
                self.detail_follow_selection = !self.detail_follow_selection;
                self.push_block(if self.detail_follow_selection {
                    "detail follow: on"
                } else {
                    "detail follow: off"
                });
                Ok(true)
            }
            (KeyCode::Backspace, _) => {
                self.delete_left();
                self.sync_editor_to_session(session);
                Ok(true)
            }
            (KeyCode::Delete, _) => {
                self.delete_right();
                self.sync_editor_to_session(session);
                Ok(true)
            }
            (KeyCode::Left, _) => {
                self.cursor_chars = self.cursor_chars.saturating_sub(1);
                self.mark_dirty();
                Ok(true)
            }
            (KeyCode::Right, _) => {
                self.cursor_chars = (self.cursor_chars + 1).min(self.input.chars().count());
                self.mark_dirty();
                Ok(true)
            }
            (KeyCode::Char('a'), KeyModifiers::CONTROL) => {
                self.cursor_chars = 0;
                self.mark_dirty();
                Ok(true)
            }
            (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
                self.cursor_chars = self.input.chars().count();
                self.mark_dirty();
                Ok(true)
            }
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                self.input.clear();
                self.cursor_chars = 0;
                self.sync_editor_to_session(session);
                Ok(true)
            }
            (KeyCode::Char('l'), KeyModifiers::CONTROL) => {
                self.styled_lines.clear();
                self.mark_dirty();
                Ok(true)
            }
            (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
                self.dispatch(session, engine, "/history-prev")
            }
            (KeyCode::Char('n'), KeyModifiers::CONTROL) => {
                self.dispatch(session, engine, "/history-next")
            }
            (KeyCode::Char(ch), modifiers)
                if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT =>
            {
                self.insert_char(ch);
                self.sync_editor_to_session(session);
                Ok(true)
            }
            _ => Ok(true),
        }
    }

    fn handle_overlay_toggle(&mut self, key: KeyEvent) -> bool {
        let next = match (key.code, key.modifiers) {
            (KeyCode::Char('?'), _) | (KeyCode::F(1), _) => Some(TuiOverlay::Help),
            (KeyCode::F(2), _) => Some(TuiOverlay::Inspector),
            (KeyCode::F(3), _) => Some(TuiOverlay::Permissions),
            _ => None,
        };
        if let Some(next) = next {
            self.overlay = if self.overlay == next {
                TuiOverlay::None
            } else {
                next
            };
            self.mark_dirty();
            true
        } else {
            false
        }
    }

    fn dispatch(
        &mut self,
        session: &mut ReplSession,
        engine: &mut QueryEngine,
        line: &str,
    ) -> io::Result<bool> {
        let mut output = Vec::new();
        let keep_running = session.process_local_line(engine, &mut output, line)?;
        self.capture_output(output);
        if self.active_pane != TuiPane::Events || line.trim_start().starts_with("/focus ") {
            self.active_pane = TuiPane::from_session_focus(session.focus_label());
        }
        self.sync_editor_from_session(session);
        Ok(keep_running)
    }

    fn focus_pane(
        &mut self,
        session: &mut ReplSession,
        engine: &mut QueryEngine,
        pane: TuiPane,
    ) -> io::Result<bool> {
        self.active_pane = pane;
        self.mark_dirty();
        match pane {
            TuiPane::Transcript => self.dispatch(session, engine, "/focus transcript"),
            TuiPane::TaskList => self.dispatch(session, engine, "/focus tasks"),
            TuiPane::TaskDetail => self.dispatch(session, engine, "/focus detail"),
            TuiPane::Events => Ok(true),
        }
    }

    fn rotate_focus(
        &mut self,
        session: &mut ReplSession,
        engine: &mut QueryEngine,
        direction: i8,
    ) -> io::Result<bool> {
        self.focus_pane(session, engine, self.active_pane.step(direction))
    }

    fn handle_up(
        &mut self,
        session: &mut ReplSession,
        engine: &mut QueryEngine,
    ) -> io::Result<bool> {
        match self.active_pane {
            TuiPane::Transcript => {
                self.transcript_scroll = self.transcript_scroll.saturating_add(1);
                self.mark_dirty();
                Ok(true)
            }
            TuiPane::TaskDetail if !self.detail_follow_selection => {
                self.task_detail_scroll = self.task_detail_scroll.saturating_add(1);
                self.mark_dirty();
                Ok(true)
            }
            TuiPane::TaskDetail => {
                self.task_detail_scroll = 0;
                self.dispatch(session, engine, "/k")
            }
            TuiPane::TaskList => {
                self.task_list_scroll = 0;
                self.dispatch(session, engine, "/k")
            }
            TuiPane::Events => {
                self.events_scroll = self.events_scroll.saturating_add(1);
                self.mark_dirty();
                Ok(true)
            }
        }
    }

    fn handle_down(
        &mut self,
        session: &mut ReplSession,
        engine: &mut QueryEngine,
    ) -> io::Result<bool> {
        match self.active_pane {
            TuiPane::Transcript => {
                self.transcript_scroll = self.transcript_scroll.saturating_sub(1);
                self.mark_dirty();
                Ok(true)
            }
            TuiPane::TaskDetail if !self.detail_follow_selection => {
                self.task_detail_scroll = self.task_detail_scroll.saturating_sub(1);
                self.mark_dirty();
                Ok(true)
            }
            TuiPane::TaskDetail => {
                self.task_detail_scroll = 0;
                self.dispatch(session, engine, "/j")
            }
            TuiPane::TaskList => {
                self.task_list_scroll = 0;
                self.dispatch(session, engine, "/j")
            }
            TuiPane::Events => {
                self.events_scroll = self.events_scroll.saturating_sub(1);
                self.mark_dirty();
                Ok(true)
            }
        }
    }

    fn sync_editor_to_session(&mut self, session: &mut ReplSession) {
        session.set_tui_draft(self.input.clone());
        self.mark_dirty();
    }

    fn sync_editor_from_session(&mut self, session: &ReplSession) {
        let draft = session.tui_draft();
        if self.input != draft {
            self.input = draft.to_string();
            self.cursor_chars = self.input.chars().count();
            self.mark_dirty();
        }
    }

    fn scroll_current_pane(&mut self, amount: usize) {
        match self.active_pane {
            TuiPane::Transcript => {
                self.transcript_scroll = self.transcript_scroll.saturating_add(amount)
            }
            TuiPane::TaskList => {
                self.task_list_scroll = self.task_list_scroll.saturating_add(amount)
            }
            TuiPane::TaskDetail => {
                self.task_detail_scroll = self.task_detail_scroll.saturating_add(amount)
            }
            TuiPane::Events => self.events_scroll = self.events_scroll.saturating_add(amount),
        }
        self.mark_dirty();
    }

    fn scroll_current_pane_back(&mut self, amount: usize) {
        match self.active_pane {
            TuiPane::Transcript => {
                self.transcript_scroll = self.transcript_scroll.saturating_sub(amount)
            }
            TuiPane::TaskList => {
                self.task_list_scroll = self.task_list_scroll.saturating_sub(amount)
            }
            TuiPane::TaskDetail => {
                self.task_detail_scroll = self.task_detail_scroll.saturating_sub(amount)
            }
            TuiPane::Events => self.events_scroll = self.events_scroll.saturating_sub(amount),
        }
        self.mark_dirty();
    }

    fn scroll_current_pane_to_top(&mut self) {
        match self.active_pane {
            TuiPane::Transcript => self.transcript_scroll = usize::MAX / 4,
            TuiPane::TaskList => self.task_list_scroll = usize::MAX / 4,
            TuiPane::TaskDetail => self.task_detail_scroll = usize::MAX / 4,
            TuiPane::Events => self.events_scroll = usize::MAX / 4,
        }
        self.mark_dirty();
    }

    fn scroll_current_pane_to_bottom(&mut self) {
        match self.active_pane {
            TuiPane::Transcript => self.transcript_scroll = 0,
            TuiPane::TaskList => self.task_list_scroll = 0,
            TuiPane::TaskDetail => self.task_detail_scroll = 0,
            TuiPane::Events => self.events_scroll = 0,
        }
        self.mark_dirty();
    }

    fn capture_output(&mut self, output: Vec<u8>) {
        if output.is_empty() {
            return;
        }
        let rendered = String::from_utf8_lossy(&output);
        self.push_block(rendered.as_ref());
    }

    fn push_block(&mut self, block: &str) {
        let trimmed = block.trim_matches('\n');
        if trimmed.is_empty() {
            return;
        }
        self.styled_lines
            .extend(trimmed.lines().map(|l| StyledContent::Plain(l.to_string())));
        if self.styled_lines.len() > LOG_LIMIT {
            let overflow = self.styled_lines.len() - LOG_LIMIT;
            self.styled_lines.drain(0..overflow);
        }
        self.mark_dirty();
    }

    /// Push a block of markdown through the renderer, storing styled segments directly.
    fn push_markdown_block(&mut self, markdown: &str) {
        let rendered_lines = render_markdown_to_lines(markdown);
        for line in rendered_lines {
            if !line.is_empty() {
                self.styled_lines.push(StyledContent::Styled(line.segments));
            }
        }
        if self.styled_lines.len() > LOG_LIMIT {
            let overflow = self.styled_lines.len() - LOG_LIMIT;
            self.styled_lines.drain(0..overflow);
        }
        self.mark_dirty();
    }

    /// Push a tool call/result event through tool_render for formatted output.
    fn push_tool_event(&mut self, line: &str) {
        // Extract tool name and content from "[CALL] tool-message: ..." or "[DONE] ..." format
        let content = line
            .trim()
            .strip_prefix("[CALL]")
            .or_else(|| line.trim().strip_prefix("[DONE]"))
            .unwrap_or(line)
            .trim();

        let is_call = line.contains("[CALL]");

        if is_call {
            // Try to extract tool name from "tool-message: ToolName#id(...)" pattern
            let tool_name = content
                .strip_prefix("tool-message: ")
                .and_then(|s| s.split('#').next())
                .unwrap_or("Unknown");
            let formatted = tool_render::format_tool_call_start(
                tool_name,
                &serde_json::Value::String(content.to_string()),
            );
            for fline in &formatted {
                self.styled_lines
                    .push(StyledContent::Styled(vec![LineSegment {
                        text: fline.clone(),
                        color: crossterm::style::Color::Yellow,
                        bold: false,
                        italic: false,
                    }]));
            }
        } else {
            // Tool result
            let formatted = tool_render::format_tool_result("Unknown", content);
            for fline in &formatted {
                self.styled_lines
                    .push(StyledContent::Styled(vec![LineSegment {
                        text: fline.clone(),
                        color: crossterm::style::Color::DarkYellow,
                        bold: false,
                        italic: false,
                    }]));
            }
        }

        if self.styled_lines.len() > LOG_LIMIT {
            let overflow = self.styled_lines.len() - LOG_LIMIT;
            self.styled_lines.drain(0..overflow);
        }
        self.mark_dirty();
    }

    /// Open the permissions overlay.
    fn set_overlay_permissions(&mut self) {
        self.overlay = TuiOverlay::Permissions;
        self.mark_dirty();
    }

    /// Handle keys when the engine is busy (moved into background thread).
    /// Only overlay toggles, navigation, and Ctrl-C are processed.
    fn handle_key_no_engine(
        &mut self,
        key: KeyEvent,
        session: &mut ReplSession,
    ) -> io::Result<bool> {
        if matches!(
            (key.code, key.modifiers),
            (KeyCode::Char('c'), KeyModifiers::CONTROL)
        ) {
            return Ok(false);
        }

        if self.handle_overlay_toggle(key) {
            return Ok(true);
        }

        if self.overlay.is_open() {
            return match (self.overlay, key.code) {
                (_, KeyCode::Esc) => {
                    self.overlay = TuiOverlay::None;
                    self.mark_dirty();
                    Ok(true)
                }
                (TuiOverlay::Permissions, KeyCode::Char('a' | 'A')) => {
                    session.resolve_permission(true);
                    if !session.has_pending_permissions() {
                        self.overlay = TuiOverlay::None;
                    }
                    self.mark_dirty();
                    Ok(true)
                }
                (TuiOverlay::Permissions, KeyCode::Char('d' | 'D')) => {
                    session.resolve_permission(false);
                    if !session.has_pending_permissions() {
                        self.overlay = TuiOverlay::None;
                    }
                    self.mark_dirty();
                    Ok(true)
                }
                (TuiOverlay::Permissions, KeyCode::Up) => {
                    session.permission_cursor_up();
                    self.mark_dirty();
                    Ok(true)
                }
                (TuiOverlay::Permissions, KeyCode::Down) => {
                    session.permission_cursor_down();
                    self.mark_dirty();
                    Ok(true)
                }
                _ => Ok(true),
            };
        }

        // Scroll keys still work while engine is busy.
        match key.code {
            KeyCode::Up => {
                self.scroll_current_pane(1);
                Ok(true)
            }
            KeyCode::Down => {
                self.scroll_current_pane_back(1);
                Ok(true)
            }
            KeyCode::PageUp => {
                self.scroll_current_pane(8);
                Ok(true)
            }
            KeyCode::PageDown => {
                self.scroll_current_pane_back(8);
                Ok(true)
            }
            _ => Ok(true),
        }
    }

    fn filtered_log_lines(&self) -> Vec<&StyledContent> {
        self.styled_lines
            .iter()
            .filter(|line| self.events_filter.matches(line.as_plain().as_str()))
            .collect()
    }

    fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    fn should_render(&self) -> bool {
        self.dirty
    }

    fn render(&mut self, stdout: &mut Stdout, session: &ReplSession) -> io::Result<()> {
        let (width, height) = terminal::size()?;
        queue!(stdout, MoveTo(0, 0), Clear(ClearType::All))?;

        if width < MIN_TUI_WIDTH || height < MIN_TUI_HEIGHT {
            queue!(
                stdout,
                Print(format!(
                    "nocode tui: terminal too small (need at least {}x{})\r\n",
                    MIN_TUI_WIDTH, MIN_TUI_HEIGHT
                ))
            )?;
            stdout.flush()?;
            self.dirty = false;
            return Ok(());
        }

        let snapshot = session.render_tui_snapshot();
        let footer_top = height - FOOTER_ROWS;
        let body_top = HEADER_ROWS;
        let body_height = footer_top.saturating_sub(body_top);
        let events_height = if body_height >= 14 { 5 } else { 4 };
        let main_height = body_height.saturating_sub(events_height + 1);
        let left_width = (width.saturating_mul(3) / 5)
            .max(30)
            .min(width.saturating_sub(26));
        let right_width = width.saturating_sub(left_width + 1);
        let right_x = left_width + 1;
        let list_height = (main_height / 2).max(4).min(main_height.saturating_sub(4));
        let detail_top = body_top + list_height + 1;
        let detail_height = main_height.saturating_sub(list_height + 1);
        let events_top = body_top + main_height + 1;

        let title = format!(
            "NOCODE TUI | focus={} | overlay={} | detail-follow={} | events-filter={} | logs={}",
            self.active_pane.label(),
            self.overlay.label(),
            if self.detail_follow_selection {
                "on"
            } else {
                "off"
            },
            self.events_filter.label(),
            self.styled_lines.len()
        );
        self.draw_strip(stdout, 0, width, title.as_str())?;
        self.draw_strip(stdout, 1, width, snapshot.status_line.as_str())?;
        self.draw_strip(stdout, 2, width, snapshot.diagnostics_line.as_str())?;
        self.draw_strip(stdout, 3, width, snapshot.hud_line.as_str())?;

        self.draw_panel(
            stdout,
            0,
            body_top,
            left_width,
            main_height,
            split_block(snapshot.transcript.as_str()),
            self.active_pane == TuiPane::Transcript,
            ScrollAnchor::Bottom,
            self.transcript_scroll,
        )?;
        self.draw_panel(
            stdout,
            right_x,
            body_top,
            right_width,
            list_height,
            split_block(snapshot.task_list.as_str()),
            self.active_pane == TuiPane::TaskList,
            ScrollAnchor::Top,
            self.task_list_scroll,
        )?;
        self.draw_panel(
            stdout,
            right_x,
            detail_top,
            right_width,
            detail_height,
            split_block(snapshot.task_detail.as_str()),
            self.active_pane == TuiPane::TaskDetail,
            ScrollAnchor::Top,
            self.task_detail_scroll,
        )?;

        let filtered_log_lines = self.filtered_log_lines();
        let events_title = format!("events pane [{}]", self.events_filter.label());
        let events_body: Vec<StyledContent> = if filtered_log_lines.is_empty() {
            vec![StyledContent::Plain("none".to_string())]
        } else {
            filtered_log_lines.into_iter().cloned().collect()
        };
        self.draw_panel(
            stdout,
            0,
            events_top,
            width,
            events_height,
            PaneBlock {
                title: events_title,
                body: events_body,
            },
            self.active_pane == TuiPane::Events,
            ScrollAnchor::Bottom,
            self.events_scroll,
        )?;

        self.draw_strip(stdout, footer_top, width, snapshot.footer_line.as_str())?;
        self.draw_strip(stdout, footer_top + 1, width, snapshot.queue_line.as_str())?;
        self.draw_strip(stdout, footer_top + 2, width, snapshot.editor_line.as_str())?;

        let (input_line, cursor_col) =
            self.render_input_line(session.prompt_text().as_str(), width as usize);
        queue!(stdout, MoveTo(0, footer_top + 3), Print(input_line))?;

        if let Some(block) = self.overlay_block(session) {
            self.draw_overlay(stdout, width, height, block)?;
            queue!(stdout, Hide)?;
        } else {
            queue!(stdout, MoveTo(cursor_col, footer_top + 3), Show)?;
        }

        stdout.flush()?;
        self.dirty = false;
        Ok(())
    }

    fn overlay_block(&self, session: &ReplSession) -> Option<PaneBlock> {
        let to_body = |s: String| -> Vec<StyledContent> {
            s.lines()
                .map(|l| StyledContent::Plain(l.to_string()))
                .collect()
        };
        match self.overlay {
            TuiOverlay::None => None,
            TuiOverlay::Help => Some(PaneBlock {
                title: String::from("help overlay"),
                body: to_body(session.render_tui_help_overlay()),
            }),
            TuiOverlay::Inspector => Some(PaneBlock {
                title: String::from("inspector overlay"),
                body: to_body(session.render_tui_inspector_overlay()),
            }),
            TuiOverlay::Permissions => Some(PaneBlock {
                title: String::from("permission overlay"),
                body: to_body(session.render_tui_permission_overlay()),
            }),
        }
    }

    fn draw_strip(&self, stdout: &mut Stdout, row: u16, width: u16, text: &str) -> io::Result<()> {
        let color = match row {
            0 => Color::Cyan,       // title bar
            1 => Color::Green,      // status line
            2 => Color::DarkYellow, // diagnostics
            3 => Color::Magenta,    // HUD: model/tokens/timing
            _ => Color::White,      // footer
        };
        queue!(
            stdout,
            MoveTo(0, row),
            SetForegroundColor(color),
            Print(pad_line(text, width as usize)),
            ResetColor
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_panel(
        &self,
        stdout: &mut Stdout,
        x: u16,
        top: u16,
        width: u16,
        height: u16,
        block: PaneBlock,
        active: bool,
        anchor: ScrollAnchor,
        scroll: usize,
    ) -> io::Result<()> {
        if width < 4 || height < 3 {
            return Ok(());
        }
        let inner_width = width.saturating_sub(2) as usize;
        let inner_height = height.saturating_sub(2) as usize;
        let body_lines: Vec<StyledContent> = block
            .body
            .iter()
            .flat_map(|sc| wrap_styled_content(sc, inner_width))
            .collect();
        if body_lines.is_empty() {
            // ensure at least one empty line for viewport_summary
        }
        let (start, end, total) = viewport_summary(body_lines.len(), inner_height, anchor, scroll);
        let title = if total == 0 {
            format!("{} {} [0-0/0]", if active { '*' } else { ' ' }, block.title)
        } else {
            format!(
                "{} {} [{}-{}/{}]",
                if active { '*' } else { ' ' },
                block.title,
                start + 1,
                end,
                total
            )
        };
        let border_color = if active { Color::Cyan } else { Color::DarkGrey };
        let top_border = format!(
            "+{}+",
            render_panel_title(title.as_str(), inner_width, active)
        );
        let bottom_border = format!("+{}+", "-".repeat(inner_width));
        queue!(
            stdout,
            MoveTo(x, top),
            SetForegroundColor(border_color),
            Print(top_border),
            ResetColor
        )?;
        for row in 0..inner_height {
            let sc = body_lines.get(start + row);
            queue!(
                stdout,
                MoveTo(x, top + 1 + row as u16),
                SetForegroundColor(border_color),
                Print("|"),
            )?;
            let chars_written = match sc {
                Some(StyledContent::Styled(segs)) => {
                    let mut written = 0usize;
                    for seg in segs {
                        if seg.bold {
                            queue!(stdout, SetAttribute(Attribute::Bold))?;
                        }
                        if seg.italic {
                            queue!(stdout, SetAttribute(Attribute::Italic))?;
                        }
                        queue!(stdout, SetForegroundColor(seg.color), Print(&seg.text),)?;
                        written += seg.text.chars().count();
                        if seg.bold || seg.italic {
                            queue!(stdout, SetAttribute(Attribute::Reset))?;
                        }
                    }
                    queue!(stdout, ResetColor)?;
                    written
                }
                Some(StyledContent::Plain(line)) => {
                    let line_color = classify_line_color(line);
                    let display = pad_line(line, inner_width);
                    queue!(
                        stdout,
                        SetForegroundColor(line_color),
                        Print(display),
                        ResetColor,
                    )?;
                    inner_width // pad_line always fills to inner_width
                }
                None => 0,
            };
            // Pad remaining space
            if chars_written < inner_width {
                queue!(stdout, Print(" ".repeat(inner_width - chars_written)))?;
            }
            queue!(
                stdout,
                SetForegroundColor(border_color),
                Print("|"),
                ResetColor
            )?;
        }
        queue!(
            stdout,
            MoveTo(x, top + height - 1),
            SetForegroundColor(border_color),
            Print(bottom_border),
            ResetColor
        )?;
        Ok(())
    }

    fn draw_overlay(
        &self,
        stdout: &mut Stdout,
        width: u16,
        height: u16,
        block: PaneBlock,
    ) -> io::Result<()> {
        let overlay_width = ((width as usize * 4) / 5).clamp(48, width.saturating_sub(4) as usize);
        let wrapped: Vec<StyledContent> = block
            .body
            .iter()
            .flat_map(|sc| wrap_styled_content(sc, overlay_width.saturating_sub(2)))
            .collect();
        let overlay_height = (wrapped.len() + 2).clamp(8, height.saturating_sub(4) as usize);
        let overlay_x = (width.saturating_sub(overlay_width as u16)) / 2;
        let overlay_y = (height.saturating_sub(overlay_height as u16)) / 2;
        self.draw_panel(
            stdout,
            overlay_x,
            overlay_y,
            overlay_width as u16,
            overlay_height as u16,
            block,
            true,
            ScrollAnchor::Top,
            0,
        )
    }

    fn render_input_line(&self, prompt: &str, width: usize) -> (String, u16) {
        let prefix = format!("{prompt}[input] ");
        let available = width.saturating_sub(prefix.len());
        let placeholder = self.input.is_empty();
        let source = if placeholder {
            "[type prompt or /help]"
        } else {
            self.input.as_str()
        };
        let total_chars = source.chars().count();
        let cursor = if placeholder {
            0
        } else {
            self.cursor_chars.min(total_chars)
        };
        let start = if cursor >= available {
            cursor + 1 - available
        } else {
            0
        };
        let visible = source
            .chars()
            .skip(start)
            .take(available)
            .collect::<String>();
        let line = format!("{prefix}{}", pad_line(visible.as_str(), available));
        let cursor_col = (prefix.len() + cursor.saturating_sub(start)) as u16;
        (line, cursor_col)
    }

    fn insert_char(&mut self, ch: char) {
        let byte_index = char_to_byte_index(self.input.as_str(), self.cursor_chars);
        self.input.insert(byte_index, ch);
        self.cursor_chars += 1;
        self.mark_dirty();
    }

    fn delete_left(&mut self) {
        if self.cursor_chars == 0 {
            return;
        }
        let end = char_to_byte_index(self.input.as_str(), self.cursor_chars);
        let start = char_to_byte_index(self.input.as_str(), self.cursor_chars - 1);
        self.input.replace_range(start..end, "");
        self.cursor_chars -= 1;
        self.mark_dirty();
    }

    fn delete_right(&mut self) {
        let total = self.input.chars().count();
        if self.cursor_chars >= total {
            return;
        }
        let start = char_to_byte_index(self.input.as_str(), self.cursor_chars);
        let end = char_to_byte_index(self.input.as_str(), self.cursor_chars + 1);
        self.input.replace_range(start..end, "");
        self.mark_dirty();
    }
}

#[derive(Debug, Clone)]
struct PaneBlock {
    title: String,
    body: Vec<StyledContent>,
}

#[derive(Debug, Clone, Copy)]
enum ScrollAnchor {
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum TuiPane {
    #[default]
    Transcript,
    TaskList,
    TaskDetail,
    Events,
}

impl TuiPane {
    fn from_session_focus(value: &str) -> Self {
        match value {
            "task_list" => Self::TaskList,
            "task_detail" => Self::TaskDetail,
            _ => Self::Transcript,
        }
    }

    fn step(self, direction: i8) -> Self {
        const ORDER: [TuiPane; 4] = [
            TuiPane::Transcript,
            TuiPane::TaskList,
            TuiPane::TaskDetail,
            TuiPane::Events,
        ];
        let index = ORDER.iter().position(|pane| pane == &self).unwrap_or(0);
        let next = if direction < 0 {
            index.checked_sub(1).unwrap_or(ORDER.len() - 1)
        } else if index + 1 < ORDER.len() {
            index + 1
        } else {
            0
        };
        ORDER[next]
    }

    fn label(self) -> &'static str {
        match self {
            Self::Transcript => "transcript",
            Self::TaskList => "task_list",
            Self::TaskDetail => "task_detail",
            Self::Events => "events",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum TuiOverlay {
    #[default]
    None,
    Help,
    Inspector,
    Permissions,
}

impl TuiOverlay {
    const fn is_open(self) -> bool {
        !matches!(self, Self::None)
    }

    const fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Help => "help",
            Self::Inspector => "inspector",
            Self::Permissions => "permissions",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum TuiEventsFilter {
    #[default]
    All,
    Task,
    Stream,
    Error,
}

impl TuiEventsFilter {
    fn step(self) -> Self {
        match self {
            Self::All => Self::Task,
            Self::Task => Self::Stream,
            Self::Stream => Self::Error,
            Self::Error => Self::All,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Task => "task",
            Self::Stream => "stream",
            Self::Error => "error",
        }
    }

    fn matches(self, line: &str) -> bool {
        match self {
            Self::All => true,
            Self::Task => line.starts_with("task "),
            Self::Stream => {
                line.starts_with("stream ")
                    || line.contains("stream=")
                    || line.contains("stream_events.")
            }
            Self::Error => {
                let normalized = line.to_ascii_lowercase();
                normalized.contains("error")
                    || normalized.contains("failed")
                    || normalized.contains("model_error")
            }
        }
    }
}

fn char_to_byte_index(value: &str, char_index: usize) -> usize {
    value
        .char_indices()
        .nth(char_index)
        .map(|(index, _)| index)
        .unwrap_or(value.len())
}

fn wrap_line(value: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    if value.is_empty() {
        return vec![String::new()];
    }
    let chars = value.chars().collect::<Vec<_>>();
    chars
        .chunks(width)
        .map(|chunk| chunk.iter().collect::<String>())
        .collect()
}

/// Wrap a `StyledContent` line to fit within `width` characters,
/// splitting at segment boundaries and within segments as needed.
fn wrap_styled_content(sc: &StyledContent, width: usize) -> Vec<StyledContent> {
    let width = width.max(1);
    match sc {
        StyledContent::Plain(s) => wrap_line(s, width)
            .into_iter()
            .map(StyledContent::Plain)
            .collect(),
        StyledContent::Styled(segs) => {
            let mut result: Vec<Vec<LineSegment>> = vec![Vec::new()];
            let mut col = 0usize;
            for seg in segs {
                let mut remaining: &str = &seg.text;
                while !remaining.is_empty() {
                    #[allow(dead_code)]
                    let avail = width.saturating_sub(col);
                    if avail == 0 {
                        result.push(Vec::new());
                        col = 0;
                        continue;
                    }
                    let take: String = remaining.chars().take(avail).collect();
                    let taken_chars = take.chars().count();
                    let taken_bytes: usize = remaining
                        .chars()
                        .take(taken_chars)
                        .map(|c| c.len_utf8())
                        .sum();
                    remaining = &remaining[taken_bytes..];
                    col += taken_chars;
                    result.last_mut().unwrap().push(LineSegment {
                        text: take,
                        color: seg.color,
                        bold: seg.bold,
                        italic: seg.italic,
                    });
                    if col >= width && !remaining.is_empty() {
                        result.push(Vec::new());
                        col = 0;
                    }
                }
            }
            result.into_iter().map(StyledContent::Styled).collect()
        }
    }
}

/// Classify a transcript/panel line and return its display color.
fn classify_line_color(line: &str) -> Color {
    let trimmed = line.trim();
    if trimmed.starts_with("[t") && trimmed.contains(":conversation] system:") {
        Color::DarkGrey
    } else if trimmed.starts_with("[t") && trimmed.contains(":conversation] user:") {
        Color::Green
    } else if trimmed.starts_with("[t") && trimmed.contains(":conversation] assistant:") {
        Color::Cyan
    } else if trimmed.starts_with("[t") && trimmed.contains(":tool_request]") {
        Color::Yellow
    } else if trimmed.starts_with("[t") && trimmed.contains(":tool_result]") {
        Color::DarkYellow
    } else if trimmed.starts_with("[t") && trimmed.contains(":tool_progress]") {
        Color::DarkGrey
    } else if trimmed.starts_with("[t") && trimmed.contains(":tool_message]") {
        Color::Magenta
    } else if trimmed.starts_with("stream start:") {
        Color::Blue
    } else if trimmed.starts_with("stream delta:") {
        Color::Cyan
    } else if trimmed.starts_with("stream complete:") {
        Color::Green
    } else if trimmed.contains("error") || trimmed.contains("failed") || trimmed.contains("denied")
    {
        Color::Red
    } else if trimmed.starts_with("status:") || trimmed.starts_with("nocode") {
        Color::Cyan
    } else {
        Color::White
    }
}

fn pad_line(value: &str, width: usize) -> String {
    let mut padded = value.chars().take(width).collect::<String>();
    let len = padded.chars().count();
    if len < width {
        padded.push_str(&" ".repeat(width - len));
    }
    padded
}

fn split_block(content: &str) -> PaneBlock {
    match content.split_once('\n') {
        Some((title, body)) => PaneBlock {
            title: title.to_string(),
            body: body
                .lines()
                .map(|l| StyledContent::Plain(l.to_string()))
                .collect(),
        },
        None => PaneBlock {
            title: content.to_string(),
            body: Vec::new(),
        },
    }
}

fn render_panel_title(title: &str, width: usize, active: bool) -> String {
    let fill = if active { '=' } else { '-' };
    let mut visible = title.chars().take(width).collect::<String>();
    let used = visible.chars().count();
    if used < width {
        visible.push_str(&fill.to_string().repeat(width - used));
    }
    visible
}

fn viewport_summary(
    total: usize,
    height: usize,
    anchor: ScrollAnchor,
    scroll: usize,
) -> (usize, usize, usize) {
    if total == 0 || height == 0 {
        return (0, 0, total);
    }
    let start = match anchor {
        ScrollAnchor::Top => scroll.min(total.saturating_sub(height)),
        ScrollAnchor::Bottom => total.saturating_sub(height).saturating_sub(scroll),
    };
    let end = (start + height).min(total);
    (start, end, total)
}

#[cfg(test)]
mod tests {
    use super::{
        ScrollAnchor, TuiEventsFilter, TuiOverlay, TuiPane, render_panel_title, viewport_summary,
    };

    #[test]
    fn tui_pane_rotation_wraps() {
        assert_eq!(TuiPane::Transcript.step(1), TuiPane::TaskList);
        assert_eq!(TuiPane::Events.step(1), TuiPane::Transcript);
        assert_eq!(TuiPane::Transcript.step(-1), TuiPane::Events);
    }

    #[test]
    fn viewport_summary_respects_anchor_and_scroll() {
        assert_eq!(viewport_summary(20, 5, ScrollAnchor::Top, 0), (0, 5, 20));
        assert_eq!(viewport_summary(20, 5, ScrollAnchor::Top, 99), (15, 20, 20));
        assert_eq!(
            viewport_summary(20, 5, ScrollAnchor::Bottom, 0),
            (15, 20, 20)
        );
        assert_eq!(
            viewport_summary(20, 5, ScrollAnchor::Bottom, 3),
            (12, 17, 20)
        );
    }

    #[test]
    fn overlay_state_tracks_visibility() {
        assert!(!TuiOverlay::None.is_open());
        assert!(TuiOverlay::Help.is_open());
        assert_eq!(TuiOverlay::Permissions.label(), "permissions");
    }

    #[test]
    fn events_filter_matches_task_lines_only_when_enabled() {
        assert!(TuiEventsFilter::All.matches("stream delta: hi"));
        assert!(TuiEventsFilter::Task.matches("task drive: a1 status=running"));
        assert!(!TuiEventsFilter::Task.matches("stream delta: hi"));
        assert!(TuiEventsFilter::Stream.matches("stream delta: hi"));
        assert!(TuiEventsFilter::Stream.matches("task drive: a1 activity=stream=delta:hi"));
        assert!(!TuiEventsFilter::Stream.matches("task drive: a1 status=running"));
        assert!(TuiEventsFilter::Error.matches("task drive error: a1 boom"));
        assert!(TuiEventsFilter::Error.matches("model_error.kind=transport"));
        assert!(!TuiEventsFilter::Error.matches("stream delta: hi"));
        assert_eq!(TuiEventsFilter::All.step(), TuiEventsFilter::Task);
        assert_eq!(TuiEventsFilter::Task.step(), TuiEventsFilter::Stream);
        assert_eq!(TuiEventsFilter::Stream.step(), TuiEventsFilter::Error);
        assert_eq!(TuiEventsFilter::Error.step(), TuiEventsFilter::All);
    }

    #[test]
    fn panel_title_fill_marks_active_panel() {
        assert_eq!(render_panel_title("* pane", 10, true), "* pane====");
        assert_eq!(render_panel_title("  pane", 10, false), "  pane----");
    }
}
