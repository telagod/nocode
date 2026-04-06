//! Ratatui-based TUI application core.

use crate::markdown_render::render_markdown_to_lines;
use crate::markdown_stream::MarkdownStreamState;
use crate::repl::ReplSession;
use crate::spinner::Spinner;
use crate::tui_widgets::{ChatMessage, ChatMessageKind, InputBox, OverlayBlock, StatusBar};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use nocode_core::QueryEngine;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::{
    Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
};
use std::io;
use std::sync::mpsc;
use std::time::Duration;

const LOG_LIMIT: usize = 256;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Overlay {
    #[default]
    None,
    Help,
    Permission,
}

impl Overlay {
    fn is_open(self) -> bool {
        !matches!(self, Self::None)
    }
}

pub(crate) struct TuiApp {
    chat_messages: Vec<ChatMessage>,
    input: String,
    cursor_pos: usize,
    chat_scroll: u16,
    overlay: Overlay,
    thinking_spinner: Option<Spinner>,
    md_stream: MarkdownStreamState,
    dirty: bool,
}

impl TuiApp {
    pub fn new() -> Self {
        let mut app = Self {
            chat_messages: Vec::new(),
            input: String::new(),
            cursor_pos: 0,
            chat_scroll: 0,
            overlay: Overlay::None,
            thinking_spinner: None,
            md_stream: MarkdownStreamState::new(),
            dirty: true,
        };
        app.push_system("nocode v0.1.6 ready. Type a message or /help. F1=help Ctrl-C=quit");
        app
    }

    // -- content push methods --

    pub fn push_system(&mut self, text: &str) {
        self.chat_messages
            .push(ChatMessage::plain(ChatMessageKind::System, text));
        self.trim_log();
        self.dirty = true;
    }

    pub fn push_user_message(&mut self, text: &str) {
        self.chat_messages
            .push(ChatMessage::plain(ChatMessageKind::User, text));
        self.trim_log();
        self.dirty = true;
    }

    pub fn push_assistant_markdown(&mut self, markdown: &str) {
        let lines = render_markdown_to_lines(markdown);
        self.chat_messages
            .push(ChatMessage::new(ChatMessageKind::Assistant, lines));
        self.trim_log();
        self.chat_scroll = 0; // auto-scroll to bottom
        self.dirty = true;
    }

    pub fn push_error(&mut self, message: &str) {
        self.chat_messages
            .push(ChatMessage::plain(ChatMessageKind::Error, message));
        self.trim_log();
        self.dirty = true;
    }

    pub fn push_tool_event(&mut self, line: &str) {
        self.chat_messages
            .push(ChatMessage::plain(ChatMessageKind::Tool, line));
        self.trim_log();
        self.dirty = true;
    }

    pub fn push_spinner_frame(&mut self, frame_text: &str) {
        // Replace last spinner message or add new one
        if let Some(last) = self.chat_messages.last_mut()
            && last.kind == ChatMessageKind::Spinner
        {
            last.lines = vec![{
                let mut rl = crate::markdown_render::RenderedLine::new();
                rl.push(crate::markdown_render::LineSegment::new(
                    frame_text,
                    crossterm::style::Color::DarkGrey,
                ));
                rl
            }];
            self.dirty = true;
            return;
        }
        let mut rl = crate::markdown_render::RenderedLine::new();
        rl.push(crate::markdown_render::LineSegment::new(
            frame_text,
            crossterm::style::Color::DarkGrey,
        ));
        self.chat_messages
            .push(ChatMessage::new(ChatMessageKind::Spinner, vec![rl]));
        self.dirty = true;
    }

    fn trim_log(&mut self) {
        while self.chat_messages.len() > LOG_LIMIT {
            self.chat_messages.remove(0);
        }
    }

    // -- drawing --

