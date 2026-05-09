//! Custom ratatui widgets for the nocode TUI.

use crate::markdown_render::{LineSegment, RenderedLine};
use crate::tui_theme::default_theme;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap};

/// Detect diff lines and return appropriate color, or None for non-diff lines.
fn diff_line_color(line: &str, theme: &crate::tui_theme::Theme) -> Option<Color> {
    if line.starts_with("+++") || line.starts_with("---") {
        Some(theme.text_dim)
    } else if line.starts_with('+') {
        Some(theme.diff_added)
    } else if line.starts_with('-') {
        Some(theme.diff_removed)
    } else if line.starts_with("@@") {
        Some(theme.assistant)
    } else {
        None
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
    /// Inline permission prompt — waiting for user y/n/a.
    Permission,
    /// Thinking/reasoning block — collapsible with Ctrl-O.
    Thinking,
    /// Inline plan choice card — user picks an option.
    PlanChoice,
}

impl ChatMessageKind {
    /// Claude Code style prefixes.
    fn prefix(&self) -> (&str, Color) {
        let theme = default_theme();
        match self {
            Self::User => ("\u{276F} ", theme.user),          // ❯
            Self::Assistant => ("  ", theme.assistant),       // just indent
            Self::System => ("\u{2022} ", theme.text_dim),    // • in dim
            Self::Error => ("\u{2716} ", theme.error),        // ✖
            Self::Tool => ("\u{25CF} ", theme.tool),          // ● (Phase 3 will change)
            Self::Spinner => ("  ", theme.spinner),           // just indent
            Self::Permission => ("\u{26A0} ", theme.warning), // ⚠
            Self::Thinking => ("  ", theme.text_dim),         // just indent
            Self::PlanChoice => ("  ", theme.claude),          // just indent
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ChatMessage {
    pub kind: ChatMessageKind,
    pub lines: Vec<RenderedLine>,
    /// For Tool messages: structured tool call info.
    pub tool_info: Option<ToolCallInfo>,
    /// For PlanChoice messages: inline choice card.
    pub plan_choice: Option<PlanChoiceInfo>,
    /// For Thinking messages: whether content is collapsed.
    pub thinking_collapsed: bool,
}

/// Structured tool call information for rendering.
#[derive(Debug, Clone)]
pub(crate) struct ToolCallInfo {
    pub tool_name: String,
    pub arguments_summary: String,
    pub result_preview: Option<String>,
    pub collapsed: bool,
}

/// Inline plan choice card rendered in the chat message flow.
#[derive(Debug, Clone)]
pub(crate) struct PlanChoiceInfo {
    pub header: String,
    pub body_lines: Vec<RenderedLine>,
    pub options: Vec<PlanChoiceOption>,
    pub selected: usize,
    pub resolved: bool,
    pub chosen: Option<usize>,
}

#[derive(Debug, Clone)]
pub(crate) struct PlanChoiceOption {
    pub label: String,
    pub description: String,
    pub key_hint: String,
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
            tool_info: None,
            plan_choice: None,
            thinking_collapsed: true,
        }
    }

    pub fn plain(kind: ChatMessageKind, text: &str) -> Self {
        let lines = text
            .lines()
            .map(|l| {
                let mut rl = RenderedLine::new();
                rl.push(LineSegment::new(l, Color::White));
                rl
            })
            .collect();
        Self {
            kind,
            lines,
            tool_info: None,
            plan_choice: None,
            thinking_collapsed: true,
        }
    }

    /// Create a tool call message with structured info.
    pub fn tool_call(tool_info: ToolCallInfo, result_lines: Vec<RenderedLine>) -> Self {
        Self {
            kind: ChatMessageKind::Tool,
            lines: result_lines,
            tool_info: Some(tool_info),
            plan_choice: None,
            thinking_collapsed: true,
        }
    }

    /// Create an inline plan choice card.
    pub fn plan_choice(info: PlanChoiceInfo) -> Self {
        Self {
            kind: ChatMessageKind::PlanChoice,
            lines: Vec::new(),
            tool_info: None,
            plan_choice: Some(info),
            thinking_collapsed: true,
        }
    }

    /// Convert to ratatui Lines — Claude Code visual language.
    pub fn to_ratatui_lines(&self) -> Vec<Line<'_>> {
        let theme = default_theme();
        let (prefix, prefix_color) = self.kind.prefix();
        let mut result = Vec::with_capacity(self.lines.len() + 3);

        // Plan choice card: ╭── header ──╮ / │ body / │ options / ╰──╯
        if let Some(choice) = &self.plan_choice {
            let border_color = if choice.resolved {
                theme.success
            } else {
                theme.claude
            };

            // Header: ╭── title ──
            result.push(Line::from(vec![
                Span::styled(
                    "  \u{256D}\u{2500}\u{2500} ",
                    Style::default().fg(border_color),
                ),
                Span::styled(
                    choice.header.as_str(),
                    Style::default()
                        .fg(theme.text)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" \u{2500}\u{2500}", Style::default().fg(border_color)),
            ]));

            if choice.resolved {
                let chosen_label = choice
                    .chosen
                    .and_then(|i| choice.options.get(i))
                    .map(|o| o.label.as_str())
                    .unwrap_or("?");
                result.push(Line::from(vec![
                    Span::styled(
                        "  \u{2570}\u{2500} ",
                        Style::default().fg(border_color),
                    ),
                    Span::styled(
                        "\u{2713} ",
                        Style::default().fg(theme.success),
                    ),
                    Span::styled(
                        format!("Chose: {chosen_label}"),
                        Style::default().fg(theme.text_dim),
                    ),
                    Span::styled(
                        " \u{2500}\u{256F}",
                        Style::default().fg(border_color),
                    ),
                ]));
            } else {
                // Body lines (spec preview)
                for line in &choice.body_lines {
                    let mut spans: Vec<Span<'_>> = vec![Span::styled(
                        "  \u{2502} ",
                        Style::default().fg(border_color),
                    )];
                    spans.extend(line.segments.iter().map(|seg| {
                        Span::styled(seg.text.as_str(), Style::default().fg(seg.color))
                    }));
                    result.push(Line::from(spans));
                }

                // Separator before options
                if !choice.body_lines.is_empty() {
                    result.push(Line::from(vec![
                        Span::styled(
                            "  \u{2502} ",
                            Style::default().fg(border_color),
                        ),
                        Span::styled(
                            "\u{2504}".repeat(40),
                            Style::default().fg(theme.border),
                        ),
                    ]));
                }

                // Option rows
                for (i, opt) in choice.options.iter().enumerate() {
                    let marker = if i == choice.selected {
                        "\u{25B8}"
                    } else {
                        " "
                    };
                    let key_style = if i == choice.selected {
                        Style::default()
                            .fg(theme.claude)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme.text_dim)
                    };
                    let label_style = if i == choice.selected {
                        Style::default()
                            .fg(theme.text)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme.text)
                    };
                    result.push(Line::from(vec![
                        Span::styled(
                            "  \u{2502} ",
                            Style::default().fg(border_color),
                        ),
                        Span::styled(
                            format!("{marker} "),
                            key_style,
                        ),
                        Span::styled(
                            format!("[{}] ", opt.key_hint),
                            key_style,
                        ),
                        Span::styled(
                            opt.label.as_str(),
                            label_style,
                        ),
                        Span::styled(
                            format!("  {}", opt.description),
                            Style::default().fg(theme.text_dim),
                        ),
                    ]));
                }

                // Footer: ╰───╯
                result.push(Line::from(vec![Span::styled(
                    "  \u{2570}\u{2500}\u{2500}\u{2500}\u{256F}",
                    Style::default().fg(border_color),
                )]));
            }
            return result;
        }

        // Tool call: rounded card style — ╭─ name ─╮ / │ output / ╰─ status ─╯
        if let Some(info) = &self.tool_info {
            let border_color = if info.result_preview.is_some() {
                theme.success
            } else {
                theme.tool
            };
            let display_name = tool_user_facing_name(&info.tool_name, &info.arguments_summary);
            let args_display = tool_args_display(&info.tool_name, &info.arguments_summary);

            // Header: ╭─ name (args) ─╮
            let header_label = if args_display.is_empty() {
                display_name
            } else {
                format!("{display_name} ({args_display})")
            };
            result.push(Line::from(vec![
                Span::styled("  \u{256D}\u{2500} ", Style::default().fg(border_color)),
                Span::styled(
                    header_label,
                    Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
                ),
                Span::styled(" \u{2500}\u{2500}", Style::default().fg(border_color)),
            ]));

            // Body: │ output lines
            if !info.collapsed {
                for rendered_line in &self.lines {
                    let mut spans: Vec<Span<'_>> = vec![Span::styled(
                        "  \u{2502} ",
                        Style::default().fg(border_color),
                    )];
                    let line_text: String = rendered_line
                        .segments
                        .iter()
                        .map(|s| s.text.as_str())
                        .collect();
                    let diff_color = diff_line_color(&line_text, &theme);
                    if let Some(color) = diff_color {
                        spans.push(Span::styled(line_text, Style::default().fg(color)));
                    } else {
                        spans.extend(rendered_line.segments.iter().map(|seg| {
                            Span::styled(seg.text.as_str(), Style::default().fg(seg.color))
                        }));
                    }
                    result.push(Line::from(spans));
                }
            }

            // Footer: ╰─ status ─╯
            let status_label = if info.result_preview.is_some() {
                "\u{2713} done"
            } else {
                "\u{2026} running"
            };
            result.push(Line::from(vec![
                Span::styled("  \u{2570}\u{2500}", Style::default().fg(border_color)),
                Span::styled(
                    format!(" {status_label} "),
                    Style::default().fg(theme.text_dim),
                ),
                Span::styled("\u{2500}\u{256F}", Style::default().fg(border_color)),
            ]));

            return result;
        }

        // Thinking block: "∴ Thinking (N lines)" — collapsible with Ctrl-O
        if matches!(self.kind, ChatMessageKind::Thinking) {
            let line_count = self.lines.len();
            if self.thinking_collapsed {
                // Collapsed: show summary only
                let summary = if line_count > 0 {
                    // Show first non-empty line as preview
                    let first: String = self
                        .lines
                        .iter()
                        .map(|l| {
                            l.segments
                                .iter()
                                .map(|s| s.text.as_str())
                                .collect::<String>()
                        })
                        .find(|s| !s.trim().is_empty())
                        .unwrap_or_default();
                    let preview = if first.chars().count() > 60 {
                        let truncated: String = first.chars().take(59).collect();
                        format!("{truncated}\u{2026}")
                    } else {
                        first
                    };
                    format!("\u{25B6} Thinking ({line_count} lines): {preview}")
                } else {
                    String::from("Thinking...")
                };
                result.push(Line::from(vec![
                    Span::styled(prefix, Style::default().fg(prefix_color)),
                    Span::styled(summary, Style::default().fg(theme.text_dim)),
                ]));
            } else {
                // Expanded: show all thinking content
                result.push(Line::from(vec![
                    Span::styled(prefix, Style::default().fg(prefix_color)),
                    Span::styled(
                        format!("\u{25BC} Thinking ({line_count} lines)"),
                        Style::default().fg(theme.text_dim),
                    ),
                ]));
                for rendered_line in &self.lines {
                    let mut spans: Vec<Span<'_>> = vec![Span::styled(
                        "  \u{23BF} ",
                        Style::default().fg(theme.border),
                    )];
                    spans.extend(rendered_line.segments.iter().map(|seg| {
                        Span::styled(seg.text.as_str(), Style::default().fg(theme.text_dim))
                    }));
                    result.push(Line::from(spans));
                }
            }
            return result;
        }

        // Spinner: "∴ Thinking..."
        if matches!(self.kind, ChatMessageKind::Spinner) {
            for rendered_line in &self.lines {
                let text: String = rendered_line
                    .segments
                    .iter()
                    .map(|s| s.text.as_str())
                    .collect();
                result.push(Line::from(vec![
                    Span::styled(prefix, Style::default().fg(prefix_color)),
                    Span::styled(text, Style::default().fg(theme.text_dim)),
                ]));
            }
            return result;
        }

        // User: "❯ message" — first line gets prefix, rest indented
        if matches!(self.kind, ChatMessageKind::User) {
            for (i, rendered_line) in self.lines.iter().enumerate() {
                let line_prefix = if i == 0 { prefix } else { "  " };
                let mut spans: Vec<Span<'_>> =
                    vec![Span::styled(line_prefix, Style::default().fg(prefix_color))];
                spans.extend(rendered_line.segments.iter().map(|seg| {
                    let mut style = Style::default().fg(seg.color);
                    if seg.bold {
                        style = style.add_modifier(Modifier::BOLD);
                    }
                    if seg.strikethrough {
                        style = style.add_modifier(Modifier::CROSSED_OUT);
                    }
                    Span::styled(seg.text.as_str(), style)
                }));
                result.push(Line::from(spans));
            }
            return result;
        }

        // Assistant/System/Error: content lines with ⎿/•/✖ on first line
        for (i, rendered_line) in self.lines.iter().enumerate() {
            let line_prefix = if i == 0 { prefix } else { "  " };
            let prefix_col = if i == 0 { prefix_color } else { theme.border };
            let mut spans: Vec<Span<'_>> =
                vec![Span::styled(line_prefix, Style::default().fg(prefix_col))];
            spans.extend(rendered_line.segments.iter().map(|seg| {
                let mut style = Style::default().fg(seg.color);
                if seg.bold {
                    style = style.add_modifier(Modifier::BOLD);
                }
                if seg.italic {
                    style = style.add_modifier(Modifier::ITALIC);
                }
                if seg.strikethrough {
                    style = style.add_modifier(Modifier::CROSSED_OUT);
                }
                Span::styled(seg.text.as_str(), style)
            }));
            result.push(Line::from(spans));
        }

        result
    }

    /// Estimate height in terminal rows at given width.
    pub fn height(&self, _width: u16) -> u16 {
        if let Some(info) = &self.plan_choice {
            if info.resolved {
                return 2; // header + resolved footer
            }
            // header + body_lines + separator(if body) + options + footer
            let sep = u16::from(!info.body_lines.is_empty());
            return (info.body_lines.len() as u16)
                .saturating_add(info.options.len() as u16)
                .saturating_add(2) // header + footer
                .saturating_add(sep);
        }
        if self.tool_info.is_some() {
            // Card: header + body lines + footer = lines + 2
            (self.lines.len() as u16).saturating_add(2).max(2)
        } else if matches!(self.kind, ChatMessageKind::Spinner) {
            (self.lines.len() as u16).max(1)
        } else {
            (self.lines.len() as u16).saturating_add(1).max(1)
        }
    }
}

