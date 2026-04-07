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
    /// Returns (label, foreground_color) — no background block, just colored text.
    fn badge(&self) -> (&str, Color) {
        let theme = default_theme();
        match self {
            Self::System => ("system", theme.system),
            Self::User => ("you", theme.user),
            Self::Assistant => ("assistant", theme.assistant),
            Self::Error => ("error", theme.error),
            Self::Tool => ("tool", theme.tool),
            Self::Spinner => ("", theme.text_dim),
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
        let (badge, badge_fg) = self.kind.badge();
        let mut result = Vec::with_capacity(self.lines.len() + 3);

        // Badge line: "  assistant  14:32:05" — no background block, just colored text
        if !matches!(self.kind, ChatMessageKind::Spinner) {
            let mut badge_spans = vec![
                Span::raw("  "),
                Span::styled(
                    badge,
                    Style::default().fg(badge_fg).add_modifier(Modifier::BOLD),
                ),
            ];
            if let Some(ts) = &self.timestamp {
                badge_spans.push(Span::styled(
                    format!("  {ts}"),
                    Style::default().fg(theme.text_dim),
                ));
            }
            result.push(Line::from(badge_spans));
        }

        // Tool call header (structured rendering)
        if let Some(info) = &self.tool_info {
            let arrow = if info.collapsed { "\u{25B8}" } else { "\u{25BE}" };
            result.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("{arrow} {}", info.tool_name),
                    Style::default().fg(theme.tool),
                ),
                Span::styled(
                    format!(" {}", info.arguments_summary),
                    Style::default().fg(theme.text_dim),
                ),
            ]));
        }

        // Content lines — 4-char left padding, no background color blocks
        let show_content = if let Some(info) = &self.tool_info {
            !info.collapsed || info.result_preview.is_none()
        } else {
            true
        };

        if show_content {
            for rendered_line in &self.lines {
                let mut spans: Vec<Span<'_>> = vec![Span::raw("    ")];
                spans.extend(rendered_line.segments.iter().map(|seg| {
                    let mut style = Style::default().fg(convert_color(seg.color));
                    if seg.bold {
                        style = style.add_modifier(Modifier::BOLD);
                    }
                    if seg.italic {
                        style = style.add_modifier(Modifier::ITALIC);
                    }
                    Span::styled(seg.text.as_str(), style)
                }));
                result.push(Line::from(spans));
            }
        } else if let Some(info) = &self.tool_info
            && let Some(preview) = &info.result_preview
        {
            result.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(preview.as_str(), Style::default().fg(theme.text_dim)),
            ]));
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
}

impl<'a> InputBox<'a> {
    pub fn new(input: &'a str, cursor_pos: usize) -> Self {
        Self { input, cursor_pos }
    }
}

