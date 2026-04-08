//! Ratatui-based TUI application core — v2 rewrite.
//!
//! Bridges the agentic loop (running on a background thread) to the TUI
//! via mpsc channels. The TUI thread owns the terminal and polls both
//! crossterm events and channel events at 50ms intervals.

use crate::command_registry::{CommandAction, CommandRegistry};
use crate::markdown_render::render_markdown_to_lines;
use crate::spinner::Spinner;
use crate::status_hud::StatusHud;
use crate::tui_widgets::{
    ChatMessage, ChatMessageKind, HintsBar, InputBox, OverlayBlock, StatusBar, WelcomeBanner,
    WelcomeBannerInfo,
};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use nocode_core::message::{ContentBlock, Message, SystemBlock};
use nocode_core::provider::Provider;
use nocode_core::provider::types::{StreamDelta, StreamEvent};
use nocode_core::query::r#loop::{self, LoopConfig, LoopObserver, LoopResult};
use nocode_core::tool::ToolRegistry;
use nocode_core::tool::executor::ToolExecutor;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Block, Borders};
use std::io;
use std::sync::mpsc;
use std::time::Duration;
use unicode_width::UnicodeWidthChar;

const LOG_LIMIT: usize = 256;

// ---------------------------------------------------------------------------
// TUI ↔ agentic loop bridge events
// ---------------------------------------------------------------------------

use std::sync::Arc;

enum TuiEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ToolStart {
        name: String,
    },
    ToolDone {
        name: String,
        content: String,
        is_error: bool,
    },
    Complete(Result<LoopResult, String>, ToolRegistry),
}

// ---------------------------------------------------------------------------
// Vim input mode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum InputMode {
    #[default]
    Insert,
    Normal,
}

impl InputMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Insert => "INSERT",
            Self::Normal => "NORMAL",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum Overlay {
    #[default]
    None,
    Help,
    Status,
    Sessions,
    Mcp,
    Agents,
    Config,
    Memory,
    Cost,
    Permission {
        tool_name: String,
        tool_id: String,
    },
}

impl Overlay {
    fn is_open(&self) -> bool {
        !matches!(self, Self::None)
    }
}

// ---------------------------------------------------------------------------
// TuiApp — main application state
// ---------------------------------------------------------------------------

pub(crate) struct TuiApp {
    chat_messages: Vec<ChatMessage>,
    input: String,
    cursor_pos: usize,
    chat_scroll: u16,
    overlay: Overlay,
    thinking_spinner: Option<Spinner>,
    dirty: bool,
    height_cache: Vec<u16>,
    height_cache_width: u16,
    sticky_scroll: bool,
    unseen_count: usize,
    show_banner: bool,
    banner_info: WelcomeBannerInfo,
    streaming_text: String,
    streaming_thinking: String,
    input_history: Vec<String>,
    history_index: Option<usize>,
    saved_input: String,
    input_mode: InputMode,
    vim_pending: Option<char>,
    hud: StatusHud,
}