// ---------------------------------------------------------------------------
// InputBox
// ---------------------------------------------------------------------------

pub(crate) struct InputBox<'a> {
    pub input: &'a str,
    #[allow(dead_code)]
    pub cursor_pos: usize,
    pub mode_label: &'a str,
    pub view_offset: usize,
    pub scroll_y: u16,
}

impl<'a> InputBox<'a> {
    pub fn new(input: &'a str, cursor_pos: usize) -> Self {
        Self {
            input,
            cursor_pos,
            mode_label: "",
            view_offset: 0,
            scroll_y: 0,
        }
    }

    pub fn with_mode(mut self, label: &'a str) -> Self {
        self.mode_label = label;
        self
    }

    pub fn with_view_offset(mut self, offset: usize) -> Self {
        self.view_offset = offset;
        self
    }

    pub fn with_scroll_y(mut self, scroll_y: u16) -> Self {
        self.scroll_y = scroll_y;
        self
    }
}

impl Widget for InputBox<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = default_theme();

        // Thin separator line above input (Pi-style)
        let content_area = if area.height >= 2 {
            let sep_area = Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: 1,
            };
            let sep: String = "\u{2500}".repeat(area.width as usize);
            Paragraph::new(sep)
                .style(Style::default().fg(theme.input_border))
                .render(sep_area, buf);
            Rect {
                x: area.x,
                y: area.y + 1,
                width: area.width,
                height: area.height - 1,
            }
        } else {
            area
        };

        // Multi-line: split by newlines, first line gets "> " prefix, rest get "  "
        let input_lines: Vec<&str> = self.input.split('\n').collect();
        let mut lines: Vec<Line<'_>> = Vec::with_capacity(input_lines.len());

        // Mode label prefix for first line (e.g. "[NORMAL] > ")
        let mode_prefix = if self.mode_label.is_empty() {
            String::new()
        } else {
            format!("[{}] ", self.mode_label)
        };

        for (i, line_text) in input_lines.iter().enumerate() {
            let visible = if self.view_offset > 0 && !line_text.is_empty() {
                let mut char_idx = 0;
                let mut col = 0usize;
                for (idx, _ch) in line_text.char_indices() {
                    if col >= self.view_offset {
                        char_idx = idx;
                        break;
                    }
                    col += 1;
                    char_idx = idx + _ch.len_utf8();
                }
                if col < self.view_offset {
                    ""
                } else {
                    &line_text[char_idx..]
                }
            } else {
                line_text
            };
            if i == 0 {
                let mut spans = Vec::new();
                if !mode_prefix.is_empty() {
                    let mode_color = if self.mode_label == "NORMAL" {
                        theme.claude
                    } else {
                        theme.success
                    };
                    spans.push(Span::styled(
                        mode_prefix.clone(),
                        Style::default().fg(mode_color).add_modifier(Modifier::BOLD),
                    ));
                }
                spans.push(Span::styled("> ", Style::default().fg(theme.text_dim)));
                spans.push(Span::styled(
                    visible.to_string(),
                    Style::default().fg(theme.text),
                ));
                lines.push(Line::from(spans));
            } else {
                lines.push(Line::from(vec![
                    Span::styled("  ", Style::default().fg(theme.text_dim)),
                    Span::styled(visible.to_string(), Style::default().fg(theme.text)),
                ]));
            }
        }
        let paragraph = Paragraph::new(lines).scroll((self.scroll_y, 0));
        paragraph.render(content_area, buf);
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
            let max_w = area.width.saturating_sub(2) as usize;
            let display = if content.len() > max_w && max_w > 3 {
                // Truncate with ellipsis
                let mut idx = max_w - 1;
                while idx > 0 && !content.is_char_boundary(idx) {
                    idx -= 1;
                }
                format!("{}\u{2026}", &content[..idx])
            } else {
                content
            };
            let w = (display.len() as u16).min(area.width.saturating_sub(2));
            let x = area.x + 1;
            if w > 0 {
                let status_area = Rect::new(x, area.y, w, 1);
                let status_p = Paragraph::new(display).style(style);
                status_p.render(status_area, buf);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// HintsBar — contextual keyboard shortcuts
// ---------------------------------------------------------------------------

pub(crate) struct HintsBar {
    pub vim_normal: bool,
    pub has_completion: bool,
    pub has_images: bool,
}

impl Widget for HintsBar {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = default_theme();
        let dim = Style::default().fg(theme.text_dim);
        let hints = if self.has_completion {
            " \u{2191}/\u{2193} select \u{00B7} tab/enter confirm \u{00B7} esc cancel"
        } else if self.vim_normal {
            " i insert \u{00B7} /cmd \u{00B7} :q quit \u{00B7} j/k scroll"
        } else if self.has_images {
            " enter send with images \u{00B7} ctrl-v more images \u{00B7} /help commands"
        } else {
            " enter send \u{00B7} /help commands \u{00B7} ctrl-v image \u{00B7} ctrl-c quit"
        };
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
    pub scroll: u16,
}

impl<'a> OverlayBlock<'a> {
    pub fn new(title: &'a str, body: &'a str) -> Self {
        Self {
            title,
            body,
            scroll: 0,
        }
    }

    pub fn with_scroll(mut self, scroll: u16) -> Self {
        self.scroll = scroll;
        self
    }
}

impl Widget for OverlayBlock<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = default_theme();
        // Center the overlay at 80% width, 60% height, clamped to terminal size
        let w = (area.width * 4 / 5).max(20).min(area.width);
        let h = (area.height * 3 / 5).max(8).min(area.height);
        let x = area.x + (area.width.saturating_sub(w)) / 2;
        let y = area.y + (area.height.saturating_sub(h)) / 2;
        let overlay_area = Rect::new(x, y, w, h);

        Clear.render(overlay_area, buf);

        let block = Block::default()
            .borders(Borders::TOP | Borders::BOTTOM)
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
            .wrap(Wrap { trim: false })
            .scroll((self.scroll, 0));
        paragraph.render(inner, buf);
    }
}

