//! Ratatui-based TUI application core.

use crate::markdown_render::render_markdown_to_lines;
use crate::markdown_stream::MarkdownStreamState;
use crate::repl::ReplSession;
use crate::spinner::Spinner;
use crate::tui_widgets::{
    ChatMessage, ChatMessageKind, HintsBar, InputBox, OverlayBlock, StatusBar, WelcomeBanner,
    WelcomeBannerInfo,
};

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
    #[allow(dead_code)]
    md_stream: MarkdownStreamState,
    dirty: bool,
    height_cache: Vec<u16>,
    height_cache_width: u16,
    sticky_scroll: bool,
    unseen_count: usize,
    show_banner: bool,
    banner_info: WelcomeBannerInfo,
    /// Accumulated assistant text during streaming — rendered incrementally.
    streaming_text: String,
}

impl TuiApp {
    pub fn new() -> Self {
        Self {
            chat_messages: Vec::new(),
            input: String::new(),
            cursor_pos: 0,
            chat_scroll: 0,
            overlay: Overlay::None,
            thinking_spinner: None,
            md_stream: MarkdownStreamState::new(),
            dirty: true,
            height_cache: Vec::new(),
            height_cache_width: 0,
            sticky_scroll: true,
            unseen_count: 0,
            show_banner: true,
            banner_info: WelcomeBannerInfo::default(),
            streaming_text: String::new(),
        }
    }

    // -- content push methods --

    fn on_message_added(&mut self) {
        self.trim_log();
        self.invalidate_height_cache();
        if self.sticky_scroll {
            self.chat_scroll = 0;
        } else {
            self.unseen_count += 1;
        }
        self.dirty = true;
    }

    pub fn push_system(&mut self, text: &str) {
        self.chat_messages
            .push(ChatMessage::plain(ChatMessageKind::System, text));
        self.on_message_added();
    }

    pub fn push_user_message(&mut self, text: &str) {
        self.chat_messages
            .push(ChatMessage::plain(ChatMessageKind::User, text));
        self.on_message_added();
    }

    #[allow(dead_code)]
    pub fn push_assistant_markdown(&mut self, markdown: &str) {
        let lines = render_markdown_to_lines(markdown);
        self.chat_messages
            .push(ChatMessage::new(ChatMessageKind::Assistant, lines));
        self.on_message_added();
    }

    /// Update the current streaming assistant message, or create one if none exists.
    /// This gives the real-time typing effect — one message grows incrementally.
    pub fn update_streaming_assistant(&mut self, accumulated_markdown: &str) {
        let lines = render_markdown_to_lines(accumulated_markdown);
        // Find the last assistant message and update it in-place
        if let Some(last) = self.chat_messages.last_mut()
            && last.kind == ChatMessageKind::Assistant
        {
            last.lines = lines;
            self.invalidate_height_cache();
            self.dirty = true;
        } else {
            // First delta — create the message
            self.chat_messages
                .push(ChatMessage::new(ChatMessageKind::Assistant, lines));
            self.on_message_added();
        }
    }

    pub fn push_error(&mut self, message: &str) {
        self.chat_messages
            .push(ChatMessage::plain(ChatMessageKind::Error, message));
        self.on_message_added();
    }

    #[allow(dead_code)]
    pub fn push_tool_event(&mut self, line: &str) {
        self.chat_messages
            .push(ChatMessage::plain(ChatMessageKind::Tool, line));
        self.on_message_added();
    }

    /// Structured tool call start — shows tool name + args with collapse arrow.
    pub fn push_tool_start(&mut self, tool_name: &str, args_summary: &str) {
        use crate::tui_widgets::ToolCallInfo;
        let info = ToolCallInfo::new(tool_name, args_summary);
        self.chat_messages
            .push(ChatMessage::tool_call(info, Vec::new()));
        self.on_message_added();
    }

    /// Structured tool call done — update the last matching tool message.
    pub fn push_tool_done(&mut self, tool_name: &str) {
        self.push_tool_done_with_args(tool_name, "");
    }

