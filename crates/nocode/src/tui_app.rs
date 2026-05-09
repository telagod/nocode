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
    ChatMessage, ChatMessageKind, InputBox, StatusBar, WelcomeBanner, WelcomeBannerInfo,
};

use base64::Engine as _;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use nocode_core::config::settings::Settings;
use nocode_core::message::{ContentBlock, Message, SystemBlock};
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
use unicode_width::UnicodeWidthStr;

const LOG_LIMIT: usize = 256;

// ---------------------------------------------------------------------------
// Provider presets (imported from single source of truth)
// ---------------------------------------------------------------------------

#[cfg(test)]
use crate::provider_presets::ALL_PRESETS;

/// Detect which preset matches the current custom URL, if any.
#[cfg(test)]
pub(crate) fn detect_preset(base_url: &str) -> Option<usize> {
    let normalized = base_url.trim().trim_end_matches('/');
    ALL_PRESETS.iter().position(|p| {
        p.base_url
            .trim_end_matches('/')
            .eq_ignore_ascii_case(normalized)
    })
}

/// Get the preset name for display, or "Manual" if no preset matches.
#[cfg(test)]
pub(crate) fn preset_label(index: Option<usize>) -> &'static str {
    match index {
        Some(i) if i < ALL_PRESETS.len() => ALL_PRESETS[i].name,
        _ => "Manual",
    }
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
pub(crate) enum Overlay {
    #[default]
    None,
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

impl TuiApp {}

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
    /// Background worker event receiver
    pub(crate) worker_event_rx:
        Option<std::sync::mpsc::Receiver<nocode_core::agent::worker::WorkerEvent>>,
    /// Command completion state
    pub(crate) completion_selected: Option<usize>,
    /// Pending images from clipboard paste, awaiting submit.
    pub(crate) pending_images: Vec<PendingImage>,
    /// Overlay scroll offset for scrollable overlays.
    pub(crate) overlay_scroll: u16,
    /// Horizontal scroll offset for input box (single-line long input).
    pub(crate) input_view_offset: usize,
    /// Vertical scroll offset for input box (multi-line input).
    pub(crate) input_scroll_y: u16,
    /// Last rendered input rect (for completion popup positioning).
    pub(crate) last_input_rect: Option<Rect>,
}

/// An image pasted from clipboard, waiting to be sent with the next message.
pub(crate) struct PendingImage {
    pub media_type: String,
    pub base64_data: String,
    pub size_bytes: usize,
}