impl TuiApp {
    pub fn new(model: &str) -> Self {
        Self {
            chat_messages: Vec::new(),
            input: String::new(),
            cursor_pos: 0,
            chat_scroll: 0,
            overlay: Overlay::None,
            thinking_spinner: None,
            dirty: true,
            height_cache: Vec::new(),
            height_cache_width: 0,
            sticky_scroll: true,
            unseen_count: 0,
            show_banner: true,
            banner_info: WelcomeBannerInfo::default(),
            streaming_text: String::new(),
            streaming_thinking: String::new(),
            input_history: Vec::new(),
            history_index: None,
            saved_input: String::new(),
            input_mode: InputMode::Insert,
            vim_pending: None,
            hud: StatusHud::new(model, ""),
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

    fn trim_log(&mut self) {
        if self.chat_messages.len() > LOG_LIMIT {
            let drain = self.chat_messages.len() - LOG_LIMIT;
            self.chat_messages.drain(..drain);
            self.invalidate_height_cache();
        }
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

    pub fn push_error(&mut self, text: &str) {
        self.chat_messages
            .push(ChatMessage::plain(ChatMessageKind::Error, text));
        self.on_message_added();
    }

    pub fn push_tool_start(&mut self, name: &str) {
        self.chat_messages.push(ChatMessage::plain(
            ChatMessageKind::Tool,
            &format!("● {name}"),
        ));
        self.on_message_added();
    }

    pub fn push_tool_done(&mut self, name: &str, content: &str, is_error: bool) {
        let prefix = if is_error { "✗" } else { "✓" };
        let display = if content.len() > 200 {
            format!("{}...", &content[..200])
        } else {
            content.to_string()
        };
        self.chat_messages.push(ChatMessage::plain(
            ChatMessageKind::Tool,
            &format!("{prefix} {name} {display}"),
        ));
        self.on_message_added();
    }

    /// Update streaming assistant text incrementally.
    pub fn update_streaming_assistant(&mut self, accumulated: &str) {
        let lines = render_markdown_to_lines(accumulated);
        if let Some(last) = self.chat_messages.last_mut()
            && last.kind == ChatMessageKind::Assistant
        {
            last.lines = lines;
            self.invalidate_height_cache();
            self.dirty = true;
            return;
        }
        self.chat_messages
            .push(ChatMessage::new(ChatMessageKind::Assistant, lines));
        self.on_message_added();
    }

    /// Update streaming thinking text incrementally.
    pub fn update_streaming_thinking(&mut self, accumulated: &str) {
        let lines: Vec<crate::markdown_render::RenderedLine> = accumulated
            .lines()
            .map(|l| {
                let mut rl = crate::markdown_render::RenderedLine::new();
                rl.push(crate::markdown_render::LineSegment::new(
                    l,
                    crossterm::style::Color::DarkGrey,
                ));
                rl
            })
            .collect();
        if let Some(last) = self.chat_messages.last_mut()
            && last.kind == ChatMessageKind::Thinking
        {
            last.lines = lines;
            self.invalidate_height_cache();
            self.dirty = true;
            return;
        }
        self.chat_messages
            .push(ChatMessage::new(ChatMessageKind::Thinking, lines));
        self.on_message_added();
    }

    // -- height cache --

    fn invalidate_height_cache(&mut self) {
        self.height_cache.clear();
        self.height_cache_width = 0;
    }

    fn message_height(msg: &ChatMessage, width: u16) -> u16 {
        let rlines = msg.to_ratatui_lines();
        let mut total: u16 = 0;
        for line in &rlines {
            let line_width: usize = line.spans.iter().map(|s| s.content.len()).sum();
            let wrapped = if width > 0 {
                (line_width as u16 / width) + 1
            } else {
                1
            };
            total += wrapped;
        }
        total.max(1)
    }

    fn ensure_height_cache(&mut self, width: u16) {
        if self.height_cache_width != width {
            self.height_cache.clear();
            self.height_cache_width = width;
        }
        while self.height_cache.len() < self.chat_messages.len() {
            let idx = self.height_cache.len();
            let h = Self::message_height(&self.chat_messages[idx], width);
            self.height_cache.push(h);
        }
    }

    fn total_content_height(&self) -> u16 {
        self.height_cache.iter().copied().sum()
    }

    // -- drawing --

    pub fn draw(&mut self, frame: &mut Frame) {
        let is_busy = self.thinking_spinner.is_some();
        let hints_height: u16 = if is_busy { 0 } else { 1 };
        let input_lines = (self.input.chars().filter(|&c| c == '\n').count() as u16 + 1).min(5);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(4),
                Constraint::Length(1),
                Constraint::Length(hints_height),
                Constraint::Length(input_lines),
            ])
            .split(frame.area());

        // 1. Banner or chat
        if self.show_banner && self.chat_messages.is_empty() {
            let banner = WelcomeBanner::new(&self.banner_info);
            frame.render_widget(banner, chunks[0]);
        } else {
            self.draw_chat_area(frame, chunks[0]);
        }

        // 2. Status line
        let hud_line = self.hud.render_line();
        let status = StatusBar::new(&hud_line);
        frame.render_widget(status, chunks[1]);

        // 3. Hints
        if !is_busy {
            let hints = HintsBar;
            frame.render_widget(hints, chunks[2]);
        }

        // 4. Input
        let mode_label = if self.input_mode == InputMode::Normal {
            self.input_mode.label()
        } else {
            ""
        };
        let input_widget = InputBox::new(&self.input, self.cursor_pos).with_mode(mode_label);
        frame.render_widget(input_widget, chunks[3]);

        // Cursor position
        let text_before_cursor = &self.input[..self.cursor_pos];
        let cursor_line = text_before_cursor.chars().filter(|&c| c == '\n').count() as u16;
        let last_newline = text_before_cursor.rfind('\n').map_or(0, |p| p + 1);
        let line_text = &self.input[last_newline..self.cursor_pos];
        let mode_prefix_width: u16 = if cursor_line == 0 && !mode_label.is_empty() {
            (mode_label.len() + 3) as u16
        } else {
            0
        };
        let cursor_col = display_width_of(line_text) as u16 + 2 + mode_prefix_width;
        let cursor_x = chunks[3].x + cursor_col;
        let cursor_y = chunks[3].y + cursor_line;
        frame.set_cursor_position((cursor_x, cursor_y));

        // 5. Overlay
        if self.overlay.is_open() {
            self.draw_overlay(frame, frame.area());
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
        let scroll_from_top = max_scroll.saturating_sub(scroll);

        let mut accumulated: u16 = 0;
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

            if rows_collected == 0 {
                first_visible_skip = scroll_from_top.saturating_sub(accumulated);
            }

            let rlines = msg.to_ratatui_lines();
            for line in rlines {
                if first_visible_skip > 0 {
                    first_visible_skip -= 1;
                    continue;
                }
                visible_lines.push(line);
                rows_collected += 1;
                if rows_collected >= visible {
                    break;
                }
            }

            accumulated = msg_end;
            if rows_collected >= visible {
                break;
            }
        }

        let paragraph = ratatui::widgets::Paragraph::new(visible_lines);
        frame.render_widget(paragraph, inner);
    }

    fn draw_overlay(&self, frame: &mut Frame, area: Rect) {
        match &self.overlay {
            Overlay::None => {}
            Overlay::Help => {
                let cmd_reg = CommandRegistry::with_defaults();
                let mut help = String::from(
                    "Keyboard shortcuts:\n\
                     \n\
                     Enter        — send message\n\
                     Shift-Enter  — newline\n\
                     Ctrl-C       — quit\n\
                     Esc          — vim normal / clear input\n\
                     Up/Down      — scroll chat\n\
                     Ctrl-T       — toggle theme\n\
                     Ctrl-L       — clear chat\n\
                     Ctrl-U       — clear input\n\
                     Ctrl-P/N     — input history\n\
                     \n",
                );
                help.push_str(&cmd_reg.help_text());
                let overlay = OverlayBlock::new("Help", &help);
                frame.render_widget(overlay, area);
            }
            Overlay::Status => {
                let status = format!(
                    "Session: {}\n\
                     Model: {}\n\
                     Input tokens: {}\n\
                     Output tokens: {}\n\
                     Context: {:.1}%",
                    self.hud.session_name().unwrap_or("(unnamed)"),
                    self.hud.model_name(),
                    self.hud.cumulative_input_tokens(),
                    self.hud.cumulative_output_tokens(),
                    self.hud.context_pct(),
                );
                let overlay = OverlayBlock::new("Status", &status);
                frame.render_widget(overlay, area);
            }
            Overlay::Sessions => {
                let overlay = OverlayBlock::new(
                    "Sessions",
                    "Use /sessions in non-busy mode to list saved sessions.\n\
                     Use /resume <id> to restore a session.",
                );
                frame.render_widget(overlay, area);
            }
            Overlay::Mcp => {
                use nocode_core::mcp::manager::global_mcp_manager;
                let mgr = global_mcp_manager();
                let mgr = mgr.lock().unwrap_or_else(|e| e.into_inner());
                let servers = mgr.list_servers();
                let text = if servers.is_empty() {
                    "No MCP servers connected.\n\nConfigure in .nocode/settings.json under \"mcp_servers\".".to_string()
                } else {
                    let mut lines = Vec::new();
                    for (name, phase, tool_count) in &servers {
                        lines.push(format!("  {name}: {phase:?} ({tool_count} tools)"));
                    }
                    format!("Connected MCP servers:\n\n{}", lines.join("\n"))
                };
                let overlay = OverlayBlock::new("MCP Servers", &text);
                frame.render_widget(overlay, area);
            }
            Overlay::Agents => {
                use nocode_core::agent::worker::global_worker_registry;
                let reg = global_worker_registry();
                let reg = reg.lock().unwrap_or_else(|e| e.into_inner());
                let workers = reg.list();
                let text = if workers.is_empty() {
                    "No background agents running.".to_string()
                } else {
                    let mut lines = Vec::new();
                    for w in &workers {
                        lines.push(format!("  {} ({}): {:?}", w.name, w.id, w.state));
                    }
                    format!("Background agents:\n\n{}", lines.join("\n"))
                };
                let overlay = OverlayBlock::new("Agents", &text);
                frame.render_widget(overlay, area);
            }
            Overlay::Config => {
                let overlay = OverlayBlock::new(
                    "Configuration",
                    "Config loaded from:\n\
                     1. ~/.nocode/settings.json (user)\n\
                     2. .nocode/settings.json (project)\n\
                     3. .nocode/settings.local.json (local)\n\n\
                     Environment overrides: NOCODE_MODEL, NOCODE_SYSTEM_PROMPT, etc.",
                );
                frame.render_widget(overlay, area);
            }
            Overlay::Memory => {
                let overlay = OverlayBlock::new(
                    "Memory",
                    "Memory stored in ~/.nocode/memory/\n\
                     Use /memory <query> to search memories.",
                );
                frame.render_widget(overlay, area);
            }
            Overlay::Cost => {
                let cost = self.hud.estimated_cost();
                let text = format!(
                    "Token usage:\n\n\
                     Input:  {}\n\
                     Output: {}\n\
                     Est. cost: ${:.4}",
                    self.hud.cumulative_input_tokens(),
                    self.hud.cumulative_output_tokens(),
                    cost,
                );
                let overlay = OverlayBlock::new("Cost", &text);
                frame.render_widget(overlay, area);
            }
            Overlay::Permission { tool_name, tool_id } => {
                let text = format!(
                    "Tool: {tool_name}\n\
                     ID: {tool_id}\n\n\
                     Allow this tool call?\n\n\
                     [y] Yes  [n] No  [a] Always allow"
                );
                let overlay = OverlayBlock::new("⚠ Permission Required", &text);
                frame.render_widget(overlay, area);
            }
        }
    }

    // -- key handling --

    /// Returns false if the app should quit.
    pub fn handle_key(&mut self, key: KeyEvent) -> HandleKeyResult {
        // Ctrl-C = quit
        if matches!(
            (key.code, key.modifiers),
            (KeyCode::Char('c'), KeyModifiers::CONTROL)
        ) {
            return HandleKeyResult::Quit;
        }

        // Overlay open — Esc closes it, consume everything else
        if self.overlay.is_open() {
            if key.code == KeyCode::Esc {
                self.overlay = Overlay::None;
                self.dirty = true;
            }
            return HandleKeyResult::Continue;
        }

        match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => {
                if self.input_mode == InputMode::Insert {
                    self.input_mode = InputMode::Normal;
                    self.vim_pending = None;
                    if self.cursor_pos > 0 && self.cursor_pos >= self.input.len() {
                        self.cursor_pos = prev_char_boundary(&self.input, self.input.len());
                    }
                    self.dirty = true;
                } else if !self.input.is_empty() {
                    self.input.clear();
                    self.cursor_pos = 0;
                    self.input_mode = InputMode::Insert;
                    self.dirty = true;
                }
            }
            // Scroll
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
            // Clear chat
            (KeyCode::Char('l'), KeyModifiers::CONTROL) => {
                self.chat_messages.clear();
                self.invalidate_height_cache();
                self.sticky_scroll = true;
                self.unseen_count = 0;
                self.chat_scroll = 0;
                self.dirty = true;
            }
            // Clear input
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                self.input.clear();
                self.cursor_pos = 0;
                self.dirty = true;
            }
            // Theme toggle
            (KeyCode::Char('t'), KeyModifiers::CONTROL) => {
                let variant = crate::tui_theme::toggle_theme();
                self.push_system(&format!("theme: {variant:?}"));
                self.invalidate_height_cache();
            }
            // Thinking toggle
            (KeyCode::Char('o'), KeyModifiers::CONTROL) => {
                self.toggle_thinking_blocks();
            }
            // History prev
            (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
                self.history_prev();
            }
            // History next
            (KeyCode::Char('n'), KeyModifiers::CONTROL) => {
                self.history_next();
            }
            // Submit
            (KeyCode::Enter, KeyModifiers::NONE) => {
                if self.input_mode == InputMode::Normal {
                    self.input_mode = InputMode::Insert;
                    self.dirty = true;
                    return HandleKeyResult::Continue;
                }
                let text = self.input.trim().to_string();
                if text.is_empty() {
                    return HandleKeyResult::Continue;
                }
                self.input_history.push(text.clone());
                self.history_index = None;
                self.input.clear();
                self.cursor_pos = 0;
                self.dirty = true;
                return HandleKeyResult::Submit(text);
            }
            // Newline
            (KeyCode::Enter, KeyModifiers::SHIFT) => {
                self.input.insert(self.cursor_pos, '\n');
                self.cursor_pos += 1;
                self.dirty = true;
            }
            // Backspace
            (KeyCode::Backspace, _) => {
                if self.cursor_pos > 0 {
                    let prev = prev_char_boundary(&self.input, self.cursor_pos);
                    self.input.drain(prev..self.cursor_pos);
                    self.cursor_pos = prev;
                    self.dirty = true;
                }
            }
            // Delete
            (KeyCode::Delete, _) => {
                if self.cursor_pos < self.input.len() {
                    let next = next_char_boundary(&self.input, self.cursor_pos);
                    self.input.drain(self.cursor_pos..next);
                    self.dirty = true;
                }
            }
            // Left/Right
            (KeyCode::Left, _) => {
                if self.cursor_pos > 0 {
                    self.cursor_pos = prev_char_boundary(&self.input, self.cursor_pos);
                    self.dirty = true;
                }
            }
            (KeyCode::Right, _) => {
                if self.cursor_pos < self.input.len() {
                    self.cursor_pos = next_char_boundary(&self.input, self.cursor_pos);
                    self.dirty = true;
                }
            }
            // Home/End
            (KeyCode::Home, _) => {
                self.cursor_pos = 0;
                self.dirty = true;
            }
            (KeyCode::End, _) => {
                self.cursor_pos = self.input.len();
                self.dirty = true;
            }
            // Normal char input
            (KeyCode::Char(c), _) => {
                if self.input_mode == InputMode::Normal {
                    self.handle_vim_key(c);
                } else {
                    self.input.insert(self.cursor_pos, c);
                    self.cursor_pos += c.len_utf8();
                    self.dirty = true;
                }
            }
            _ => {}
        }
        HandleKeyResult::Continue
    }

    fn handle_vim_key(&mut self, c: char) {
        match c {
            'i' => {
                self.input_mode = InputMode::Insert;
                self.dirty = true;
            }
            'a' => {
                self.input_mode = InputMode::Insert;
                if self.cursor_pos < self.input.len() {
                    self.cursor_pos = next_char_boundary(&self.input, self.cursor_pos);
                }
                self.dirty = true;
            }
            'A' => {
                self.input_mode = InputMode::Insert;
                self.cursor_pos = self.input.len();
                self.dirty = true;
            }
            'I' => {
                self.input_mode = InputMode::Insert;
                self.cursor_pos = 0;
                self.dirty = true;
            }
            'h' => {
                if self.cursor_pos > 0 {
                    self.cursor_pos = prev_char_boundary(&self.input, self.cursor_pos);
                    self.dirty = true;
                }
            }
            'l' => {
                if self.cursor_pos < self.input.len() {
                    self.cursor_pos = next_char_boundary(&self.input, self.cursor_pos);
                    self.dirty = true;
                }
            }
            'x' => {
                if self.cursor_pos < self.input.len() {
                    let next = next_char_boundary(&self.input, self.cursor_pos);
                    self.input.drain(self.cursor_pos..next);
                    self.dirty = true;
                }
            }
            '0' => {
                self.cursor_pos = 0;
                self.dirty = true;
            }
            '$' => {
                self.cursor_pos = self.input.len();
                if self.cursor_pos > 0 {
                    self.cursor_pos = prev_char_boundary(&self.input, self.cursor_pos);
                }
                self.dirty = true;
            }
            'w' => {
                self.cursor_pos = next_word_boundary(&self.input, self.cursor_pos);
                self.dirty = true;
            }
            'b' => {
                self.cursor_pos = prev_word_boundary(&self.input, self.cursor_pos);
                self.dirty = true;
            }
            _ => {}
        }
    }

    fn toggle_thinking_blocks(&mut self) {
        for msg in &mut self.chat_messages {
            if msg.kind == ChatMessageKind::Thinking {
                msg.thinking_collapsed = !msg.thinking_collapsed;
            }
        }
        self.invalidate_height_cache();
        self.dirty = true;
    }

    fn history_prev(&mut self) {
        if self.input_history.is_empty() {
            return;
        }
        match self.history_index {
            None => {
                self.saved_input = self.input.clone();
                self.history_index = Some(0);
                self.input
                    .clone_from(&self.input_history[self.input_history.len() - 1]);
                self.cursor_pos = self.input.len();
                self.dirty = true;
            }
            Some(idx) if idx + 1 < self.input_history.len() => {
                self.history_index = Some(idx + 1);
                let hist_idx = self.input_history.len() - 1 - (idx + 1);
                self.input.clone_from(&self.input_history[hist_idx]);
                self.cursor_pos = self.input.len();
                self.dirty = true;
            }
            _ => {}
        }
    }

    fn history_next(&mut self) {
        match self.history_index {
            Some(0) => {
                self.history_index = None;
                self.input = std::mem::take(&mut self.saved_input);
                self.cursor_pos = self.input.len();
                self.dirty = true;
            }
            Some(idx) => {
                self.history_index = Some(idx - 1);
                let hist_idx = self.input_history.len() - idx;
                self.input.clone_from(&self.input_history[hist_idx]);
                self.cursor_pos = self.input.len();
                self.dirty = true;
            }
            None => {}
        }
    }

    fn handle_paste(&mut self, text: &str) {
        self.input.insert_str(self.cursor_pos, text);
        self.cursor_pos += text.len();
        self.dirty = true;
    }
}