// ---------------------------------------------------------------------------
// WelcomeBanner — startup splash with ASCII logo + system info
// ---------------------------------------------------------------------------

/// System info passed to the welcome banner for display.
pub(crate) struct WelcomeBannerInfo {
    pub model: String,
    pub provider: String,
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
            provider: String::new(),
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

const LOGO: &[&str] = &[
    r"  _ __   ___   ___ ___  __| | ___  ",
    r" | '_ \ / _ \ / __/ _ \/ _` |/ _ \ ",
    r" | | | | (_) | (_| (_) | (_| |  __/",
    r" |_| |_|\___/ \___\___/ \__,_|\___|",
];

/// Prompt suggestions shown on startup — rotates each minute.
const SUGGESTIONS: &[&str] = &[
    "fix typecheck errors",
    "explain this codebase",
    "write tests for the last change",
    "refactor this function",
    "review my recent changes",
];

impl Widget for WelcomeBanner<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = default_theme();

        if area.height < 8 || area.width < 24 {
            return;
        }

        let mut lines: Vec<Line<'_>> = Vec::with_capacity(8);

        // ── Layer 1: ASCII Art Logo ──
        for logo_line in LOGO {
            lines.push(Line::from(Span::styled(
                *logo_line,
                Style::default()
                    .fg(theme.claude)
                    .add_modifier(Modifier::BOLD),
            )));
        }