impl TuiApp {
    pub fn new(model: &str) -> Self {
        let banner = WelcomeBannerInfo {
            model: model.to_string(),
            ..WelcomeBannerInfo::default()
        };
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
            banner_info: banner,
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
            worker_event_rx: None,
            completion_selected: None,
            pending_images: Vec::new(),
            overlay_scroll: 0,
            input_view_offset: 0,
            input_scroll_y: 0,
            last_input_rect: None,
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
                // Auto-expand errors, keep normal results collapsed
                info.collapsed = !is_error;
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
            crate::tool_render::truncate_str(content, 200)
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
            self.invalidate_last_height();
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
                    ratatui::style::Color::DarkGray,
                ));
                rl
            })
            .collect();
        if let Some(last) = self.chat_messages.last_mut()
            && last.kind == ChatMessageKind::Thinking
        {
            last.lines = lines;
            self.invalidate_last_height();
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

    /// Invalidate only the last message's cached height (for streaming updates).
    fn invalidate_last_height(&mut self) {
        if !self.height_cache.is_empty() {
            self.height_cache.pop();
        }
    }

    fn message_height(msg: &ChatMessage, width: u16) -> u16 {
        // Fast path: collapsed tool messages are 1 line
        if (msg.kind == ChatMessageKind::Tool || msg.kind == ChatMessageKind::Error)
            && let Some(info) = &msg.tool_info
            && info.collapsed
        {
            return 1;
        }
        // Use pre-computed lines count instead of re-rendering
        let mut total: u16 = 0;
        for line in &msg.lines {
            let line_width: usize = line
                .segments
                .iter()
                .map(|s| UnicodeWidthStr::width(s.text.as_str()))
                .sum();
            let wrapped = if width > 0 {
                (line_width as u16).div_ceil(width)
            } else {
                1
            };
            total += wrapped.max(1);
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
        let msg_h: u16 = self.height_cache.iter().copied().sum();
        let spacers = self.height_cache.len().saturating_sub(1) as u16;
        msg_h + spacers
    }

    // -- drawing --

    pub fn draw(&mut self, frame: &mut Frame) {
        let total_height = frame.area().height;
        if total_height < 4 || frame.area().width < 20 {
            let msg = ratatui::widgets::Paragraph::new("Terminal too small");
            frame.render_widget(msg, frame.area());
            self.dirty = false;
            return;
        }

        let is_busy = self.thinking_spinner.is_some();

        // Unified layout: content area fills everything, bottom area for status or completion
        let has_completion = self.completion_selected.is_some() && !self.overlay.is_open();
        let completion_count = if has_completion {
            self.completion_suggestions().len().min(10) as u16
        } else {
            0
        };
        let bottom_h = if completion_count > 0 {
            completion_count + 1 // completion rows + status bar
        } else {
            1 // just status bar
        };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(bottom_h)])
            .split(frame.area());

        let content_area = chunks[0];
        let status_area = chunks[1];

        // 1. Banner or unified content (chat + input in one scrollable flow)
        if self.show_banner {
            // Input at bottom, banner fills the space above
            let input_lines =
                (self.input.chars().filter(|&c| c == '\n').count() as u16 + 1).min(10);
            let input_h = (input_lines + 1).min(content_area.height.saturating_sub(2)); // +1 for separator

            // If there are system messages (warnings, update notices), show them between banner and input
            let system_lines: Vec<ratatui::text::Line<'_>> = self
                .chat_messages
                .iter()
                .flat_map(|m| m.to_ratatui_lines())
                .collect();
            let system_h = (system_lines.len() as u16).min(content_area.height / 3);

            let banner_h = content_area
                .height
                .saturating_sub(input_h)
                .saturating_sub(system_h);

            if banner_h > 0 {
                let banner_area = Rect {
                    x: content_area.x,
                    y: content_area.y,
                    width: content_area.width,
                    height: banner_h,
                };
                let banner = WelcomeBanner::new(&self.banner_info);
                frame.render_widget(banner, banner_area);
            }

            // Render system messages (warnings etc.) between banner and input
            if system_h > 0 && !system_lines.is_empty() {
                let sys_area = Rect {
                    x: content_area.x,
                    y: content_area.y + banner_h,
                    width: content_area.width,
                    height: system_h,
                };
                let para = ratatui::widgets::Paragraph::new(system_lines)
                    .wrap(ratatui::widgets::Wrap { trim: false });
                frame.render_widget(para, sys_area);
            }

            if input_h > 0 {
                let input_rect = Rect {
                    x: content_area.x,
                    y: content_area.y + banner_h + system_h,
                    width: content_area.width,
                    height: input_h,
                };
                let mode_label = if self.input_mode == InputMode::Normal {
                    self.input_mode.label()
                } else {
                    ""
                };
                let input_widget = InputBox::new(&self.input, self.cursor_pos)
                    .with_mode(mode_label)
                    .with_view_offset(self.input_view_offset)
                    .with_scroll_y(self.input_scroll_y);
                frame.render_widget(input_widget, input_rect);
                self.last_input_rect = Some(input_rect);
                self.set_cursor_in_rect(frame, input_rect, mode_label);
            }
        } else {
            self.draw_unified_content(frame, content_area);
        }

        // 2. Status bar (always visible, floating at bottom)
        let pending_img_hint = if self.pending_images.is_empty() {
            None
        } else {
            let total_kb: usize = self
                .pending_images
                .iter()
                .map(|i| i.size_bytes / 1024)
                .sum();
            Some(format!(
                "[{} image{} attached, {}KB]",
                self.pending_images.len(),
                if self.pending_images.len() > 1 {
                    "s"
                } else {
                    ""
                },
                total_kb
            ))
        };
        let mut status_base = pending_img_hint
            .or_else(|| self.search_status())
            .or_else(|| self.slash_command_hint())
            .unwrap_or_else(|| self.hud.render_line());
        {
            let history = nocode_core::storage::file_history::global_file_history();
            if let Ok(h) = history.lock() {
                let uc = h.undo_count();
                let rc = h.redo_count();
                if uc > 0 || rc > 0 {
                    status_base.push_str(&format!(" | undo:{uc} redo:{rc}"));
                }
            }
        }
        if self.unseen_count > 0 && self.chat_scroll > 0 {
            status_base.push_str(&format!(" | {} new", self.unseen_count));
        }
        // Hints inline with status when not busy
        if !is_busy {
            let hints_text = self.hints_text();
            if !hints_text.is_empty() {
                status_base.push_str(" | ");
                status_base.push_str(&hints_text);
            }
        }
        let status = StatusBar::new(&status_base);

        // 3. Bottom area: completion dropdown (if active) + status bar
        if completion_count > 0 {
            // Split bottom area: completion rows on top, status bar at very bottom
            let bottom_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(completion_count), Constraint::Length(1)])
                .split(status_area);
            let completion_area = bottom_chunks[0];
            let real_status_area = bottom_chunks[1];
            frame.render_widget(status, real_status_area);
            if let Some(input_rect) = self.last_input_rect {
                self.draw_completion_inline(frame, completion_area, input_rect);
            }
        } else {
            frame.render_widget(status, status_area);
        }

        // 4. Overlay
        if self.overlay.is_open() {
            self.draw_overlay(frame, frame.area());
        }

        self.dirty = false;
    }

    fn hints_text(&self) -> String {
        if self.input_mode == InputMode::Normal {
            "i:insert  /:cmd  ?:help  q:quit".to_string()
        } else if self.completion_selected.is_some() {
            "Tab/↓:next  Enter:accept  Esc:dismiss".to_string()
        } else if !self.pending_images.is_empty() {
            "Enter:send  Esc:clear images".to_string()
        } else {
            "Enter:send  Shift+Enter:newline  Esc:vim  /:cmd".to_string()
        }
    }

    fn set_cursor_in_rect(&mut self, frame: &mut Frame, input_rect: Rect, mode_label: &str) {
        let text_before_cursor = &self.input[..self.cursor_pos];
        let cursor_line = text_before_cursor.chars().filter(|&c| c == '\n').count() as u16;
        let last_newline = text_before_cursor.rfind('\n').map_or(0, |p| p + 1);
        let line_text = &self.input[last_newline..self.cursor_pos];
        let mode_prefix_width: u16 = if cursor_line == 0 && !mode_label.is_empty() {
            (mode_label.len() + 3) as u16
        } else {
            0
        };
        let char_col = line_text.chars().count();
        // Account for separator line occupying the first row
        let sep_offset: u16 = if input_rect.height >= 2 { 1 } else { 0 };
        let content_height = input_rect.height.saturating_sub(sep_offset);
        let usable_width = input_rect.width.saturating_sub(2 + mode_prefix_width) as usize;
        if usable_width > 0 {
            if char_col < self.input_view_offset {
                self.input_view_offset = char_col;
            } else if char_col >= self.input_view_offset + usable_width {
                self.input_view_offset = char_col.saturating_sub(usable_width) + 1;
            }
        }
        let visible_col = char_col.saturating_sub(self.input_view_offset) as u16;
        let cursor_col = visible_col + 2 + mode_prefix_width;
        let cursor_x = input_rect.x + cursor_col;
        if content_height > 0 {
            if cursor_line < self.input_scroll_y {
                self.input_scroll_y = cursor_line;
            } else if cursor_line >= self.input_scroll_y + content_height {
                self.input_scroll_y = cursor_line.saturating_sub(content_height) + 1;
            }
        }
        let visible_cursor_line = cursor_line.saturating_sub(self.input_scroll_y);
        let cursor_y = input_rect.y + sep_offset + visible_cursor_line;
        if cursor_y < input_rect.y + input_rect.height {
            frame.set_cursor_position((cursor_x, cursor_y));
        }
    }

    fn draw_unified_content(&mut self, frame: &mut Frame, area: Rect) {
        let block = Block::default().borders(Borders::NONE);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        self.ensure_height_cache(inner.width);

        let input_lines =
            (self.input.chars().filter(|&c| c == '\n').count() as u16 + 1).clamp(1, 10) + 1; // +1 for separator

        let chat_h = self.total_content_height();
        let separator_h: u16 = if chat_h > 0 { 1 } else { 0 };
        let total_h = chat_h + separator_h + input_lines;

        let visible = inner.height;
        let max_scroll = total_h.saturating_sub(visible);
        let scroll = self.chat_scroll.min(max_scroll);
        let scroll_from_top = max_scroll.saturating_sub(scroll);

        // Bottom-align: when content is shorter than viewport, push it down
        let top_padding = visible.saturating_sub(total_h);

        let theme = crate::tui_theme::default_theme();

        let mut accumulated: u16 = 0;
        let mut first_visible_skip: u16 = 0;
        let mut y_offset: u16 = top_padding;

        for (i, msg) in self.chat_messages.iter().enumerate() {
            let h = self.height_cache.get(i).copied().unwrap_or(1);
            let spacer: u16 = if i + 1 < self.chat_messages.len() {
                1
            } else {
                0
            };
            let msg_end = accumulated + h + spacer;

            if msg_end <= scroll_from_top {
                accumulated = msg_end;
                continue;
            }

            if y_offset == 0 && accumulated < scroll_from_top {
                first_visible_skip = scroll_from_top.saturating_sub(accumulated);
            }

            let bg = match msg.kind {
                ChatMessageKind::User => theme.user_msg_bg,
                ChatMessageKind::Assistant => theme.assistant_msg_bg,
                ChatMessageKind::Tool => theme.tool_msg_bg,
                ChatMessageKind::Error => theme.error_msg_bg,
                _ => ratatui::style::Color::Reset,
            };

            let rlines = msg.to_ratatui_lines();
            let search_q = if self.search_active && !self.search_query.is_empty() {
                Some(self.search_query.clone())
            } else {
                None
            };
            for line in rlines {
                let line = if let Some(ref q) = search_q {
                    highlight_search_in_line(line, q)
                } else {
                    line
                };

                let line_display_width: usize = line.spans.iter().map(|s| s.width()).sum();
                let line_rows = if inner.width > 0 && line_display_width > inner.width as usize {
                    (line_display_width as u16).div_ceil(inner.width)
                } else {
                    1
                };

                if first_visible_skip > 0 {
                    if first_visible_skip >= line_rows {
                        first_visible_skip -= line_rows;
                        continue;
                    }
                    first_visible_skip = 0;
                }

                let rows_available = visible.saturating_sub(y_offset);
                let render_rows = line_rows.min(rows_available);

                if bg != ratatui::style::Color::Reset {
                    let line_rect = Rect {
                        x: inner.x,
                        y: inner.y + y_offset,
                        width: inner.width,
                        height: render_rows,
                    };
                    let bg_block = Block::default().style(ratatui::style::Style::default().bg(bg));
                    frame.render_widget(bg_block, line_rect);
                }

                let line_rect = Rect {
                    x: inner.x,
                    y: inner.y + y_offset,
                    width: inner.width,
                    height: render_rows,
                };
                let para = ratatui::widgets::Paragraph::new(vec![line])
                    .wrap(ratatui::widgets::Wrap { trim: false });
                frame.render_widget(para, line_rect);

                y_offset += render_rows;
                if y_offset >= visible {
                    break;
                }
            }

            // Breathing spacer between messages (not after last)
            if spacer > 0 && y_offset < visible && first_visible_skip == 0 {
                y_offset += 1;
            }

            accumulated = msg_end;
            if y_offset >= visible {
                break;
            }
        }

        // Render separator between chat and input (if there are messages)
        if y_offset < visible && chat_h > 0 {
            // Account for separator in scroll math
            let sep_start = chat_h;
            if sep_start >= scroll_from_top && sep_start < scroll_from_top + visible {
                // Skip separator if partially scrolled past
                if first_visible_skip == 0 {
                    y_offset += 1;
                }
            } else if sep_start < scroll_from_top {
                // separator scrolled past
            } else {
                y_offset += 1;
            }
        }

        // Render input box as part of the content flow
        if y_offset < visible {
            let input_rect = Rect {
                x: inner.x,
                y: inner.y + y_offset,
                width: inner.width,
                height: input_lines.min(visible.saturating_sub(y_offset)),
            };
            let mode_label = if self.input_mode == InputMode::Normal {
                self.input_mode.label()
            } else {
                ""
            };
            let input_widget = InputBox::new(&self.input, self.cursor_pos)
                .with_mode(mode_label)
                .with_view_offset(self.input_view_offset)
                .with_scroll_y(self.input_scroll_y);
            frame.render_widget(input_widget, input_rect);
            self.last_input_rect = Some(input_rect);
            self.set_cursor_in_rect(frame, input_rect, mode_label);
        } else {
            self.last_input_rect = None;
        }
    }

    fn draw_overlay(&self, frame: &mut Frame, area: Rect) {
        crate::tui_overlays::draw_overlay(
            &self.overlay,
            &self.hud,
            self.overlay_scroll,
            frame,
            area,
        );
    }

    /// Draw completion suggestions inline in a dedicated area (Pi-style dropdown below input).
    fn draw_completion_inline(&self, frame: &mut Frame, area: Rect, _input_area: Rect) {
        let suggestions = self.completion_suggestions();
        if suggestions.is_empty() || area.height == 0 {
            return;
        }
        let selected = self.completion_selected.unwrap_or(0);
        let theme = crate::tui_theme::default_theme();
        let count = suggestions.len().min(area.height as usize);

        for (i, (label, summary)) in suggestions.iter().take(count).enumerate() {
            let y = area.y + i as u16;
            if y >= area.y + area.height {
                break;
            }
            let is_selected = i == selected;
            let line_area = Rect {
                x: area.x,
                y,
                width: area.width,
                height: 1,
            };

            let max_label = 20.min(area.width as usize / 2);
            let display_label = safe_truncate(label, max_label);
            let remaining = area.width as usize - display_label.len() - 1;
            let display_summary = safe_truncate_ellipsis(summary, remaining);

            let style = if is_selected {
                ratatui::style::Style::default()
                    .fg(theme.background)
                    .bg(theme.claude)
            } else {
                ratatui::style::Style::default().fg(theme.text_dim)
            };

            if is_selected {
                let bg_block =
                    Block::default().style(ratatui::style::Style::default().bg(theme.claude));
                frame.render_widget(bg_block, line_area);
            }

            let text = format!("  {display_label} {display_summary}");
            let para = ratatui::widgets::Paragraph::new(text).style(style);
            frame.render_widget(para, line_area);
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

        // Overlay open — handle keys
        if self.overlay.is_open() {
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
                        KeyCode::Up if !selected.is_empty() => {
                            // Move to previous question
                        }
                        KeyCode::Down if !selected.is_empty() => {
                            // Move to next question
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
            // Other overlays: Esc closes, Up/Down/PageUp/PageDown scrolls
            match key.code {
                KeyCode::Esc => {
                    self.overlay = Overlay::None;
                    self.overlay_scroll = 0;
                    self.dirty = true;
                }
                KeyCode::Up => {
                    self.overlay_scroll = self.overlay_scroll.saturating_add(1);
                    self.dirty = true;
                }
                KeyCode::Down => {
                    self.overlay_scroll = self.overlay_scroll.saturating_sub(1);
                    self.dirty = true;
                }
                KeyCode::PageUp => {
                    self.overlay_scroll = self.overlay_scroll.saturating_add(10);
                    self.dirty = true;
                }
                KeyCode::PageDown => {
                    self.overlay_scroll = self.overlay_scroll.saturating_sub(10);
                    self.dirty = true;
                }
                _ => {}
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
                KeyCode::Enter
                    // Jump to next match
                    if !self.search_matches.is_empty() => {
                        self.search_index = (self.search_index + 1) % self.search_matches.len();
                        let msg_idx = self.search_matches[self.search_index];
                        self.scroll_to_message(msg_idx);
                        self.dirty = true;
                    }
                KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL)
                    && !self.search_matches.is_empty() => {
                        self.search_index = (self.search_index + 1) % self.search_matches.len();
                        let msg_idx = self.search_matches[self.search_index];
                        self.scroll_to_message(msg_idx);
                        self.dirty = true;
                    }
                KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL)
                    && !self.search_matches.is_empty() => {
                        self.search_index = if self.search_index == 0 {
                            self.search_matches.len() - 1
                        } else {
                            self.search_index - 1
                        };
                        let msg_idx = self.search_matches[self.search_index];
                        self.scroll_to_message(msg_idx);
                        self.dirty = true;
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

        // Command completion intercept — when popup is active
        if self.completion_selected.is_some() {
            match (key.code, key.modifiers) {
                (KeyCode::Up, KeyModifiers::NONE) => {
                    if let Some(sel) = self.completion_selected.as_mut() {
                        *sel = sel.saturating_sub(1);
                    }
                    self.dirty = true;
                    return HandleKeyResult::Continue;
                }
                (KeyCode::Down, KeyModifiers::NONE) => {
                    let count = self.completion_suggestions().len();
                    if let Some(sel) = self.completion_selected.as_mut()
                        && *sel + 1 < count
                    {
                        *sel += 1;
                    }
                    self.dirty = true;
                    return HandleKeyResult::Continue;
                }
                (KeyCode::Tab, _) | (KeyCode::Enter, KeyModifiers::NONE) => {
                    // Apply selected completion
                    let suggestions = self.completion_suggestions();
                    if let Some(sel) = self.completion_selected
                        && let Some((label, _)) = suggestions.get(sel)
                    {
                        // Extract command name (e.g. "/help" from "/help [topic]")
                        let cmd = label.split_whitespace().next().unwrap_or(label);
                        self.input = cmd.to_string();
                        self.cursor_pos = self.input.len();
                    }
                    self.completion_selected = None;
                    self.dirty = true;
                    return HandleKeyResult::Continue;
                }
                (KeyCode::Esc, _) => {
                    self.completion_selected = None;
                    self.dirty = true;
                    return HandleKeyResult::Continue;
                }
                _ => {
                    // Fall through to normal handling, but clear completion
                    // if it's not a character key (completion will be re-evaluated)
                }
            }
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
                    self.input_view_offset = 0;
                    self.input_scroll_y = 0;
                    self.input_mode = InputMode::Insert;
                    self.update_completion();
                    self.dirty = true;
                } else if !self.pending_images.is_empty() {
                    let count = self.pending_images.len();
                    self.pending_images.clear();
                    self.push_system(&format!("Cleared {count} pending image(s)."));
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
                self.input_view_offset = 0;
                self.input_scroll_y = 0;
                self.update_completion();
                self.dirty = true;
            }
            // Jump to start of line
            (KeyCode::Char('a'), KeyModifiers::CONTROL) => {
                // Find start of current line
                let before = &self.input[..self.cursor_pos];
                self.cursor_pos = before.rfind('\n').map_or(0, |p| p + 1);
                self.dirty = true;
            }
            // Jump to end of line
            (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
                let after = &self.input[self.cursor_pos..];
                self.cursor_pos += after.find('\n').unwrap_or(after.len());
                self.dirty = true;
            }
            // Delete word backward
            (KeyCode::Char('w'), KeyModifiers::CONTROL) if self.cursor_pos > 0 => {
                let before = &self.input[..self.cursor_pos];
                // Skip trailing whitespace, then skip word chars
                let trimmed = before.trim_end();
                let word_start = trimmed
                    .rfind(|c: char| c.is_whitespace() || c == '/')
                    .map_or(0, |p| p + 1);
                self.input.drain(word_start..self.cursor_pos);
                self.cursor_pos = word_start;
                self.update_completion();
                self.dirty = true;
            }
            // Delete to end of line
            (KeyCode::Char('k'), KeyModifiers::CONTROL) => {
                let after = &self.input[self.cursor_pos..];
                let end = self.cursor_pos + after.find('\n').unwrap_or(after.len());
                self.input.drain(self.cursor_pos..end);
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
            // Paste image from clipboard (text paste handled by Event::Paste)
            (KeyCode::Char('v'), KeyModifiers::CONTROL) => {
                let _ = self.try_paste_image();
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
            // Error log overlay
            (KeyCode::F(4), _) => {
                if self.error_log.is_empty() {
                    self.push_system("No errors logged.");
                } else {
                    self.overlay = Overlay::Errors(self.error_log.clone());
                    self.overlay_scroll = 0;
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
                self.save_history_entry(&text);
                self.history_index = None;
                self.input.clear();
                self.cursor_pos = 0;
                self.input_view_offset = 0;
                self.input_scroll_y = 0;
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
            (KeyCode::Backspace, _) if self.cursor_pos > 0 => {
                let prev = prev_char_boundary(&self.input, self.cursor_pos);
                self.input.drain(prev..self.cursor_pos);
                self.cursor_pos = prev;
                self.update_completion();
                self.dirty = true;
            }
            // Delete
            (KeyCode::Delete, _) if self.cursor_pos < self.input.len() => {
                let next = next_char_boundary(&self.input, self.cursor_pos);
                self.input.drain(self.cursor_pos..next);
                self.update_completion();
                self.dirty = true;
            }
            // Left/Right
            (KeyCode::Left, _) if self.cursor_pos > 0 => {
                self.cursor_pos = prev_char_boundary(&self.input, self.cursor_pos);
                self.dirty = true;
            }
            (KeyCode::Right, _) if self.cursor_pos < self.input.len() => {
                self.cursor_pos = next_char_boundary(&self.input, self.cursor_pos);
                self.dirty = true;
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
                    self.update_completion();
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
            'h' if self.cursor_pos > 0 => {
                self.cursor_pos = prev_char_boundary(&self.input, self.cursor_pos);
                self.dirty = true;
            }
            'l' if self.cursor_pos < self.input.len() => {
                self.cursor_pos = next_char_boundary(&self.input, self.cursor_pos);
                self.dirty = true;
            }
            'x' if self.cursor_pos < self.input.len() => {
                let next = next_char_boundary(&self.input, self.cursor_pos);
                self.input.drain(self.cursor_pos..next);
                self.dirty = true;
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
            if (msg.kind == ChatMessageKind::Tool || msg.kind == ChatMessageKind::Error)
                && let Some(info) = &mut msg.tool_info
            {
                info.collapsed = !info.collapsed;
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
        // Auto-scroll to first match
        if let Some(&msg_idx) = self.search_matches.first() {
            self.scroll_to_message(msg_idx);
        }
    }

    /// Scroll chat so that the message at `msg_idx` is visible.
    fn scroll_to_message(&mut self, msg_idx: usize) {
        if msg_idx >= self.height_cache.len() {
            return;
        }
        // Calculate the row offset of this message from the bottom
        let total = self.total_content_height();
        let msg_top: u16 = self.height_cache[..msg_idx].iter().copied().sum();
        let msg_bottom: u16 = msg_top + self.height_cache[msg_idx];
        // scroll is measured from bottom (0 = at bottom)
        let scroll_to_show = total.saturating_sub(msg_bottom);
        self.chat_scroll = scroll_to_show;
        self.sticky_scroll = scroll_to_show == 0;
        self.dirty = true;
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
        let suggestions = registry.recommend(&self.input, 10);
        if suggestions.is_empty() {
            return Some("Commands: no matches".to_string());
        }
        // When completion popup is active, show selected command in status bar
        if let Some(sel) = self.completion_selected
            && let Some(cmd) = suggestions.get(sel)
        {
            return Some(match cmd.argument_hint {
                Some(hint) => format!("/{} {} — {}", cmd.name, hint, cmd.summary),
                None => format!("/{} — {}", cmd.name, cmd.summary),
            });
        }
        let parts: Vec<String> = suggestions
            .iter()
            .take(4)
            .map(|cmd| match cmd.argument_hint {
                Some(hint) => format!("/{} {}", cmd.name, hint),
                None => format!("/{}", cmd.name),
            })
            .collect();
        Some(format!("Commands: {}", parts.join("  ·  ")))
    }

    /// Get completion suggestions for the current input.
    pub(crate) fn completion_suggestions(&self) -> Vec<(String, String)> {
        if !self.input.trim_start().starts_with('/') {
            return Vec::new();
        }
        let registry = CommandRegistry::with_defaults();
        registry
            .recommend(&self.input, 10)
            .into_iter()
            .map(|cmd| {
                let label = match cmd.argument_hint {
                    Some(hint) => format!("/{} {}", cmd.name, hint),
                    None => format!("/{}", cmd.name),
                };
                (label, cmd.summary.to_string())
            })
            .collect()
    }

    /// Update completion state after input changes.
    fn update_completion(&mut self) {
        if self.input.trim_start().starts_with('/') && !self.input.contains(' ') {
            // Activate completion, reset selection to 0 if we have suggestions
            let has_suggestions = {
                let registry = CommandRegistry::with_defaults();
                !registry.recommend(&self.input, 1).is_empty()
            };
            self.completion_selected = if has_suggestions { Some(0) } else { None };
        } else {
            self.completion_selected = None;
        }
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

    /// Load input history from persistent file.
    pub(crate) fn load_input_history(&mut self) {
        let Some(path) = input_history_path() else {
            return;
        };
        if let Ok(content) = std::fs::read_to_string(&path) {
            self.input_history = content
                .lines()
                .filter(|l| !l.is_empty())
                .map(|l| l.replace("\\n", "\n"))
                .collect();
            // Cap at 500 entries
            if self.input_history.len() > 500 {
                let start = self.input_history.len() - 500;
                self.input_history = self.input_history.split_off(start);
            }
        }
    }

    /// Append a single entry to the persistent history file.
    pub(crate) fn save_history_entry(&self, entry: &str) {
        let Some(path) = input_history_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        else {
            return;
        };
        use std::io::Write;
        // Escape newlines for multi-line inputs
        let escaped = entry.replace('\n', "\\n");
        let _ = writeln!(file, "{escaped}");
    }

    fn handle_paste(&mut self, text: &str) {
        self.input.insert_str(self.cursor_pos, text);
        self.cursor_pos += text.len();
        self.update_completion();
        self.dirty = true;
    }

    /// Try to read an image from the system clipboard and add it to pending_images.
    fn try_paste_image(&mut self) -> bool {
        const MAX_IMAGES: usize = 10;
        const MAX_DIMENSION: usize = 4096;
        const MAX_BYTES: usize = 20 * 1024 * 1024; // 20MB

        // Check if current model supports vision
        let caps = nocode_core::provider::model_caps::lookup(&self.hud.model_name);
        if !caps.supports_vision {
            self.push_system(&format!(
                "Model '{}' does not support images.",
                self.hud.model_name
            ));
            self.dirty = true;
            return true;
        }

        let Ok(mut clipboard) = arboard::Clipboard::new() else {
            return false;
        };
        let Ok(img) = clipboard.get_image() else {
            return false;
        };

        // Validate limits
        if self.pending_images.len() >= MAX_IMAGES {
            self.push_system(&format!(
                "Cannot paste: already {MAX_IMAGES} images attached. Submit or clear first."
            ));
            self.dirty = true;
            return true;
        }

        let width = img.width;
        let height = img.height;

        if width > MAX_DIMENSION || height > MAX_DIMENSION {
            self.push_system(&format!(
                "Image too large ({width}x{height}). Max {MAX_DIMENSION}x{MAX_DIMENSION}."
            ));
            self.dirty = true;
            return true;
        }

        let rgba_bytes = img.bytes;

        // Encode as PNG
        let mut png_data: Vec<u8> = Vec::new();
        if let Err(e) = encode_rgba_to_png(&mut png_data, &rgba_bytes, width, height) {
            self.push_system(&format!("Failed to encode image: {e}"));
            self.dirty = true;
            return true;
        }

        let size_bytes = png_data.len();
        if size_bytes > MAX_BYTES {
            let mb = size_bytes / (1024 * 1024);
            self.push_system(&format!(
                "Image too large ({mb}MB). Max 20MB after encoding."
            ));
            self.dirty = true;
            return true;
        }

        let base64_data = base64::engine::general_purpose::STANDARD.encode(&png_data);

        self.pending_images.push(PendingImage {
            media_type: "image/png".to_string(),
            base64_data,
            size_bytes,
        });

        let count = self.pending_images.len();
        let size_kb = size_bytes / 1024;
        self.push_system(&format!(
            "Image {count}/{MAX_IMAGES} pasted ({width}x{height}, {size_kb}KB). Will be sent with your next message."
        ));
        self.dirty = true;
        true
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_app_loop(
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<io::Stdout>>,
    provider: Box<dyn Provider>,
    registry: ToolRegistry,
    system: Vec<SystemBlock>,
    model: &str,
    max_tokens: u32,
    max_turns: u32,
    warnings: Vec<String>,
) -> io::Result<()> {
    let mut app = TuiApp::new(model);

    // Set provider info on banner from settings
    {
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| ".".to_string());
        let settings = Settings::load_merged(&cwd);
        if let Some(ref provider_str) = settings.model_provider {
            app.banner_info.provider = provider_str.clone();
        } else if settings.custom_base_url.is_some() {
            app.banner_info.provider = "Custom".to_string();
        }
        if let Some(ref mode) = settings.permission_mode {
            app.banner_info.mode = mode.clone();
            app.hud.set_permission_mode(mode);
        }
    }

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

    // Load persistent input history
    app.load_input_history();

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
                ..
            } = checker.check_cached_only()
            {
                app.push_system(&format!(
                    "Update {current} \u{2192} {latest} \u{2014} run /update"
                ));
            }
        }
    }

    // Install worker event channel into global registry
    {
        let (worker_tx, worker_rx) = std::sync::mpsc::channel();
        let reg = nocode_core::agent::worker::global_worker_registry();
        let mut guard = reg.lock().unwrap_or_else(|e| e.into_inner());
        guard.set_event_channel(worker_tx);
        app.worker_event_rx = Some(worker_rx);
    }

    // Display provider warnings (if any remain) as system messages
    if !warnings.is_empty() {
        for w in &warnings {
            app.push_system(w);
        }
    }

    let mut event_rx: Option<mpsc::Receiver<TuiEvent>> = None;
    let mut is_busy = false;

    let _provider: Arc<dyn Provider> = Arc::from(provider);
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
                                crate::tool_render::truncate_str(&partial_json, 120)
                            } else {
                                partial_json
                            };
                            let label = if name.is_empty() { "tool" } else { &name };
                            last.lines = vec![{
                                let mut rl = crate::markdown_render::RenderedLine::new();
                                rl.push(crate::markdown_render::LineSegment::new(
                                    format!("\u{276F} {label} {preview}"),
                                    ratatui::style::Color::DarkGray,
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
                            app.push_error("Model response interrupted. Try submitting again or restart with /clear.");
                            is_busy = false;
                            event_rx = None;
                        }
                        break;
                    }
                }
            }
        }

        // 2b. Config overlay (read-only, no polling needed)

        // 2c. Poll worker events
        {
            use nocode_core::agent::worker::WorkerEvent;
            let mut worker_events = Vec::new();
            if let Some(ref worker_rx) = app.worker_event_rx {
                loop {
                    match worker_rx.try_recv() {
                        Ok(event) => worker_events.push(event),
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                    }
                }
            }
            for event in worker_events {
                match event {
                    WorkerEvent::Finished {
                        worker_id,
                        name,
                        result,
                    } => {
                        let preview = crate::tool_render::truncate_str(&result, 500);
                        app.push_system(&format!(
                            "Agent '{name}' ({worker_id}) finished:\n{preview}"
                        ));
                    }
                    WorkerEvent::Failed {
                        worker_id,
                        name,
                        error,
                    } => {
                        app.push_error(&format!("Agent '{name}' ({worker_id}) failed: {error}"));
                    }
                    WorkerEvent::TimedOut { worker_id, name } => {
                        app.push_error(&format!(
                            "Agent '{name}' ({worker_id}) timed out and was cancelled"
                        ));
                    }
                    WorkerEvent::StateChanged { .. } => {
                        app.dirty = true;
                    }
                }
            }
        }

        // 2d. Check worker timeouts
        {
            let reg = nocode_core::agent::worker::global_worker_registry();
            let mut guard = reg.lock().unwrap_or_else(|e| e.into_inner());
            let _ = guard.check_timeouts();
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

                                // Build message with text + any pending images
                                let mut blocks: Vec<ContentBlock> = Vec::new();
                                for img in app.pending_images.drain(..) {
                                    blocks
                                        .push(ContentBlock::image(img.media_type, img.base64_data));
                                }
                                blocks.push(ContentBlock::text(&text));
                                messages.push(Message::user(blocks));

                                app.hud.start_turn();
                                app.thinking_spinner = Some(Spinner::new("Thinking..."));
                                is_busy = true;

                                // Launch background thread
                                let (tx, rx) = mpsc::channel();
                                event_rx = Some(rx);

                                let r = registry_slot.take().expect("registry available");
                                let msgs = messages.clone();
                                let current_model = app.hud.model_name.clone();
                                let cfg = LoopConfig {
                                    model: current_model.clone(),
                                    max_tokens,
                                    max_turns,
                                    system: system.clone(),
                                    tools: tool_defs.clone(),
                                    parallel_tool_execution: true,
                                    reasoning_effort: None,
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
                                    let cwd = std::env::current_dir()
                                        .map(|p| p.to_string_lossy().into_owned())
                                        .unwrap_or_default();
                                    let settings = Settings::load_merged(&cwd);
                                    let provider_type = crate::resolve_provider(&settings);
                                    let (provider, _) =
                                        crate::build_provider(&provider_type, &settings);
                                    let executor =
                                        ToolExecutor::new(&r).with_prompter(&perm_bridge);
                                    let mut observer = ChannelObserver { tx };
                                    let result = r#loop::run_agentic_loop(
                                        provider.as_ref(),
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
                Event::Paste(text) if !is_busy => {
                    app.handle_paste(&text);
                }
                Event::Mouse(mouse) => {
                    use crossterm::event::{MouseEvent, MouseEventKind};
                    match mouse {
                        MouseEvent {
                            kind: MouseEventKind::ScrollUp,
                            ..
                        } => {
                            app.chat_scroll = app.chat_scroll.saturating_add(3);
                            app.sticky_scroll = false;
                            app.dirty = true;
                        }
                        MouseEvent {
                            kind: MouseEventKind::ScrollDown,
                            ..
                        } => {
                            app.chat_scroll = app.chat_scroll.saturating_sub(3);
                            if app.chat_scroll == 0 {
                                app.sticky_scroll = true;
                                app.unseen_count = 0;
                            }
                            app.dirty = true;
                        }
                        _ => {}
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

    // Show resume hint if session has messages
    if !messages.is_empty() && !app.hud.session_id.is_empty() {
        eprintln!(
            "\n  To resume this session: nocode --resume {}\n",
            app.hud.session_id
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn highlight_search_in_line<'a>(
    line: ratatui::text::Line<'a>,
    query: &str,
) -> ratatui::text::Line<'a> {
    use ratatui::style::{Color, Modifier};
    use ratatui::text::Span;
    if query.is_empty() {
        return line;
    }
    let query_lower = query.to_lowercase();
    let mut new_spans = Vec::new();
    for span in line.spans {
        let text = span.content.to_string();
        let text_lower = text.to_lowercase();
        let base_style = span.style;
        let hl_style = base_style
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD);
        let mut start = 0;
        while let Some(pos) = text_lower[start..].find(&query_lower) {
            let abs = start + pos;
            if abs > start {
                new_spans.push(Span::styled(text[start..abs].to_string(), base_style));
            }
            let end = abs + query.len();
            let end = end.min(text.len());
            new_spans.push(Span::styled(text[abs..end].to_string(), hl_style));
            start = end;
        }
        if start < text.len() {
            new_spans.push(Span::styled(text[start..].to_string(), base_style));
        } else if start == 0 && text.is_empty() {
            new_spans.push(span);
        }
    }
    ratatui::text::Line::from(new_spans)
}

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

#[allow(dead_code)]
fn display_width_of(s: &str) -> usize {
    s.chars().map(|c| c.width().unwrap_or(0)).sum()
}

fn next_word_boundary(s: &str, pos: usize) -> usize {
    let mut p = pos;
    // Skip current word chars (non-whitespace)
    for c in s[pos..].chars() {
        if c.is_whitespace() {
            break;
        }
        p += c.len_utf8();
    }
    // Skip whitespace
    for c in s[p..].chars() {
        if !c.is_whitespace() {
            break;
        }
        p += c.len_utf8();
    }
    p.min(s.len())
}

fn prev_word_boundary(s: &str, pos: usize) -> usize {
    if pos == 0 {
        return 0;
    }
    // Collect char boundaries up to pos
    let mut boundaries: Vec<usize> = s.char_indices().map(|(i, _)| i).collect();
    boundaries.push(s.len());
    // Find current char index
    let char_idx = boundaries
        .iter()
        .position(|&b| b >= pos)
        .unwrap_or(boundaries.len());
    let mut ci = char_idx.saturating_sub(1);
    // Skip whitespace backwards
    while ci > 0 {
        let ch = s[boundaries[ci]..].chars().next().unwrap_or(' ');
        if !ch.is_whitespace() {
            break;
        }
        ci -= 1;
    }
    // Skip word chars backwards
    while ci > 0 {
        let prev_ch = s[boundaries[ci - 1]..].chars().next().unwrap_or(' ');
        if prev_ch.is_whitespace() {
            break;
        }
        ci -= 1;
    }
    boundaries.get(ci).copied().unwrap_or(0)
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

    Err("Clipboard not available. Install xclip, xsel, or wl-copy.".to_string())
}

/// Truncate a string to at most `max` bytes on a valid UTF-8 boundary.
fn safe_truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut idx = max;
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    &s[..idx]
}

/// Truncate a string with "..." suffix if it exceeds `max` bytes.
fn safe_truncate_ellipsis(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut idx = max.saturating_sub(3);
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    format!("{}...", &s[..idx])
}

/// Path to persistent input history file.
fn input_history_path() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(
        std::path::PathBuf::from(home)
            .join(".nocode")
            .join("input_history.txt"),
    )
}

/// Encode RGBA pixel data to PNG format.
fn encode_rgba_to_png(
    out: &mut Vec<u8>,
    rgba: &[u8],
    width: usize,
    height: usize,
) -> Result<(), String> {
    let mut encoder = png::Encoder::new(std::io::Cursor::new(out), width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|e| format!("PNG header: {e}"))?;
    writer
        .write_image_data(rgba)
        .map_err(|e| format!("PNG data: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // detect_preset
    // -----------------------------------------------------------------------

    #[test]
    fn detect_preset_exact_match() {
        // Big 3
        assert_eq!(detect_preset("https://api.anthropic.com"), Some(0)); // Anthropic
        assert_eq!(detect_preset("https://api.openai.com"), Some(1)); // OpenAI
        assert_eq!(
            detect_preset("https://generativelanguage.googleapis.com"),
            Some(2),
        ); // Gemini
        // Custom presets (offset +3)
        assert_eq!(detect_preset("https://openrouter.ai/api/v1"), Some(3)); // OpenRouter
        assert_eq!(detect_preset("https://api.together.xyz/v1"), Some(4)); // Together
        assert_eq!(detect_preset("https://api.groq.com/openai/v1"), Some(5)); // Groq
        assert_eq!(
            detect_preset("https://api.fireworks.ai/inference/v1"),
            Some(6)
        ); // Fireworks
        assert_eq!(detect_preset("https://api.deepseek.com/v1"), Some(7)); // DeepSeek
        assert_eq!(detect_preset("https://api.mistral.ai/v1"), Some(8)); // Mistral
        assert_eq!(detect_preset("http://localhost:11434/v1"), Some(9)); // Ollama
        assert_eq!(detect_preset("http://localhost:8000/v1"), Some(10)); // vLLM
        assert_eq!(detect_preset("http://localhost:4000/v1"), Some(11)); // LiteLLM
        assert_eq!(detect_preset("http://localhost:8080/v1"), Some(12)); // LocalAI
        assert_eq!(detect_preset("http://localhost:1234/v1"), Some(13)); // LM Studio
    }

    #[test]
    fn detect_preset_trailing_slash() {
        assert_eq!(detect_preset("https://openrouter.ai/api/v1/"), Some(3));
        assert_eq!(detect_preset("http://localhost:11434/v1/"), Some(9));
    }

    #[test]
    fn detect_preset_whitespace() {
        assert_eq!(detect_preset("  https://openrouter.ai/api/v1  "), Some(3));
        assert_eq!(detect_preset("\thttp://localhost:8000/v1\n"), Some(10));
    }

    #[test]
    fn detect_preset_case_insensitive() {
        assert_eq!(detect_preset("HTTPS://OPENROUTER.AI/API/V1"), Some(3));
        assert_eq!(detect_preset("HTTP://LOCALHOST:11434/V1"), Some(9));
    }

    #[test]
    fn detect_preset_no_match() {
        assert_eq!(detect_preset("http://localhost:9999/v1"), None);
        assert_eq!(detect_preset("https://api.openai.com/v1"), None);
        assert_eq!(detect_preset(""), None);
        assert_eq!(detect_preset("  "), None);
    }

    // -----------------------------------------------------------------------
    // preset_label
    // -----------------------------------------------------------------------

    #[test]
    fn preset_label_valid_indices() {
        assert_eq!(preset_label(Some(0)), "Anthropic");
        assert_eq!(preset_label(Some(1)), "OpenAI");
        assert_eq!(preset_label(Some(2)), "Gemini");
        assert_eq!(preset_label(Some(3)), "OpenRouter");
        assert_eq!(preset_label(Some(4)), "Together");
        assert_eq!(preset_label(Some(9)), "Ollama");
        assert_eq!(preset_label(Some(10)), "vLLM");
        assert_eq!(preset_label(Some(13)), "LM Studio");
    }

    #[test]
    fn preset_label_manual_cases() {
        assert_eq!(preset_label(None), "Manual");
        assert_eq!(preset_label(Some(999)), "Manual");
        assert_eq!(preset_label(Some(usize::MAX)), "Manual");
    }

    // -----------------------------------------------------------------------
    // ALL_PRESETS invariants
    // -----------------------------------------------------------------------

    #[test]
    fn presets_have_unique_urls() {
        let urls: Vec<&str> = ALL_PRESETS.iter().map(|p| p.base_url).collect();
        for (i, url) in urls.iter().enumerate() {
            for (j, other) in urls.iter().enumerate() {
                if i != j {
                    assert_ne!(
                        url.to_ascii_lowercase(),
                        other.to_ascii_lowercase(),
                        "Duplicate preset URL: {url}"
                    );
                }
            }
        }
    }

    #[test]
    fn presets_have_valid_api_format() {
        let valid = nocode_core::config::settings::API_FORMATS;
        for preset in ALL_PRESETS {
            assert!(
                valid.contains(&preset.api_format),
                "Invalid api_format '{}' in preset '{}'",
                preset.api_format,
                preset.name
            );
        }
    }

    #[test]
    fn presets_not_empty() {
        assert!(!ALL_PRESETS.is_empty(), "Preset list must not be empty");
        for preset in ALL_PRESETS {
            assert!(!preset.name.is_empty(), "Preset name must not be empty");
            assert!(
                !preset.base_url.is_empty(),
                "Preset base_url must not be empty"
            );
            assert!(
                !preset.auth_hint.is_empty(),
                "Preset auth_hint must not be empty"
            );
        }
    }

    #[test]
    fn detect_preset_roundtrip() {
        // Every preset's own URL should be detected back to its index.
        for (i, preset) in ALL_PRESETS.iter().enumerate() {
            assert_eq!(
                detect_preset(preset.base_url),
                Some(i),
                "Preset '{}' at index {} failed roundtrip",
                preset.name,
                i
            );
        }
    }
}