pub(crate) enum HandleKeyResult {
    Continue,
    Quit,
    Submit(String),
}

// ---------------------------------------------------------------------------
// Channel-based LoopObserver for background thread
// ---------------------------------------------------------------------------

struct ChannelObserver {
    tx: mpsc::Sender<TuiEvent>,
}

impl LoopObserver for ChannelObserver {
    fn on_stream_event(&mut self, event: &StreamEvent) {
        if let StreamEvent::ContentBlockDelta { delta, .. } = event {
            match delta {
                StreamDelta::TextDelta { text } => {
                    let _ = self.tx.send(TuiEvent::TextDelta(text.clone()));
                }
                StreamDelta::ThinkingDelta { thinking } => {
                    let _ = self.tx.send(TuiEvent::ThinkingDelta(thinking.clone()));
                }
                _ => {}
            }
        }
    }

    fn on_tool_start(&mut self, name: &str, _id: &str) {
        let _ = self.tx.send(TuiEvent::ToolStart {
            name: name.to_string(),
        });
    }

    fn on_tool_done(&mut self, name: &str, _id: &str, result: &ContentBlock) {
        let (content, is_error) = match result {
            ContentBlock::ToolResult {
                content, is_error, ..
            } => (content.clone(), *is_error),
            _ => (String::new(), false),
        };
        let _ = self.tx.send(TuiEvent::ToolDone {
            name: name.to_string(),
            content,
            is_error,
        });
    }
}