        // Single breathing line between logo and info
        lines.push(Line::from(""));

        // ── Layer 2: Single info line — version · model · provider · mode ──
        let mut info_parts: Vec<Span<'_>> = Vec::new();
        info_parts.push(Span::styled(
            format!("nocode v{}", self.info.version),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        ));
        info_parts.push(Span::styled(
            "  \u{2022}  ",
            Style::default().fg(theme.border),
        ));
        info_parts.push(Span::styled(
            self.info.model.as_str(),
            Style::default().fg(theme.assistant),
        ));
        if !self.info.provider.is_empty() {
            info_parts.push(Span::styled(
                "  \u{2022}  ",
                Style::default().fg(theme.border),
            ));
            info_parts.push(Span::styled(
                self.info.provider.as_str(),
                Style::default().fg(theme.text_dim),
            ));
        }
        if self.info.mode != "default" && !self.info.mode.is_empty() {
            info_parts.push(Span::styled(
                "  \u{2022}  ",
                Style::default().fg(theme.border),
            ));
            info_parts.push(Span::styled(
                self.info.mode.as_str(),
                Style::default().fg(theme.text_dim),
            ));
        }
        lines.push(Line::from(info_parts));

        // ── Layer 3: cwd ──
        let home = std::env::var("HOME").unwrap_or_default();
        let cwd_display = if !home.is_empty() && self.info.cwd.starts_with(&home) {
            format!("~{}", &self.info.cwd[home.len()..])
        } else {
            self.info.cwd.clone()
        };
        lines.push(Line::from(Span::styled(
            cwd_display,
            Style::default().fg(theme.text_inactive),
        )));