    /// Structured tool call done with arguments summary.
    pub fn push_tool_done_with_args(&mut self, tool_name: &str, args: &str) {
        for msg in self.chat_messages.iter_mut().rev() {
            if let Some(info) = &mut msg.tool_info
                && info.tool_name == tool_name
            {
                if !args.is_empty() {
                    info.arguments_summary = args.to_owned();
                }
                info.result_preview = Some(String::from("done"));
                break;
            }
        }
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
            if !self.height_cache.is_empty() {
                self.height_cache.remove(0);
            }
        }
    }

    fn invalidate_height_cache(&mut self) {
        self.height_cache.clear();
        self.height_cache_width = 0;
    }

    /// Compute height for a single message at the given width.
    /// Each message = badge line (1) + content lines + 1 spacing line.
    fn message_height(msg: &ChatMessage, _width: u16) -> u16 {
        let badge = if matches!(msg.kind, ChatMessageKind::Spinner) {
            0
        } else {
            1
        };
        let content = msg.lines.len() as u16;
        badge + content + 1 // +1 for spacing between messages
    }

    /// Ensure height cache is populated for all messages at the given width.
    fn ensure_height_cache(&mut self, width: u16) {
        if self.height_cache_width == width && self.height_cache.len() == self.chat_messages.len() {
            return;
        }
        if self.height_cache_width != width {
            self.height_cache.clear();
        }
        self.height_cache_width = width;
        // Fill missing entries (append-only fast path).
        while self.height_cache.len() < self.chat_messages.len() {
            let idx = self.height_cache.len();
            let h = Self::message_height(&self.chat_messages[idx], width);
            self.height_cache.push(h);
        }
    }

    /// Total height of all messages in terminal rows.
    fn total_content_height(&self) -> u16 {
        self.height_cache.iter().copied().sum()
    }

    // -- drawing --

    pub fn draw(&mut self, frame: &mut Frame, session: &ReplSession) {
        let is_busy = self.thinking_spinner.is_some();
        let hints_height = if is_busy { 0 } else { 1 };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(4),                    // chat area / banner (top, fills)
                Constraint::Length(1),                  // status line
                Constraint::Length(hints_height),       // keyboard hints (hidden when busy)
                Constraint::Length(1),                  // input line
            ])
            .split(frame.area());

        // 1. Main area — banner or chat
        if self.show_banner && self.chat_messages.is_empty() {
            let banner = WelcomeBanner::new(&self.banner_info);
            frame.render_widget(banner, chunks[0]);
        } else {
            self.draw_chat_area(frame, chunks[0]);
        }

        // 2. Status line (above input)
        let snapshot = session.render_tui_snapshot();
        let status = StatusBar::new(&snapshot.hud_line);
        frame.render_widget(status, chunks[1]);

        // 3. Keyboard hints (only when idle)
        if !is_busy {
            let hints = HintsBar;
            frame.render_widget(hints, chunks[2]);
        }

        // 4. Input line (bottom)
        let input_widget = InputBox::new(&self.input, self.cursor_pos);
        frame.render_widget(input_widget, chunks[3]);

        // Cursor position: "> " prefix = 2 chars, then display width of text before cursor
        let display_width = display_width_of(&self.input[..self.cursor_pos]);
        let cursor_x = chunks[3].x + 2 + display_width as u16;
        let cursor_y = chunks[3].y;
        frame.set_cursor_position((cursor_x, cursor_y));

        // 5. Overlay (on top)
        if self.overlay.is_open() {
            self.draw_overlay(frame, session, frame.area());
        }

        self.dirty = false;
    }

    fn draw_chat_area(&mut self, frame: &mut Frame, area: Rect) {
        let block = Block::default().borders(Borders::NONE);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        self.ensure_height_cache(inner.width);

        let total_h = self.total_content_height();
        let visible = inner.height;
        let max_scroll = total_h.saturating_sub(visible);
        let scroll = self.chat_scroll.min(max_scroll);

        // Bottom-anchored: effective scroll from top = max_scroll - scroll
        let scroll_from_top = max_scroll.saturating_sub(scroll);

        // Viewport culling: find which messages are visible.
        let mut accumulated: u16 = 0;
        let mut first_visible_idx: Option<usize> = None;
        let mut first_visible_skip: u16 = 0;
        let mut visible_lines: Vec<ratatui::text::Line<'_>> = Vec::new();
        let mut rows_collected: u16 = 0;

        for (i, msg) in self.chat_messages.iter().enumerate() {
            let h = self.height_cache.get(i).copied().unwrap_or(1);
            let msg_end = accumulated + h;

            if msg_end <= scroll_from_top {
                accumulated = msg_end;
                continue;
            }

            if first_visible_idx.is_none() {
                first_visible_idx = Some(i);
                first_visible_skip = scroll_from_top.saturating_sub(accumulated);
            }

            // Collect lines from this message
            let msg_lines = msg.to_ratatui_lines();
            let mut all_msg_lines: Vec<ratatui::text::Line<'_>> = msg_lines;
            all_msg_lines.push(ratatui::text::Line::from("")); // spacing

            for line in all_msg_lines {
                if rows_collected >= visible {
                    break;
                }
                visible_lines.push(line);
                rows_collected += 1;
            }

            accumulated = msg_end;

            if rows_collected >= visible {
                break;
            }
        }

        // Skip partial top lines
        let skip = first_visible_skip as usize;
        if skip > 0 && skip < visible_lines.len() {
            visible_lines = visible_lines[skip..].to_vec();
        }

        let paragraph = Paragraph::new(visible_lines).wrap(Wrap { trim: false });
        frame.render_widget(paragraph, inner);

        // Scrollbar
        if total_h > visible {
            let mut scrollbar_state =
                ScrollbarState::new(max_scroll as usize).position(scroll_from_top as usize);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .style(Style::default().fg(Color::DarkGray)),
                inner,
                &mut scrollbar_state,
            );
        }

        // Unseen message indicator
        if self.unseen_count > 0 && !self.sticky_scroll {
            let indicator = format!(" {} unseen ", self.unseen_count);
            let indicator_width = indicator.len() as u16;
            let x = inner.right().saturating_sub(indicator_width + 1);
            let y = inner.bottom().saturating_sub(1);
            let indicator_area = Rect::new(x, y, indicator_width, 1);
            let badge = Paragraph::new(indicator)
                .style(Style::default().fg(Color::Black).bg(Color::Yellow));
            frame.render_widget(badge, indicator_area);
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
            // Scroll — up releases sticky, down at bottom re-engages
            (KeyCode::Up, KeyModifiers::NONE) => {
                self.chat_scroll = self.chat_scroll.saturating_add(1);
                self.sticky_scroll = false;
                self.dirty = true;
            }
            (KeyCode::Down, KeyModifiers::NONE) => {
                if self.chat_scroll > 0 {
                    self.chat_scroll = self.chat_scroll.saturating_sub(1);
                }
                if self.chat_scroll == 0 {
                    self.sticky_scroll = true;
                    self.unseen_count = 0;
                }
                self.dirty = true;
            }
            (KeyCode::PageUp, _) => {
                self.chat_scroll = self.chat_scroll.saturating_add(10);
                self.sticky_scroll = false;
                self.dirty = true;
            }
            (KeyCode::PageDown, _) => {
                self.chat_scroll = self.chat_scroll.saturating_sub(10);
                if self.chat_scroll == 0 {
                    self.sticky_scroll = true;
                    self.unseen_count = 0;
                }
                self.dirty = true;
            }
            // Clear
            (KeyCode::Char('l'), KeyModifiers::CONTROL) => {
                self.chat_messages.clear();
                self.invalidate_height_cache();
                self.sticky_scroll = true;
                self.unseen_count = 0;
                self.chat_scroll = 0;
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
                    self.show_banner = false;
                    let text = std::mem::take(&mut self.input);
                    self.cursor_pos = 0;
                    self.push_user_message(&text);
                    session.submit_prompt(&text);
                    self.chat_scroll = 0;
                    self.sticky_scroll = true;
                    self.unseen_count = 0;
                }
            }
            // Cursor movement (char-boundary aware for multibyte UTF-8)
            (KeyCode::Left, _) => {
                if self.cursor_pos > 0 {
                    self.cursor_pos = prev_char_boundary(&self.input, self.cursor_pos);
                }
                self.dirty = true;
            }
            (KeyCode::Right, _) => {
                if self.cursor_pos < self.input.len() {
                    self.cursor_pos = next_char_boundary(&self.input, self.cursor_pos);
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
            // Backspace / Delete (char-boundary aware)
            (KeyCode::Backspace, _) => {
                if self.cursor_pos > 0 {
                    let prev = prev_char_boundary(&self.input, self.cursor_pos);
                    self.input.drain(prev..self.cursor_pos);
                    self.cursor_pos = prev;
                    self.dirty = true;
                }
            }
            (KeyCode::Delete, _) => {
                if self.cursor_pos < self.input.len() {
                    let next = next_char_boundary(&self.input, self.cursor_pos);
                    self.input.drain(self.cursor_pos..next);
                    self.dirty = true;
                }
            }
            // Character input (insert at char boundary, advance by char's byte len)
            (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                self.input.insert(self.cursor_pos, c);
                self.cursor_pos += c.len_utf8();
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
                app.thinking_spinner = Some(Spinner::new("Thinking..."));
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
            if line.starts_with("tool start: ") {
                let tool_name = line.strip_prefix("tool start: ").unwrap_or("tool");
                app.push_tool_start(tool_name, "");
            } else if line.starts_with("tool done: ") {
                let rest = line.strip_prefix("tool done: ").unwrap_or("");
                let (tool_name, args) = rest.split_once(' ').unwrap_or((rest, ""));
                // Update existing tool start message with args, or create new
                app.push_tool_done_with_args(tool_name, args);
            } else if line.contains("[CALL]") {
                let tool_name = extract_between(line, "[CALL] ", " ")
                    .or_else(|| extract_after(line, "[CALL] "))
                    .unwrap_or("tool");
                let args = extract_after(line, &format!("{tool_name} ")).unwrap_or("");
                app.push_tool_start(tool_name, args);
            } else if line.contains("[DONE]") {
                let tool_name = extract_between(line, "[DONE] ", " ")
                    .or_else(|| extract_after(line, "[DONE] "))
                    .unwrap_or("tool");
                app.push_tool_done(tool_name);
            } else if line.starts_with("stream start:") {
                // Internal event — don't display in chat, just dismiss banner
                app.show_banner = false;
            } else if line.starts_with("stream delta:") {
                let text = line.strip_prefix("stream delta: ").unwrap_or(line);
                // Remove spinner on first delta
                if app.streaming_text.is_empty() {
                    app.chat_messages
                        .retain(|m| m.kind != ChatMessageKind::Spinner);
                    app.thinking_spinner = None;
                }
                app.streaming_text.push_str(text);
                app.update_streaming_assistant(&app.streaming_text.clone());
            } else if line.starts_with("stream error:") {
                let msg = line.strip_prefix("stream error: ").unwrap_or(line);
                app.push_error(msg);
            } else if line.starts_with("stream complete:") {
                // Finalize: if we have accumulated text, it's already rendered.
                // Reset streaming state for next turn.
                app.streaming_text.clear();
            } else {
                // Filter out system prompt / bootstrap noise — only show short, meaningful lines
                let dominated_by_noise = line.contains("] system:")
                    || line.contains("system prompt")
                    || line.contains("You are nocode")
                    || line.contains("OVERRIDE")
                    || line.contains("Available tools")
                    || line.contains("Codebase and user instructions")
                    || line.len() > 300;
                if !dominated_by_noise {
                    app.push_system(line);
                }
            }
        }
        if let Some(eng) = returned_engine {
            // Finalize any remaining streaming text
            if !app.streaming_text.is_empty() {
                let final_text = app.streaming_text.clone();
                app.update_streaming_assistant(&final_text);
                app.streaming_text.clear();
            }
            // Remove spinner messages from chat history
            app.chat_messages
                .retain(|m| m.kind != ChatMessageKind::Spinner);
            app.invalidate_height_cache();
            app.thinking_spinner = None;
            app.dirty = true;
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
                                app.sticky_scroll = false;
                                app.dirty = true;
                            }
                            (KeyCode::Down, _) => {
                                if app.chat_scroll > 0 {
                                    app.chat_scroll = app.chat_scroll.saturating_sub(1);
                                }
                                if app.chat_scroll == 0 {
                                    app.sticky_scroll = true;
                                    app.unseen_count = 0;
                                }
                                app.dirty = true;
                            }
                            _ => {
                                let _ = app.handle_overlay_key(key, session);
                            }
                        }
                    }
                }
                Event::Resize(_, _) => {
                    app.invalidate_height_cache();
                    app.dirty = true;
                }
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