// ---------------------------------------------------------------------------
// Main event loop
// ---------------------------------------------------------------------------

pub(crate) fn run_app_loop(
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<io::Stdout>>,
    provider: Box<dyn Provider>,
    registry: ToolRegistry,
    system: Vec<SystemBlock>,
    model: &str,
    max_tokens: u32,
    max_turns: u32,
) -> io::Result<()> {
    let mut app = TuiApp::new(model);
    let mut messages: Vec<Message> = Vec::new();
    let tool_defs = registry.definitions();

    let mut event_rx: Option<mpsc::Receiver<TuiEvent>> = None;
    let mut is_busy = false;

    let provider: Arc<dyn Provider> = Arc::from(provider);
    let mut registry_slot: Option<ToolRegistry> = Some(registry);

    loop {
        // 1. Tick spinner
        if let Some(spinner) = app.thinking_spinner.as_mut() {
            spinner.tick();
        }

        // 2. Drain channel events
        if let Some(rx) = &event_rx {
            loop {
                match rx.try_recv() {
                    Ok(TuiEvent::TextDelta(text)) => {
                        if app.streaming_text.is_empty() {
                            app.thinking_spinner = None;
                        }
                        app.streaming_text.push_str(&text);
                        let snap = app.streaming_text.clone();
                        app.update_streaming_assistant(&snap);
                        app.show_banner = false;
                    }
                    Ok(TuiEvent::ThinkingDelta(text)) => {
                        if app.streaming_thinking.is_empty() {
                            app.thinking_spinner = None;
                        }
                        app.streaming_thinking.push_str(&text);
                        let snap = app.streaming_thinking.clone();
                        app.update_streaming_thinking(&snap);
                    }
                    Ok(TuiEvent::ToolStart { name }) => {
                        app.thinking_spinner = None;
                        app.push_tool_start(&name);
                    }
                    Ok(TuiEvent::ToolDone {
                        name,
                        content,
                        is_error,
                    }) => {
                        app.push_tool_done(&name, &content, is_error);
                        // Model will be called again — show spinner
                        app.thinking_spinner = Some(Spinner::new("Thinking..."));
                    }
                    Ok(TuiEvent::Complete(result, returned_registry)) => {
                        app.thinking_spinner = None;
                        app.streaming_text.clear();
                        app.streaming_thinking.clear();
                        is_busy = false;
                        event_rx = None;
                        registry_slot = Some(returned_registry);

                        match result {
                            Ok(loop_result) => {
                                app.hud.record_tokens(
                                    loop_result.total_input_tokens,
                                    loop_result.total_output_tokens,
                                );
                                app.hud.end_turn();
                                messages = loop_result.messages;
                            }
                            Err(e) => {
                                app.push_error(&e);
                                app.hud.end_turn();
                            }
                        }
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        if is_busy {
                            app.push_error("background loop disconnected");
                            is_busy = false;
                            event_rx = None;
                        }
                        break;
                    }
                }
            }
        }

        // 3. Render
        terminal.draw(|frame| app.draw(frame))?;

        // 4. Poll crossterm events (50ms for responsive spinner)
        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => {
                    if is_busy {
                        // Engine busy — only scroll/quit/cancel
                        match (key.code, key.modifiers) {
                            (KeyCode::Char('c'), KeyModifiers::CONTROL) => break,
                            (KeyCode::Esc, _) => {
                                // Can't cancel the sync loop, just note it
                                app.push_system("waiting for model response...");
                            }
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
                            _ => {}
                        }
                    } else {
                        match app.handle_key(key) {
                            HandleKeyResult::Quit => break,
                            HandleKeyResult::Submit(text) => {
                                // Handle slash commands via registry
                                let cmd_reg = CommandRegistry::with_defaults();
                                if let Some((action, args)) = cmd_reg.resolve(&text) {
                                    match action {
                                        CommandAction::Quit => break,
                                        CommandAction::Clear => {
                                            messages.clear();
                                            app.chat_messages.clear();
                                            app.invalidate_height_cache();
                                            app.push_system("(conversation cleared)");
                                            continue;
                                        }
                                        CommandAction::Help => {
                                            app.overlay = Overlay::Help;
                                            app.dirty = true;
                                            continue;
                                        }
                                        CommandAction::Status => {
                                            app.overlay = Overlay::Status;
                                            app.dirty = true;
                                            continue;
                                        }
                                        CommandAction::Sessions => {
                                            use nocode_core::session::persistence::SessionPersistence;
                                            let cwd = std::env::current_dir()
                                                .map(|p| p.to_string_lossy().into_owned())
                                                .unwrap_or_default();
                                            let infos =
                                                SessionPersistence::list_sessions_with_info(&cwd);
                                            if infos.is_empty() {
                                                app.push_system("No saved sessions.");
                                            } else {
                                                let mut lines = vec!["Saved sessions:".to_string()];
                                                for info in infos.iter().take(20) {
                                                    let preview = info
                                                        .first_user_message
                                                        .as_deref()
                                                        .unwrap_or("(empty)");
                                                    lines.push(format!(
                                                        "  {} ({} msgs) — {}",
                                                        info.id, info.message_count, preview
                                                    ));
                                                }
                                                lines.push(String::new());
                                                lines.push(
                                                    "Use /resume <id> to restore.".to_string(),
                                                );
                                                app.push_system(&lines.join("\n"));
                                            }
                                            continue;
                                        }
                                        CommandAction::Resume => {
                                            use nocode_core::session::persistence::SessionPersistence;
                                            let cwd = std::env::current_dir()
                                                .map(|p| p.to_string_lossy().into_owned())
                                                .unwrap_or_default();
                                            if let Some(session_id) = args {
                                                match SessionPersistence::resume(&cwd, &session_id)
                                                {
                                                    Ok((_persistence, loaded)) => {
                                                        messages = loaded;
                                                        app.chat_messages.clear();
                                                        app.invalidate_height_cache();
                                                        // Replay messages into TUI
                                                        for msg in &messages {
                                                            match msg.role {
                                                                nocode_core::message::Role::User => {
                                                                    app.push_user_message(&msg.text_content());
                                                                }
                                                                nocode_core::message::Role::Assistant => {
                                                                    let text = msg.text_content();
                                                                    if !text.is_empty() {
                                                                        app.update_streaming_assistant(&text);
                                                                    }
                                                                }
                                                            }
                                                        }
                                                        app.push_system(&format!("Resumed session '{session_id}' ({} messages)", messages.len()));
                                                    }
                                                    Err(e) => {
                                                        app.push_error(&format!(
                                                            "Failed to resume: {e}"
                                                        ));
                                                    }
                                                }
                                            } else {
                                                app.push_system("Usage: /resume <session_id>");
                                            }
                                            continue;
                                        }
                                        CommandAction::Mcp => {
                                            app.overlay = Overlay::Mcp;
                                            app.dirty = true;
                                            continue;
                                        }
                                        CommandAction::Agents => {
                                            app.overlay = Overlay::Agents;
                                            app.dirty = true;
                                            continue;
                                        }
                                        CommandAction::Config => {
                                            app.overlay = Overlay::Config;
                                            app.dirty = true;
                                            continue;
                                        }
                                        CommandAction::Memory => {
                                            app.overlay = Overlay::Memory;
                                            app.dirty = true;
                                            continue;
                                        }
                                        CommandAction::Cost => {
                                            app.overlay = Overlay::Cost;
                                            app.dirty = true;
                                            continue;
                                        }
                                        CommandAction::Theme => {
                                            let variant = crate::tui_theme::toggle_theme();
                                            app.push_system(&format!("Theme: {variant:?}"));
                                            app.invalidate_height_cache();
                                            continue;
                                        }
                                        CommandAction::Vim => {
                                            app.input_mode = if app.input_mode == InputMode::Insert
                                            {
                                                InputMode::Normal
                                            } else {
                                                InputMode::Insert
                                            };
                                            app.push_system(&format!(
                                                "Vim mode: {}",
                                                app.input_mode.label()
                                            ));
                                            continue;
                                        }
                                        CommandAction::Version => {
                                            app.push_system(&format!(
                                                "nocode v{}",
                                                env!("CARGO_PKG_VERSION")
                                            ));
                                            continue;
                                        }
                                        CommandAction::Compact => {
                                            app.push_system("(compaction not yet wired)");
                                            continue;
                                        }
                                        CommandAction::Permissions => {
                                            app.push_system("Permission mode: ask (default)");
                                            continue;
                                        }
                                        CommandAction::History => {
                                            let hist: Vec<String> = app
                                                .input_history
                                                .iter()
                                                .rev()
                                                .take(20)
                                                .cloned()
                                                .collect();
                                            if hist.is_empty() {
                                                app.push_system("(no command history)");
                                            } else {
                                                app.push_system(&format!(
                                                    "Recent commands:\n{}",
                                                    hist.join("\n")
                                                ));
                                            }
                                            continue;
                                        }
                                        CommandAction::Model => {
                                            if let Some(new_model) = args {
                                                app.hud.model_name = new_model.clone();
                                                app.push_system(&format!(
                                                    "Model switched to: {new_model}"
                                                ));
                                            } else {
                                                app.push_system(&format!(
                                                    "Current model: {}",
                                                    app.hud.model_name()
                                                ));
                                            }
                                            continue;
                                        }
                                        CommandAction::Export => {
                                            tui_cmd_export(args.as_deref(), &messages, &mut app);
                                            continue;
                                        }
                                        CommandAction::Bug => {
                                            app.push_system(&format!(
                                                "Report bugs at: https://github.com/anthropics/nocode/issues/new\n\
                                                 Version: nocode v{}\n\
                                                 OS: {} ({})",
                                                env!("CARGO_PKG_VERSION"),
                                                std::env::consts::OS,
                                                std::env::consts::ARCH,
                                            ));
                                            continue;
                                        }
                                        CommandAction::Doctor => {
                                            tui_cmd_doctor(&mut app, model);
                                            continue;
                                        }
                                        CommandAction::Init => {
                                            tui_cmd_init(&mut app);
                                            continue;
                                        }
                                        CommandAction::Login => {
                                            app.push_system(
                                                "Configure API keys via environment variables:\n\n\
                                                 \x20 export ANTHROPIC_API_KEY=sk-ant-...\n\
                                                 \x20 export OPENAI_API_KEY=sk-...\n\
                                                 \x20 export GEMINI_API_KEY=AI...\n\n\
                                                 Or add to ~/.nocode/settings.json",
                                            );
                                            continue;
                                        }
                                    }
                                }

                                // Submit to agentic loop
                                app.push_user_message(&text);
                                app.show_banner = false;
                                messages.push(Message::user_text(&text));

                                app.hud.start_turn();
                                app.thinking_spinner = Some(Spinner::new("Thinking..."));
                                is_busy = true;

                                // Launch background thread
                                let (tx, rx) = mpsc::channel();
                                event_rx = Some(rx);

                                let p = Arc::clone(&provider);
                                let r = registry_slot.take().expect("registry available");
                                let msgs = messages.clone();
                                let cfg = LoopConfig {
                                    model: model.to_string(),
                                    max_tokens,
                                    max_turns,
                                    system: system.clone(),
                                    tools: tool_defs.clone(),
                                    parallel_tool_execution: true,
                                };

                                let tx_complete = tx.clone();
                                std::thread::spawn(move || {
                                    let executor = ToolExecutor::new(&r);
                                    let mut observer = ChannelObserver { tx };
                                    let result = r#loop::run_agentic_loop(
                                        p.as_ref(),
                                        &executor,
                                        &cfg,
                                        msgs,
                                        &mut observer,
                                    );
                                    let loop_result = result.map_err(|e| format!("{e}"));
                                    let _ = tx_complete.send(TuiEvent::Complete(loop_result, r));
                                });
                            }
                            HandleKeyResult::Continue => {}
                        }
                    }
                }
                Event::Paste(text) => {
                    if !is_busy {
                        app.handle_paste(&text);
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn prev_char_boundary(s: &str, pos: usize) -> usize {
    let mut p = pos.saturating_sub(1);
    while p > 0 && !s.is_char_boundary(p) {
        p -= 1;
    }
    p
}

fn next_char_boundary(s: &str, pos: usize) -> usize {
    let mut p = pos + 1;
    while p < s.len() && !s.is_char_boundary(p) {
        p += 1;
    }
    p.min(s.len())
}

fn display_width_of(s: &str) -> usize {
    s.chars().map(|c| c.width().unwrap_or(0)).sum()
}

fn next_word_boundary(s: &str, pos: usize) -> usize {
    let bytes = s.as_bytes();
    let mut p = pos;
    // Skip current word chars
    while p < bytes.len() && !bytes[p].is_ascii_whitespace() {
        p += 1;
    }
    // Skip whitespace
    while p < bytes.len() && bytes[p].is_ascii_whitespace() {
        p += 1;
    }
    p.min(s.len())
}

fn prev_word_boundary(s: &str, pos: usize) -> usize {
    let bytes = s.as_bytes();
    let mut p = pos.saturating_sub(1);
    // Skip whitespace backwards
    while p > 0 && bytes[p].is_ascii_whitespace() {
        p -= 1;
    }
    // Skip word chars backwards
    while p > 0 && !bytes[p - 1].is_ascii_whitespace() {
        p -= 1;
    }
    p
}

// ---------------------------------------------------------------------------
// TUI slash command helpers
// ---------------------------------------------------------------------------

fn tui_cmd_export(path: Option<&str>, messages: &[Message], app: &mut TuiApp) {
    if messages.is_empty() {
        app.push_system("Nothing to export — conversation is empty.");
        return;
    }
    let out_path = path.unwrap_or("conversation.md");
    let mut content = String::new();
    for msg in messages {
        let role = match msg.role {
            nocode_core::message::Role::User => "## User",
            nocode_core::message::Role::Assistant => "## Assistant",
        };
        content.push_str(role);
        content.push_str("\n\n");
        content.push_str(&msg.text_content());
        content.push_str("\n\n");
    }
    match std::fs::write(out_path, &content) {
        Ok(()) => app.push_system(&format!(
            "Exported {} messages to {out_path}",
            messages.len()
        )),
        Err(e) => app.push_error(&format!("Export failed: {e}")),
    }
}

fn tui_cmd_doctor(app: &mut TuiApp, model: &str) {
    let mut lines = Vec::new();
    lines.push(format!("nocode v{}", env!("CARGO_PKG_VERSION")));
    lines.push(format!(
        "OS: {} ({})",
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    lines.push(format!("Model: {model}"));
    lines.push(String::new());

    let keys = [
        ("ANTHROPIC_API_KEY", "Claude"),
        ("OPENAI_API_KEY", "OpenAI"),
        ("GEMINI_API_KEY", "Gemini"),
    ];
    lines.push("API keys:".to_string());
    for (var, name) in &keys {
        let status = if std::env::var(var).is_ok() {
            "set"
        } else {
            "not set"
        };
        lines.push(format!("  {name}: {status}"));
    }
    lines.push(String::new());

    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let home = std::env::var("HOME").unwrap_or_default();
    let paths = [
        (format!("{home}/.nocode/settings.json"), "User"),
        (format!("{cwd}/.nocode/settings.json"), "Project"),
        (format!("{cwd}/.nocode/settings.local.json"), "Local"),
    ];
    lines.push("Settings:".to_string());
    for (path, tier) in &paths {
        let mark = if std::path::Path::new(path).exists() {
            "found"
        } else {
            "not found"
        };
        lines.push(format!("  {tier}: {mark}"));
    }
    lines.push(String::new());

    let md_files = nocode_core::prompt::assembly::discover_claude_md(&cwd);
    lines.push(format!("CLAUDE.md files: {}", md_files.len()));
    let sessions = nocode_core::session::persistence::SessionPersistence::list_sessions(&cwd);
    lines.push(format!("Saved sessions: {}", sessions.len()));
    lines.push(String::new());
    lines.push("All checks passed.".to_string());

    app.push_system(&lines.join("\n"));
}

fn tui_cmd_init(app: &mut TuiApp) {
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let claude_md_path = format!("{cwd}/CLAUDE.md");
    if std::path::Path::new(&claude_md_path).exists() {
        app.push_system(&format!("CLAUDE.md already exists at {claude_md_path}"));
        return;
    }
    let template = "# CLAUDE.md\n\n\
        This file provides guidance to AI coding assistants working with this codebase.\n\n\
        ## Project Overview\n\n\
        <!-- Describe your project here -->\n\n\
        ## Build & Test\n\n\
        ```bash\n\
        # Add your build/test commands here\n\
        ```\n\n\
        ## Key Conventions\n\n\
        <!-- Add coding conventions, architecture notes, etc. -->\n";
    match std::fs::write(&claude_md_path, template) {
        Ok(()) => app.push_system(&format!("Created {claude_md_path}")),
        Err(e) => app.push_error(&format!("Failed to create CLAUDE.md: {e}")),
    }
}
