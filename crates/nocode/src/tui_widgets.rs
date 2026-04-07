//! Custom ratatui widgets for the nocode TUI.

use crate::markdown_render::{LineSegment, RenderedLine};
use crate::tui_theme::default_theme;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap};

// ---------------------------------------------------------------------------
// Color bridge: crossterm::style::Color → ratatui::style::Color
// ---------------------------------------------------------------------------

pub(crate) fn convert_color(c: crossterm::style::Color) -> Color {
    match c {
        crossterm::style::Color::Rgb { r, g, b } => Color::Rgb(r, g, b),
        crossterm::style::Color::Red => Color::Red,
        crossterm::style::Color::Green => Color::Green,
        crossterm::style::Color::Blue => Color::Blue,
        crossterm::style::Color::Yellow => Color::Yellow,
        crossterm::style::Color::Cyan => Color::Cyan,
        crossterm::style::Color::Magenta => Color::Magenta,
        crossterm::style::Color::White => Color::White,
        crossterm::style::Color::DarkGrey => Color::DarkGray,
        crossterm::style::Color::DarkYellow => Color::Yellow,
        crossterm::style::Color::Grey => Color::Gray,
        crossterm::style::Color::DarkRed => Color::LightRed,
        crossterm::style::Color::DarkGreen => Color::DarkGray,
        crossterm::style::Color::DarkBlue => Color::Blue,
        crossterm::style::Color::DarkMagenta => Color::Magenta,
        crossterm::style::Color::DarkCyan => Color::DarkGray,
        crossterm::style::Color::Black => Color::Black,
        _ => Color::White,
    }
}

// ---------------------------------------------------------------------------
// ChatMessage
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChatMessageKind {
    System,
    User,
    Assistant,
    Error,
    Tool,
    Spinner,
}

impl ChatMessageKind {
    fn badge(&self) -> (&str, Color, Color) {
        let theme = default_theme();
        match self {
            Self::System => ("SYS", theme.badge_system_bg, theme.badge_system_fg),
            Self::User => ("YOU", theme.badge_user_bg, theme.badge_user_fg),
            Self::Assistant => ("AST", theme.badge_assistant_bg, theme.badge_assistant_fg),
            Self::Error => ("ERR", theme.badge_error_bg, theme.badge_error_fg),
            Self::Tool => ("TUL", theme.badge_tool_bg, theme.badge_tool_fg),
            Self::Spinner => ("...", theme.system, theme.text_dim),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ChatMessage {
    pub kind: ChatMessageKind,
    pub lines: Vec<RenderedLine>,
    /// Optional timestamp for display (formatted string, e.g. "14:32:05").
    pub timestamp: Option<String>,
    /// For Tool messages: structured tool call info.
    pub tool_info: Option<ToolCallInfo>,
}

/// Structured tool call information for rendering.
#[derive(Debug, Clone)]
pub(crate) struct ToolCallInfo {
    pub tool_name: String,
    pub arguments_summary: String,
    pub result_preview: Option<String>,
    pub collapsed: bool,
}

impl ToolCallInfo {
    #[allow(dead_code)]
    pub fn new(tool_name: &str, arguments_summary: &str) -> Self {
        Self {
            tool_name: tool_name.to_owned(),
            arguments_summary: arguments_summary.to_owned(),
            result_preview: None,
            collapsed: true,
        }
    }

    #[allow(dead_code)]
    pub fn with_result(mut self, result: &str) -> Self {
        self.result_preview = Some(result.to_owned());
        self
    }
}

impl ChatMessage {
    pub fn new(kind: ChatMessageKind, lines: Vec<RenderedLine>) -> Self {
        Self {
            kind,
            lines,
            timestamp: Some(current_timestamp()),
            tool_info: None,
        }
    }

    pub fn plain(kind: ChatMessageKind, text: &str) -> Self {
        let lines = text
            .lines()
            .map(|l| {
                let mut rl = RenderedLine::new();
                rl.push(LineSegment::new(l, crossterm::style::Color::White));
                rl
            })
            .collect();
        Self {
            kind,
            lines,
            timestamp: Some(current_timestamp()),
            tool_info: None,
        }
    }

    /// Create a tool call message with structured info.
    #[allow(dead_code)]
    pub fn tool_call(tool_info: ToolCallInfo, result_lines: Vec<RenderedLine>) -> Self {
        Self {
            kind: ChatMessageKind::Tool,
            lines: result_lines,
            timestamp: Some(current_timestamp()),
            tool_info: Some(tool_info),
        }
    }

    /// Convert to ratatui Lines for rendering.
    pub fn to_ratatui_lines(&self) -> Vec<Line<'_>> {
        let theme = default_theme();
        let (badge, badge_bg, badge_fg) = self.kind.badge();
        let mut result = Vec::with_capacity(self.lines.len() + 4);

        // Message background color based on kind
        let msg_bg = match self.kind {
            ChatMessageKind::User => theme.user_msg_bg,
            ChatMessageKind::Error => theme.error_msg_bg,
            ChatMessageKind::Assistant => theme.assistant_msg_bg,
            _ => Color::Reset,
        };

        // Badge line with optional timestamp
        if !matches!(self.kind, ChatMessageKind::Spinner) {
            let mut badge_spans = vec![Span::styled(
                format!(" {badge} "),
                Style::default().fg(badge_fg).bg(badge_bg),
            )];
            if let Some(ts) = &self.timestamp {
                badge_spans.push(Span::styled(
                    format!(" {ts}"),
                    Style::default().fg(theme.text_dim),
                ));
            }
            result.push(Line::from(badge_spans));
        }

        // Tool call header (structured rendering)
        if let Some(info) = &self.tool_info {
            let header = format!(
                " {} {}",
                if info.collapsed {
                    "\u{25B6}"
                } else {
                    "\u{25BC}"
                },
                info.tool_name
            );
            result.push(Line::from(vec![
                Span::styled(
                    header,
                    Style::default().fg(theme.tool).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" {}", info.arguments_summary),
                    Style::default().fg(theme.text_dim),
                ),
            ]));
        }

        // Content lines with message background
        let content_style_base = if msg_bg == Color::Reset {
            Style::default()
        } else {
            Style::default().bg(msg_bg)
        };

        // For collapsed tool calls with result, show only first line
        let show_content = if let Some(info) = &self.tool_info {
            !info.collapsed || info.result_preview.is_none()
        } else {
            true
        };

        if show_content {
            for rendered_line in &self.lines {
                let spans: Vec<Span<'_>> = rendered_line
                    .segments
                    .iter()
                    .map(|seg| {
                        let mut style = content_style_base.fg(convert_color(seg.color));
                        if seg.bold {
                            style = style.add_modifier(Modifier::BOLD);
                        }
                        if seg.italic {
                            style = style.add_modifier(Modifier::ITALIC);
                        }
                        Span::styled(seg.text.as_str(), style)
                    })
                    .collect();
                result.push(Line::from(spans));
            }
        } else if let Some(info) = &self.tool_info
            && let Some(preview) = &info.result_preview
        {
            result.push(Line::from(Span::styled(
                preview.as_str(),
                Style::default().fg(theme.text_dim),
            )));
        }

        result
    }

    /// Estimate height in terminal rows at given width.
    #[allow(dead_code)]
    pub fn height(&self, _width: u16) -> u16 {
        let badge_line = if matches!(self.kind, ChatMessageKind::Spinner) {
            0
        } else {
            1
        };
        (self.lines.len() as u16).saturating_add(badge_line).max(1)
    }
}

/// Generate a timestamp string for the current local time (HH:MM:SS).
/// Uses libc localtime to avoid adding chrono as a dependency.
fn current_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Simple UTC-based HH:MM:SS — good enough for a TUI timestamp.
    let h = (secs / 3600) % 24;
    let m = (secs / 60) % 60;
    let s = secs % 60;
    format!("{h:02}:{m:02}:{s:02}")
}