        // ── Layer 4: Call to action — prompt suggestion ──
        let suggestion_idx = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as usize
            / 60)
            % SUGGESTIONS.len();
        let suggestion = SUGGESTIONS[suggestion_idx];
        lines.push(Line::from(vec![
            Span::styled("\u{276F} ", Style::default().fg(theme.user)),
            Span::styled(
                format!("Try \u{2018}{suggestion}\u{2019}"),
                Style::default()
                    .fg(theme.text_dim)
                    .add_modifier(Modifier::ITALIC),
            ),
        ]));

        // Render: vertically center the banner in the available area, biased toward bottom
        let total_lines = lines.len() as u16;
        let available = area.height;
        let y_start = if available > total_lines + 4 {
            // Place at ~60% down the viewport for visual weight
            area.y + (available * 3 / 5).saturating_sub(total_lines / 2)
        } else {
            area.y + available.saturating_sub(total_lines + 1)
        };
        let render_area = Rect::new(
            area.x,
            y_start.min(area.y + available.saturating_sub(total_lines)),
            area.width,
            total_lines.min(available),
        );

        let paragraph = Paragraph::new(lines).alignment(Alignment::Center);
        paragraph.render(render_area, buf);
    }
}

// ---------------------------------------------------------------------------
// Tool-specific display helpers
// ---------------------------------------------------------------------------