    pub fn draw(&mut self, frame: &mut Frame, session: &ReplSession) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // status bar
                Constraint::Min(4),    // chat area
                Constraint::Length(3), // input box
            ])
            .split(frame.area());

        // 1. Status bar
        let snapshot = session.render_tui_snapshot();
        let status = StatusBar::new(&snapshot.hud_line);
        frame.render_widget(status, chunks[0]);

        // 2. Chat area
        self.draw_chat_area(frame, chunks[1]);

        // 3. Input box
        let prompt_text = session.prompt_text();
        let input_widget = InputBox::new(&self.input, self.cursor_pos, &prompt_text);
        frame.render_widget(input_widget, chunks[2]);

        // Cursor position
        let prompt_len = prompt_text.len() as u16;
        let cursor_x = chunks[2].x + prompt_len + self.cursor_pos as u16;
        let cursor_y = chunks[2].y + 1;
        frame.set_cursor_position((cursor_x, cursor_y));

        // 4. Overlay (on top)
        if self.overlay.is_open() {
            self.draw_overlay(frame, session, frame.area());
        }

        self.dirty = false;
    }

    fn draw_chat_area(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default().borders(Borders::NONE);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Collect all lines from all messages
        let mut all_lines: Vec<ratatui::text::Line<'_>> = Vec::new();
        for msg in &self.chat_messages {
            all_lines.extend(msg.to_ratatui_lines());
            all_lines.push(ratatui::text::Line::from("")); // spacing
        }

        let total_lines = all_lines.len() as u16;
        let visible = inner.height;
        let max_scroll = total_lines.saturating_sub(visible);
        let scroll = self.chat_scroll.min(max_scroll);

        let paragraph = Paragraph::new(all_lines)
            .wrap(Wrap { trim: false })
            .scroll((max_scroll.saturating_sub(scroll), 0)); // bottom-anchored

        frame.render_widget(paragraph, inner);

        // Scrollbar
        if total_lines > visible {
            let mut scrollbar_state = ScrollbarState::new(max_scroll as usize)
                .position((max_scroll.saturating_sub(scroll)) as usize);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .style(Style::default().fg(Color::DarkGray)),
                inner,
                &mut scrollbar_state,
            );
        }
    }

    fn draw_overlay(&self, frame: &mut Frame, session: &ReplSession, area: Rect) {
        match self.overlay {
            Overlay::Help => {
                let body = "Keyboard shortcuts:\n\n\
                    F1 / ?      Help overlay\n\
                    F3          Permission overlay\n\
                    Ctrl-C      Quit\n\
                    Up/Down     Scroll chat\n\
                    PgUp/PgDn   Scroll page\n\
                    Ctrl-L      Clear chat\n\
                    Ctrl-U      Clear input\n\
                    Ctrl-P/N    History prev/next\n\
                    Enter       Send message\n\n\
                    Slash commands: /help /status /tasks /commit /diff";
                frame.render_widget(OverlayBlock::new("Help", body), area);
            }
            Overlay::Permission => {
                let body = session.render_tui_permission_overlay();
                frame.render_widget(OverlayBlock::new("Permissions", &body), area);
            }
            Overlay::None => {}
        }
    }

    // -- key handling --

    pub fn handle_key(&mut self, key: KeyEvent, session: &mut ReplSession) -> io::Result<bool> {
        // Ctrl-C = quit
        if matches!(
            (key.code, key.modifiers),
            (KeyCode::Char('c'), KeyModifiers::CONTROL)
        ) {
            return Ok(false);
        }

        // Overlay toggles
        if self.handle_overlay_key(key, session) {
            return Ok(true);
        }

        // If overlay is open, consume keys
        if self.overlay.is_open() {
            if key.code == KeyCode::Esc {
                self.overlay = Overlay::None;
                self.dirty = true;
            }
            return Ok(true);
        }

        match (key.code, key.modifiers) {
            // Scroll
            (KeyCode::Up, KeyModifiers::NONE) => {
                self.chat_scroll = self.chat_scroll.saturating_add(1);
                self.dirty = true;
            }
            (KeyCode::Down, KeyModifiers::NONE) => {
                self.chat_scroll = self.chat_scroll.saturating_sub(1);
                self.dirty = true;
            }
            (KeyCode::PageUp, _) => {
                self.chat_scroll = self.chat_scroll.saturating_add(10);
                self.dirty = true;
            }
            (KeyCode::PageDown, _) => {
                self.chat_scroll = self.chat_scroll.saturating_sub(10);
                self.dirty = true;
            }
            // Clear
            (KeyCode::Char('l'), KeyModifiers::CONTROL) => {
                self.chat_messages.clear();
                self.dirty = true;
            }
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                self.input.clear();
                self.cursor_pos = 0;
                self.dirty = true;
            }
            // History — simplified: Ctrl-P/N not yet wired
            (KeyCode::Char('p'), KeyModifiers::CONTROL) => {}
            (KeyCode::Char('n'), KeyModifiers::CONTROL) => {}
            // Submit
            (KeyCode::Enter, _) => {
                if !self.input.is_empty() {
                    let text = std::mem::take(&mut self.input);
                    self.cursor_pos = 0;
                    self.push_user_message(&text);
                    session.submit_prompt(&text);
                    self.chat_scroll = 0;
                }
            }
            // Cursor movement
            (KeyCode::Left, _) => {
                self.cursor_pos = self.cursor_pos.saturating_sub(1);
                self.dirty = true;
            }
            (KeyCode::Right, _) => {
                if self.cursor_pos < self.input.len() {
                    self.cursor_pos += 1;
                    self.dirty = true;
                }
            }
            (KeyCode::Home, _) | (KeyCode::Char('a'), KeyModifiers::CONTROL) => {
                self.cursor_pos = 0;
                self.dirty = true;
            }
            (KeyCode::End, _) | (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
                self.cursor_pos = self.input.len();
                self.dirty = true;
            }
            // Backspace / Delete
            (KeyCode::Backspace, _) => {
                if self.cursor_pos > 0 {
                    self.input.remove(self.cursor_pos - 1);
                    self.cursor_pos -= 1;
                    self.dirty = true;
                }
            }
            (KeyCode::Delete, _) => {
                if self.cursor_pos < self.input.len() {
                    self.input.remove(self.cursor_pos);
                    self.dirty = true;
                }
            }
            // Character input
            (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                self.input.insert(self.cursor_pos, c);
                self.cursor_pos += 1;
                self.dirty = true;
            }
            _ => {}
        }

        Ok(true)
    }

    fn handle_overlay_key(&mut self, key: KeyEvent, session: &mut ReplSession) -> bool {
        match key.code {
            KeyCode::F(1) => {
                self.overlay = if self.overlay == Overlay::Help {
                    Overlay::None
                } else {
                    Overlay::Help
                };
                self.dirty = true;
                true
            }
            KeyCode::F(3) => {
                self.overlay = if self.overlay == Overlay::Permission {
                    Overlay::None
                } else {
                    Overlay::Permission
                };
                self.dirty = true;
                true
            }
            // Permission overlay keys
            KeyCode::Char('a' | 'A') if self.overlay == Overlay::Permission => {
                session.resolve_permission(true);
                if !session.has_pending_permissions() {
                    self.overlay = Overlay::None;
                }
                self.dirty = true;
                true
            }
            KeyCode::Char('d' | 'D') if self.overlay == Overlay::Permission => {
                session.resolve_permission(false);
                if !session.has_pending_permissions() {
                    self.overlay = Overlay::None;
                }
                self.dirty = true;
                true
            }
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// Event loop
// ---------------------------------------------------------------------------

pub(crate) fn run_app_loop(
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<io::Stdout>>,
    session: &mut ReplSession,
    engine_slot: &mut Option<QueryEngine>,
) -> io::Result<()> {
    let mut app = TuiApp::new();

    loop {
        // 1. Check pending intent → launch async submission
        if let Some(intent) = session.take_pending_intent() {
            if let Some(eng) = engine_slot.take() {
                launch_async_submission(session, eng, intent);
                app.thinking_spinner = Some(Spinner::new("\u{1F980} Thinking..."));
            } else {
                app.push_system("engine busy — waiting for current submission");
            }
        }

        // 2. Tick spinner
        if let Some(spinner) = app.thinking_spinner.as_mut()
            && !spinner.is_done()
        {
            let frame = spinner.tick();
            app.push_spinner_frame(frame.display.as_str());
        }

        // 3. Poll stream events
        let (stream_lines, returned_engine) = session.poll_pending_stream();
        for line in &stream_lines {
            if line.contains("[CALL]") || line.contains("[DONE]") {
                app.push_tool_event(line);
            } else if line.starts_with("stream delta:") {
                let text = line.strip_prefix("stream delta: ").unwrap_or(line);
                if let Some(rendered) = app.md_stream.push(text) {
                    app.push_assistant_markdown(&rendered);
                }
            } else if line.starts_with("stream error:") {
                let msg = line.strip_prefix("stream error: ").unwrap_or(line);
                app.push_error(msg);
            } else if line.starts_with("stream complete:") {
                // Flush remaining markdown buffer
                if let Some(rendered) = app.md_stream.flush() {
                    app.push_assistant_markdown(&rendered);
                }
            } else {
                app.push_system(line);
            }
        }
        if let Some(eng) = returned_engine {
            if let Some(rendered) = app.md_stream.flush() {
                app.push_assistant_markdown(&rendered);
            }
            if let Some(spinner) = app.thinking_spinner.as_mut() {
                spinner.finish("Done");
            }
            app.thinking_spinner = None;
            *engine_slot = Some(eng);
        }

        // 4. Poll permissions
        if session.poll_permissions() {
            app.overlay = Overlay::Permission;
            app.dirty = true;
        }

        // 5. Render
        terminal.draw(|frame| app.draw(frame, session))?;

        // 6. Poll events (50ms timeout for responsive spinner)
        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => {
                    if engine_slot.is_some() {
                        if !app.handle_key(key, session)? {
                            break;
                        }
                    } else {
                        // Engine busy — only scroll/overlay/quit
                        match (key.code, key.modifiers) {
                            (KeyCode::Char('c'), KeyModifiers::CONTROL) => break,
                            (KeyCode::Up, _) => {
                                app.chat_scroll = app.chat_scroll.saturating_add(1);
                                app.dirty = true;
                            }
                            (KeyCode::Down, _) => {
                                app.chat_scroll = app.chat_scroll.saturating_sub(1);
                                app.dirty = true;
                            }
                            _ => {
                                let _ = app.handle_overlay_key(key, session);
                            }
                        }
                    }
                }
                Event::Resize(_, _) => app.dirty = true,
                _ => {}
            }
        }
    }

    Ok(())
}

fn launch_async_submission(
    session: &mut ReplSession,
    engine: QueryEngine,
    intent: crate::repl::ReplIntent,
) {
    use crate::repl::PendingSubmission;
    use nocode_core::ChannelModelStreamSink;

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
}