// ---------------------------------------------------------------------------
// UTF-8 char boundary helpers
// ---------------------------------------------------------------------------

/// Move backward to the previous char boundary in `s` from byte position `pos`.
fn prev_char_boundary(s: &str, pos: usize) -> usize {
    let mut p = pos.saturating_sub(1);
    while p > 0 && !s.is_char_boundary(p) {
        p -= 1;
    }
    p
}

/// Move forward to the next char boundary in `s` from byte position `pos`.
fn next_char_boundary(s: &str, pos: usize) -> usize {
    let mut p = pos + 1;
    while p < s.len() && !s.is_char_boundary(p) {
        p += 1;
    }
    p.min(s.len())
}

/// Compute display width of a string (CJK chars = 2 columns, ASCII = 1).
fn display_width_of(s: &str) -> usize {
    use unicode_width::UnicodeWidthChar;
    s.chars().map(|c| c.width().unwrap_or(0)).sum()
}

// ---------------------------------------------------------------------------
// String extraction helpers for parsing tool call lines
// ---------------------------------------------------------------------------

/// Extract text between `start` and `end` markers.
fn extract_between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let after = s.find(start).map(|i| &s[i + start.len()..])?;
    after.find(end).map(|i| &after[..i])
}

/// Extract everything after `prefix`.
fn extract_after<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    s.find(prefix).map(|i| &s[i + prefix.len()..])
}