impl Widget for InputBox<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = default_theme();
        // Simple "> input_text" on a single line, no borders
        let prompt_span = Span::styled("> ", Style::default().fg(theme.text_dim));
        let input_span = Span::styled(self.input, Style::default().fg(theme.text));
        let paragraph = Paragraph::new(Line::from(vec![prompt_span, input_span]));
        paragraph.render(area, buf);
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
        let style = Style::default().fg(theme.text_dim);
        // Thin dim separator line with content
        let sep = "\u{2500}"; // ─
        let sep_line: String = sep.repeat(area.width as usize);
        // Render separator as background
        let paragraph = Paragraph::new(sep_line).style(Style::default().fg(theme.border));
        paragraph.render(area, buf);
        // Overlay the status content centered-ish
        if !self.content.is_empty() {
            let content = format!(" {} ", self.content);
            let content_len = content.len() as u16;
            let x = area.x + 1;
            let max_w = area.width.saturating_sub(2);
            let w = content_len.min(max_w);
            if w > 0 {
                let status_area = Rect::new(x, area.y, w, 1);
                let status_p = Paragraph::new(content).style(style);
                status_p.render(status_area, buf);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// HintsBar — contextual keyboard shortcuts
// ---------------------------------------------------------------------------

pub(crate) struct HintsBar;

impl Widget for HintsBar {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = default_theme();
        let dim = Style::default().fg(theme.text_dim);
        let hints = " enter send \u{00B7} /help commands \u{00B7} ctrl-c quit";
        let paragraph = Paragraph::new(hints).style(dim);
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

// ---------------------------------------------------------------------------
// WelcomeBanner — startup splash with ASCII logo + system info
// ---------------------------------------------------------------------------

/// System info passed to the welcome banner for display.
pub(crate) struct WelcomeBannerInfo {
    pub model: String,
    pub mode: String,
    pub cwd: String,
    pub version: String,
    pub username: String,
}

impl Default for WelcomeBannerInfo {
    fn default() -> Self {
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "unknown".into());
        let username = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_default();
        Self {
            model: String::from("pending"),
            mode: String::from("default"),
            cwd,
            version: String::from(env!("CARGO_PKG_VERSION")),
            username,
        }
    }
}

pub(crate) struct WelcomeBanner<'a> {
    pub info: &'a WelcomeBannerInfo,
}

impl<'a> WelcomeBanner<'a> {
    pub fn new(info: &'a WelcomeBannerInfo) -> Self {
        Self { info }
    }
}

const LOGO: [&str; 5] = [
    r"  ╔╗╔╔═╗╔═╗╔═╗╔╦╗╔═╗ ",
    r"  ║║║║ ║║  ║ ║ ║║║╣   ",
    r"  ╝╚╝╚═╝╚═╝╚═╝═╩╝╚═╝ ",
    r"                       ",
    r"  terminal ai assistant",
];

impl Widget for WelcomeBanner<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = default_theme();

        let box_w = area.width.clamp(30, 60);
        let box_h: u16 = 12;
        let x = area.x + (area.width.saturating_sub(box_w)) / 2;
        let y = area.y + (area.height.saturating_sub(box_h)) / 3;
        let banner_area = Rect::new(x, y, box_w, box_h.min(area.height));

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(Style::default().fg(theme.border));
        let inner = block.inner(banner_area);
        block.render(banner_area, buf);

        if inner.height == 0 || inner.width == 0 {
            return;
        }

        let mut lines: Vec<Line<'_>> = Vec::with_capacity(12);

        for logo_line in &LOGO {
            lines.push(Line::from(Span::styled(
                *logo_line,
                Style::default().fg(theme.claude),
            )));
        }

        let greeting = if self.info.username.is_empty() {
            String::from("  Welcome back!")
        } else {
            format!("  Welcome back, {}!", self.info.username)
        };
        lines.push(Line::from(Span::styled(
            greeting,
            Style::default()
                .fg(theme.text)
                .add_modifier(Modifier::BOLD),
        )));

        lines.push(Line::from(""));

        let model_line = format!(
            "  {} \u{00B7} {}",
            self.info.model, self.info.mode
        );
        lines.push(Line::from(Span::styled(
            model_line,
            Style::default().fg(theme.text_dim),
        )));

        let max_path = (inner.width as usize).saturating_sub(4);
        let cwd_display = if self.info.cwd.len() > max_path && max_path > 10 {
            let half = (max_path - 3) / 2;
            format!(
                "  {}...{}",
                &self.info.cwd[..half],
                &self.info.cwd[self.info.cwd.len() - half..]
            )
        } else {
            format!("  {}", self.info.cwd)
        };
        lines.push(Line::from(Span::styled(
            cwd_display,
            Style::default().fg(theme.text_dim),
        )));

        lines.push(Line::from(Span::styled(
            format!("  v{}", self.info.version),
            Style::default().fg(theme.text_inactive),
        )));

        let paragraph = Paragraph::new(lines);
        paragraph.render(inner, buf);
    }
}
