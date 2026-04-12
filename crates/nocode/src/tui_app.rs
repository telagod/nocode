//! Ratatui-based TUI application core — v2 rewrite.
//!
//! Bridges the agentic loop (running on a background thread) to the TUI
//! via mpsc channels. The TUI thread owns the terminal and polls both
//! crossterm events and channel events at 50ms intervals.

use crate::command_registry::CommandRegistry;
use crate::markdown_render::render_markdown_to_lines;
use crate::spinner::Spinner;
use crate::status_hud::StatusHud;
use crate::tui_commands::{SlashResult, handle_slash_command};
use crate::tui_events::{ChannelObserver, TuiEvent};
use crate::tui_widgets::{
    ChatMessage, ChatMessageKind, HintsBar, InputBox, StatusBar, WelcomeBanner, WelcomeBannerInfo,
};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use nocode_core::config::settings::{Settings, SettingsTier};
use nocode_core::message::{Message, SystemBlock};
use nocode_core::provider::Provider;
use nocode_core::query::r#loop::{self, LoopConfig};
use nocode_core::tool::ToolRegistry;
use nocode_core::tool::executor::ToolExecutor;
use nocode_core::tool::global_registry::tool_definitions_for_model;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Block, Borders};
use std::io;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;
use unicode_width::UnicodeWidthChar;

const LOG_LIMIT: usize = 256;

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
    Config {
        selected: usize,
        tier: usize,
        editing: bool,
        input: String,
        status: Option<String>,
        model: String,
        custom_base_url: String,
        custom_api_format: String,
    },
    Memory,
    Cost,
    Permission {
        tool_name: String,
        tool_id: String,
    },
    Question {
        questions: serde_json::Value,
        selected: Vec<usize>,
    },
    Errors(Vec<String>),
}

impl Overlay {
    fn is_open(&self) -> bool {
        !matches!(self, Self::None)
    }
}

impl TuiApp {
    pub(crate) fn open_config_overlay(&mut self) {
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let settings = Settings::load_merged(&cwd);
        self.overlay = Overlay::Config {
            selected: 0,
            tier: 0,
            editing: false,
            input: String::new(),
            status: None,
            model: settings.model.unwrap_or_default(),
            custom_base_url: settings.custom_base_url.unwrap_or_default(),
            custom_api_format: settings
                .custom_api_format
                .unwrap_or_else(|| "openai".to_string()),
        };
        self.dirty = true;
    }
}

// ---------------------------------------------------------------------------
// TuiApp — main application state
// ---------------------------------------------------------------------------