/// Map tool name to a user-facing display name (Claude Code style).
fn tool_user_facing_name(tool_name: &str, args: &str) -> String {
    match tool_name {
        "Bash" => {
            // Show the command itself as the name
            let cmd = extract_kv(args, "command").unwrap_or_default();
            if cmd.is_empty() {
                String::from("Bash")
            } else {
                let short = truncate_str(&cmd, 50);
                format!("$ {short}")
            }
        }
        "Read" => String::from("Read"),
        "Edit" => String::from("Edit"),
        "Write" => String::from("Write"),
        "Glob" => String::from("Glob"),
        "Grep" => String::from("Grep"),
        "WebFetch" => String::from("Fetch"),
        "WebSearch" => String::from("Search"),
        "Agent" => String::from("Agent"),
        other => other.to_string(),
    }
}

/// Extract the most relevant argument for display in parentheses.
fn tool_args_display(tool_name: &str, args: &str) -> String {
    match tool_name {
        "Read" | "Edit" | "Write" => extract_kv(args, "file_path").unwrap_or_default(),
        "Glob" => extract_kv(args, "pattern").unwrap_or_default(),
        "Grep" => extract_kv(args, "pattern").unwrap_or_default(),
        "WebFetch" => extract_kv(args, "url").unwrap_or_default(),
        "WebSearch" => extract_kv(args, "query").unwrap_or_default(),
        "Agent" => extract_kv(args, "prompt")
            .map(|s| truncate_str(&s, 40))
            .unwrap_or_default(),
        "Bash" => String::new(), // command already in display name
        _ => {
            // Generic: show first key=value, truncated
            truncate_str(args, 50)
        }
    }
}

/// Extract value for a key from "key=value key2=value2" format.
fn extract_kv(args: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    let start = args.find(&prefix)?;
    let after = &args[start + prefix.len()..];
    let end = after.find(' ').unwrap_or(after.len());
    let val = &after[..end];
    if val.is_empty() {
        None
    } else {
        Some(val.to_string())
    }
}

/// Truncate a string to max chars, adding … if truncated.
fn truncate_str(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{truncated}\u{2026}")
    }
}