// ---------------------------------------------------------------------------
// InputBox
// ---------------------------------------------------------------------------

pub(crate) struct InputBox<'a> {
    pub input: &'a str,
    #[allow(dead_code)]
    pub cursor_pos: usize,
    pub prompt: &'a str,
}

impl<'a> InputBox<'a> {
    pub fn new(input: &'a str, cursor_pos: usize, prompt: &'a str) -> Self {
        Self {
            input,
            cursor_pos,
            prompt,
        }
    }
}

impl Widget for InputBox<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = default_theme();
        let block = Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(theme.input_border))
            .title(" input ")
            .title_style(Style::default().fg(theme.input_border));
        let inner = block.inner(area);
        block.render(area, buf);

        let prompt_span = Span::styled(self.prompt, Style::default().fg(theme.text_dim));
        let input_span = Span::styled(self.input, Style::default().fg(theme.text));
        let paragraph = Paragraph::new(Line::from(vec![prompt_span, input_span]));
        paragraph.render(inner, buf);
    }
}

// ---------------------------------------------------------------------------
// StatusBar
// ---------------------------------------------------------------------------

pub(crate) struct StatusBar<'a> {
    pub content: &'a str,
}

impl<'a> StatusBar<'a> {
    pub fn new(content: &'a str) -> Self {
        Self { content }
    }
}

impl Widget for StatusBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = default_theme();
        let style = Style::default()
            .fg(theme.status_bar_fg)
            .bg(theme.status_bar_bg);
        // Fill entire area with background
        for x in area.left()..area.right() {
            buf[(x, area.top())].set_char(' ').set_style(style);
        }
        let paragraph = Paragraph::new(format!("  {}", self.content)).style(style);
        paragraph.render(area, buf);
    }
}

// ---------------------------------------------------------------------------
// OverlayBlock
// ---------------------------------------------------------------------------

pub(crate) struct OverlayBlock<'a> {
    pub title: &'a str,
    pub body: &'a str,
}

impl<'a> OverlayBlock<'a> {
    pub fn new(title: &'a str, body: &'a str) -> Self {
        Self { title, body }
    }
}

impl Widget for OverlayBlock<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = default_theme();
        // Center the overlay at 80% width, 60% height
        let w = (area.width * 4 / 5).max(20);
        let h = (area.height * 3 / 5).max(8);
        let x = area.x + (area.width.saturating_sub(w)) / 2;
        let y = area.y + (area.height.saturating_sub(h)) / 2;
        let overlay_area = Rect::new(x, y, w, h);

        Clear.render(overlay_area, buf);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.claude))
            .title(format!(" {} ", self.title))
            .title_style(
                Style::default()
                    .fg(theme.claude)
                    .add_modifier(Modifier::BOLD),
            );
        let inner = block.inner(overlay_area);
        block.render(overlay_area, buf);

        let paragraph = Paragraph::new(self.body)
            .style(Style::default().fg(theme.text))
            .wrap(Wrap { trim: false });
        paragraph.render(inner, buf);
    }
}