pub(crate) struct TuiApp {
    pub(crate) chat_messages: Vec<ChatMessage>,
    pub(crate) input: String,
    pub(crate) cursor_pos: usize,
    pub(crate) chat_scroll: u16,
    pub(crate) overlay: Overlay,
    pub(crate) thinking_spinner: Option<Spinner>,
    pub(crate) dirty: bool,
    height_cache: Vec<u16>,
    height_cache_width: u16,
    pub(crate) sticky_scroll: bool,
    pub(crate) unseen_count: usize,
    pub(crate) show_banner: bool,
    banner_info: WelcomeBannerInfo,
    pub(crate) streaming_text: String,
    pub(crate) streaming_thinking: String,
    pub(crate) input_history: Vec<String>,
    pub(crate) history_index: Option<usize>,
    pub(crate) saved_input: String,
    pub(crate) input_mode: InputMode,
    pub(crate) vim_pending: Option<char>,
    pub(crate) hud: StatusHud,
    pub(crate) error_log: Vec<String>,
    /// Channel to send permission decisions back to the executor thread.
    pub(crate) permission_tx:
        Option<std::sync::mpsc::Sender<nocode_core::tool::permission::PermissionDecision>>,
    /// Channel to send question answers back to the executor thread.
    pub(crate) question_tx:
        Option<std::sync::mpsc::Sender<Result<nocode_core::tool::permission::UserAnswer, String>>>,
    /// Search state
    pub(crate) search_query: String,
    pub(crate) search_active: bool,
    pub(crate) search_matches: Vec<usize>,
    pub(crate) search_index: usize,
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
            error_log: Vec::new(),
            permission_tx: None,
            question_tx: None,
            search_query: String::new(),
            search_active: false,
            search_matches: Vec::new(),
            search_index: 0,
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
        self.error_log.push(text.to_string());
        self.chat_messages
            .push(ChatMessage::plain(ChatMessageKind::Error, text));
        self.on_message_added();
    }

    pub fn push_tool_start(&mut self, name: &str) {
        let info = crate::tui_widgets::ToolCallInfo::new(name, "");
        let msg = ChatMessage::tool_call(info, Vec::new());
        self.chat_messages.push(msg);
        self.on_message_added();
    }

    pub fn push_tool_done(&mut self, name: &str, content: &str, is_error: bool) {
        // Try to update the last Tool message with structured result
        let content_lines = crate::markdown_render::render_markdown_to_lines(content);
        if let Some(last) = self.chat_messages.last_mut()
            && last.kind == ChatMessageKind::Tool
        {
            if let Some(info) = &mut last.tool_info {
                info.result_preview = Some(content.to_string());
                info.collapsed = false;
                if is_error {
                    last.kind = ChatMessageKind::Error;
                }
            }
            last.lines = content_lines;
            self.invalidate_height_cache();
            self.dirty = true;
            return;
        }
        // Fallback: plain message
        let prefix = if is_error { "✖" } else { "⎿" };
        let display = if content.len() > 200 {
            format!("{}...", &content[..200])
        } else {
            content.to_string()
        };
        let kind = if is_error {
            ChatMessageKind::Error
        } else {
            ChatMessageKind::Tool
        };
        self.chat_messages.push(ChatMessage::plain(
            kind,
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
            .enumerate()
            .map(|(i, l)| {
                let mut rl = crate::markdown_render::RenderedLine::new();
                let prefix = if i == 0 { "∴ " } else { "  " };
                rl.push(crate::markdown_render::LineSegment::new(
                    format!("{prefix}{l}"),
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

    pub(crate) fn invalidate_height_cache(&mut self) {
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

        // 2. Status line (search overrides when active)
        let status_text = self
            .search_status()
            .or_else(|| self.slash_command_hint())
            .unwrap_or_else(|| self.hud.render_line());
        let status = StatusBar::new(&status_text);
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

        let theme = crate::tui_theme::default_theme();

        let mut accumulated: u16 = 0;
        let mut first_visible_skip: u16 = 0;
        let mut y_offset: u16 = 0;

        for (i, msg) in self.chat_messages.iter().enumerate() {
            let h = self.height_cache.get(i).copied().unwrap_or(1);
            let msg_end = accumulated + h;

            if msg_end <= scroll_from_top {
                accumulated = msg_end;
                continue;
            }

            if y_offset == 0 && accumulated < scroll_from_top {
                first_visible_skip = scroll_from_top.saturating_sub(accumulated);
            }

            // Pick background color by message kind
            let bg = match msg.kind {
                ChatMessageKind::User => theme.user_msg_bg,
                ChatMessageKind::Assistant => theme.assistant_msg_bg,
                ChatMessageKind::Tool => theme.tool_msg_bg,
                ChatMessageKind::Error => theme.error_msg_bg,
                _ => ratatui::style::Color::Reset,
            };

            let rlines = msg.to_ratatui_lines();
            for line in rlines {
                if first_visible_skip > 0 {
                    first_visible_skip -= 1;
                    continue;
                }

                // Render background fill for this line
                if bg != ratatui::style::Color::Reset {
                    let line_rect = Rect {
                        x: inner.x,
                        y: inner.y + y_offset,
                        width: inner.width,
                        height: 1,
                    };
                    let bg_block = Block::default().style(ratatui::style::Style::default().bg(bg));
                    frame.render_widget(bg_block, line_rect);
                }

                // Render the text line
                let line_rect = Rect {
                    x: inner.x,
                    y: inner.y + y_offset,
                    width: inner.width,
                    height: 1,
                };
                let para = ratatui::widgets::Paragraph::new(vec![line]);
                frame.render_widget(para, line_rect);

                y_offset += 1;
                if y_offset >= visible {
                    break;
                }
            }

            accumulated = msg_end;
            if y_offset >= visible {
                break;
            }
        }
    }

    fn draw_overlay(&self, frame: &mut Frame, area: Rect) {
        crate::tui_overlays::draw_overlay(&self.overlay, &self.hud, frame, area);
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

        // Overlay open — handle keys
        if self.overlay.is_open() {
            if matches!(self.overlay, Overlay::Config { .. }) {
                if let Overlay::Config {
                    ref mut selected,
                    ref mut tier,
                    ref mut editing,
                    ref mut input,
                    ref mut status,
                    ref mut model,
                    ref mut custom_base_url,
                    ref mut custom_api_format,
                } = self.overlay
                {
                    match key.code {
                        KeyCode::Esc => {
                            if *editing {
                                *editing = false;
                                input.clear();
                                *status = Some("Edit cancelled".to_string());
                            } else {
                                self.overlay = Overlay::None;
                            }
                            self.dirty = true;
                        }
                        KeyCode::Up if !*editing => {
                            if *selected > 0 {
                                *selected -= 1;
                            }
                            self.dirty = true;
                        }
                        KeyCode::Down if !*editing => {
                            if *selected < 2 {
                                *selected += 1;
                            }
                            self.dirty = true;
                        }
                        KeyCode::Tab if !*editing => {
                            *tier = (*tier + 1) % 3;
                            self.dirty = true;
                        }
                        KeyCode::Enter => {
                            if *editing {
                                match *selected {
                                    0 => *model = input.clone(),
                                    1 => *custom_base_url = input.clone(),
                                    _ => *custom_api_format = input.clone(),
                                }
                                *editing = false;
                                input.clear();
                                *status = Some("Field updated locally".to_string());
                            } else {
                                *editing = true;
                                match *selected {
                                    0 => input.clone_from(model),
                                    1 => input.clone_from(custom_base_url),
                                    _ => input.clone_from(custom_api_format),
                                }
                                *status = Some("Editing field".to_string());
                            }
                            self.dirty = true;
                        }
                        KeyCode::Char('s') | KeyCode::Char('S') if !*editing => {
                            let cwd = std::env::current_dir()
                                .map(|p| p.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            let tier_value = match *tier {
                                0 => SettingsTier::User,
                                1 => SettingsTier::Project,
                                _ => SettingsTier::Local,
                            };
                            let mut settings = Settings::load_merged(&cwd);
                            settings.model = if model.trim().is_empty() {
                                None
                            } else {
                                Some(model.trim().to_string())
                            };
                            settings.custom_base_url = if custom_base_url.trim().is_empty() {
                                None
                            } else {
                                Some(custom_base_url.trim().to_string())
                            };
                            settings.custom_api_format = if custom_api_format.trim().is_empty() {
                                None
                            } else {
                                Some(custom_api_format.trim().to_string())
                            };
                            match settings.save_tier(tier_value, &cwd) {
                                Ok(()) => {
                                    self.hud.model_name = if model.trim().is_empty() {
                                        self.hud.model_name.clone()
                                    } else {
                                        model.trim().to_string()
                                    };
                                    *status = Some("Saved configuration".to_string());
                                }
                                Err(e) => {
                                    *status = Some(format!("Save failed: {e}"));
                                }
                            }
                            self.dirty = true;
                        }
                        KeyCode::Backspace if *editing => {
                            input.pop();
                            self.dirty = true;
                        }
                        KeyCode::Char(c) if *editing => {
                            input.push(c);
                            self.dirty = true;
                        }
                        _ => {}
                    }
                }
                return HandleKeyResult::Continue;
            }
            // Permission overlay: y/n/a to respond
            if matches!(self.overlay, Overlay::Permission { .. }) {
                use nocode_core::tool::permission::PermissionDecision;
                let decision = match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => Some(PermissionDecision::Allow),
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                        Some(PermissionDecision::Deny)
                    }
                    KeyCode::Char('a') | KeyCode::Char('A') => {
                        Some(PermissionDecision::AlwaysAllow)
                    }
                    _ => None,
                };
                if let Some(d) = decision {
                    if let Some(tx) = self.permission_tx.take() {
                        let _ = tx.send(d);
                    }
                    self.overlay = Overlay::None;
                    self.dirty = true;
                }
                return HandleKeyResult::Continue;
            }
            // Question overlay: navigate options and confirm
            if matches!(self.overlay, Overlay::Question { .. }) {
                if let Overlay::Question {
                    ref questions,
                    ref mut selected,
                } = self.overlay
                {
                    match key.code {
                        KeyCode::Up => {
                            if !selected.is_empty() {
                                // Move to previous question
                            }
                        }
                        KeyCode::Down => {
                            if !selected.is_empty() {
                                // Move to next question
                            }
                        }
                        KeyCode::Left => {
                            if let Some(cur) = selected.first_mut()
                                && *cur > 0
                            {
                                *cur -= 1;
                            }
                        }
                        KeyCode::Right => {
                            if let (Some(cur), Some(arr)) =
                                (selected.first_mut(), questions.as_array())
                                && let Some(q) = arr.first()
                            {
                                let opt_count =
                                    q["options"].as_array().map(|a| a.len()).unwrap_or(1);
                                if *cur + 1 < opt_count {
                                    *cur += 1;
                                }
                            }
                        }
                        KeyCode::Char('\n') | KeyCode::Enter => {
                            // Build answer from selected options
                            let selections: Vec<String> = if let Some(arr) = questions.as_array() {
                                arr.iter()
                                    .enumerate()
                                    .map(|(i, q)| {
                                        let idx = selected.get(i).copied().unwrap_or(0);
                                        q["options"]
                                            .as_array()
                                            .and_then(|opts| opts.get(idx))
                                            .and_then(|o| o["label"].as_str())
                                            .unwrap_or("N/A")
                                            .to_string()
                                    })
                                    .collect()
                            } else {
                                Vec::new()
                            };
                            let answer = nocode_core::tool::permission::UserAnswer { selections };
                            if let Some(tx) = self.question_tx.take() {
                                let _ = tx.send(Ok(answer));
                            }
                            self.overlay = Overlay::None;
                            self.dirty = true;
                        }
                        KeyCode::Esc => {
                            if let Some(tx) = self.question_tx.take() {
                                let _ = tx.send(Err("Cancelled by user".to_string()));
                            }
                            self.overlay = Overlay::None;
                            self.dirty = true;
                        }
                        // Number keys 1-4 for quick option selection
                        KeyCode::Char(c) if ('1'..='4').contains(&c) => {
                            let idx = (c as usize) - ('1' as usize);
                            if let Some(cur) = selected.first_mut() {
                                *cur = idx;
                            }
                        }
                        _ => {}
                    }
                }
                self.dirty = true;
                return HandleKeyResult::Continue;
            }
            // Other overlays: Esc closes
            if key.code == KeyCode::Esc {
                self.overlay = Overlay::None;
                self.dirty = true;
            }
            return HandleKeyResult::Continue;
        }

        // Search mode — intercept keys for search input
        if self.search_active {
            match key.code {
                KeyCode::Esc => {
                    self.search_active = false;
                    self.search_query.clear();
                    self.search_matches.clear();
                    self.dirty = true;
                }
                KeyCode::Enter => {
                    // Jump to next match
                    if !self.search_matches.is_empty() {
                        self.search_index = (self.search_index + 1) % self.search_matches.len();
                        self.dirty = true;
                    }
                }
                KeyCode::Backspace => {
                    self.search_query.pop();
                    self.update_search_matches();
                    self.dirty = true;
                }
                KeyCode::Char(c) => {
                    self.search_query.push(c);
                    self.update_search_matches();
                    self.dirty = true;
                }
                _ => {}
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
            // Copy last assistant message to clipboard
            (KeyCode::Char('y'), KeyModifiers::CONTROL) => {
                self.copy_last_assistant_to_clipboard();
            }
            // Search toggle
            (KeyCode::Char('f'), KeyModifiers::CONTROL) => {
                if self.search_active {
                    self.search_active = false;
                    self.search_query.clear();
                    self.search_matches.clear();
                } else {
                    self.search_active = true;
                    self.search_query.clear();
                    self.search_matches.clear();
                    self.search_index = 0;
                }
                self.dirty = true;
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

    pub(crate) fn copy_last_assistant_to_clipboard(&mut self) {
        // Find last assistant message
        let text = self
            .chat_messages
            .iter()
            .rev()
            .find(|m| m.kind == ChatMessageKind::Assistant)
            .map(|m| {
                m.lines
                    .iter()
                    .map(|l| {
                        l.segments
                            .iter()
                            .map(|s| s.text.as_str())
                            .collect::<String>()
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            });

        let Some(text) = text else {
            self.push_system("No assistant message to copy.");
            return;
        };

        match copy_to_clipboard(&text) {
            Ok(()) => self.push_system("Copied to clipboard."),
            Err(e) => self.push_error(&format!("Clipboard: {e}")),
        }
    }

    fn update_search_matches(&mut self) {
        self.search_matches.clear();
        if self.search_query.is_empty() {
            return;
        }
        let query = self.search_query.to_lowercase();
        for (i, msg) in self.chat_messages.iter().enumerate() {
            let text: String = msg
                .lines
                .iter()
                .flat_map(|l| l.segments.iter().map(|s| s.text.as_str()))
                .collect();
            if text.to_lowercase().contains(&query) {
                self.search_matches.push(i);
            }
        }
        self.search_index = 0;
    }

    /// Get the search status line for display.
    pub(crate) fn search_status(&self) -> Option<String> {
        if !self.search_active {
            return None;
        }
        let count = self.search_matches.len();
        let idx = if count > 0 { self.search_index + 1 } else { 0 };
        Some(format!("Search: {} ({}/{})", self.search_query, idx, count))
    }

    pub(crate) fn slash_command_hint(&self) -> Option<String> {
        if !self.input.trim_start().starts_with('/') {
            return None;
        }
        let registry = CommandRegistry::with_defaults();
        let suggestions = registry.recommend(&self.input, 4);
        if suggestions.is_empty() {
            return Some("Commands: no matches".to_string());
        }
        let parts: Vec<String> = suggestions
            .into_iter()
            .map(|cmd| match cmd.argument_hint {
                Some(hint) => format!("/{} {} — {}", cmd.name, hint, cmd.summary),
                None => format!("/{} — {}", cmd.name, cmd.summary),
            })
            .collect();
        Some(format!("Commands: {}", parts.join("  ·  ")))
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
    let tool_defs = tool_definitions_for_model(&registry);

    // Auto-generate session ID for persistence
    let session_id = format!(
        "{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        std::process::id()
    );
    app.hud.session_id = session_id;

    // Auto-update check (non-blocking, cached)
    {
        let ff = nocode_core::config::feature_flags::global_feature_flags();
        let ff_guard = ff.lock().unwrap_or_else(|e| e.into_inner());
        if ff_guard.is_enabled(nocode_core::config::feature_flags::FeatureFlag::AutoUpdate) {
            drop(ff_guard);
            let home = std::env::var("HOME").unwrap_or_default();
            let cache_path = format!("{home}/.nocode/update_cache.json");
            let checker = nocode_core::update_checker::UpdateChecker::new(
                env!("CARGO_PKG_VERSION"),
                &cache_path,
                "telagod/nocode",
            );
            if let nocode_core::update_checker::UpdateStatus::UpdateAvailable {
                current,
                latest,
                download_url,
            } = checker.check_cached_only()
            {
                app.push_system(&format!(
                    "Update available: {current} → {latest}\n  {download_url}"
                ));
            }
        }
    }

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
                    Ok(TuiEvent::InputJsonDelta { name, partial_json }) => {
                        // Update last tool message with streaming args
                        if let Some(last) = app.chat_messages.last_mut()
                            && last.kind == ChatMessageKind::Tool
                        {
                            let preview = if partial_json.len() > 120 {
                                format!("{}...", &partial_json[..120])
                            } else {
                                partial_json
                            };
                            let label = if name.is_empty() { "tool" } else { &name };
                            last.lines = vec![{
                                let mut rl = crate::markdown_render::RenderedLine::new();
                                rl.push(crate::markdown_render::LineSegment::new(
                                    format!("\u{276F} {label} {preview}"),
                                    crossterm::style::Color::DarkGrey,
                                ));
                                rl
                            }];
                            app.invalidate_height_cache();
                            app.dirty = true;
                        }
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
                    Ok(TuiEvent::PermissionRequest {
                        tool_name,
                        tool_id,
                        response_tx,
                    }) => {
                        app.permission_tx = Some(response_tx);
                        app.overlay = Overlay::Permission { tool_name, tool_id };
                        app.dirty = true;
                    }
                    Ok(TuiEvent::QuestionRequest {
                        questions,
                        response_tx,
                    }) => {
                        app.question_tx = Some(response_tx);
                        // Initialize selection index per question (default: first option)
                        let selected = questions
                            .as_array()
                            .map(|arr| vec![0usize; arr.len()])
                            .unwrap_or_default();
                        app.overlay = Overlay::Question {
                            questions,
                            selected,
                        };
                        app.dirty = true;
                    }
                    Ok(TuiEvent::MessagesUpdated(updated_msgs)) => {
                        // Incremental session persistence
                        use nocode_core::session::persistence::SessionPersistence;
                        let cwd = std::env::current_dir()
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let session_id = app.hud.session_id.clone();
                        if !session_id.is_empty() {
                            let mut persistence = SessionPersistence::new(&cwd, &session_id);
                            persistence.flush_incremental(&updated_msgs);
                        }
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
                                app.hud.record_cache_tokens(
                                    loop_result.total_cache_read_tokens,
                                    loop_result.total_cache_write_tokens,
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
                                    match handle_slash_command(
                                        action,
                                        args,
                                        &mut app,
                                        &mut messages,
                                        model,
                                    ) {
                                        SlashResult::Quit => break,
                                        SlashResult::Handled => continue,
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
                                let tx_perm = tx.clone();
                                std::thread::spawn(move || {
                                    let perm_bridge =
                                        crate::tui_events::TuiEventPermissionBridge::new(tx_perm);
                                    let q_bridge =
                                        crate::tui_events::TuiEventQuestionBridge::new(tx.clone());
                                    // Inject question prompter into AskUserQuestion tool
                                    if let Some(ask_tool) = r.get_as::<nocode_core::tool::interactive_tools::AskUserQuestionTool>("AskUserQuestion") {
                                        ask_tool.set_prompter(Box::new(q_bridge));
                                    }
                                    let executor =
                                        ToolExecutor::new(&r).with_prompter(&perm_bridge);
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

/// Copy text to system clipboard. Cross-platform: xclip/xsel (Linux), pbcopy (macOS), clip.exe (WSL/Windows).
fn copy_to_clipboard(text: &str) -> Result<(), String> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let candidates: &[&[&str]] = if cfg!(target_os = "macos") {
        &[&["pbcopy"]]
    } else {
        // Linux: try xclip, xsel, wl-copy (Wayland), clip.exe (WSL)
        &[
            &["xclip", "-selection", "clipboard"],
            &["xsel", "--clipboard", "--input"],
            &["wl-copy"],
            &["clip.exe"],
        ]
    };

    for args in candidates {
        let program = args[0];
        let extra_args = &args[1..];
        if let Ok(mut child) = Command::new(program)
            .args(extra_args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            if let Some(stdin) = child.stdin.as_mut() {
                let _ = stdin.write_all(text.as_bytes());
            }
            if let Ok(status) = child.wait()
                && status.success()
            {
                return Ok(());
            }
        }
    }

    Err("No clipboard tool found (tried xclip, xsel, wl-copy, pbcopy, clip.exe)".to_string())
}
