use crate::config_flow::{ConfigField, ConfigFormState, ValueSource};
use crate::tui_theme::default_theme;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

pub(crate) struct TuiConfigOverlay {
    pub form: ConfigFormState,
    pub closed: bool,
}

impl TuiConfigOverlay {
    pub fn new() -> Self {
        Self {
            form: ConfigFormState::new(),
            closed: false,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                self.form.focus = self.form.focus.next();
            }
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
                self.form.focus = self.form.focus.prev();
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                self.closed = true;
            }
            _ => {}
        }
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect) {
        let theme = default_theme();
        let dim = Style::default().fg(theme.text_dim);
        let normal = Style::default().fg(theme.text);
        let highlight = Style::default()
            .fg(theme.claude)
            .add_modifier(Modifier::BOLD);
        let section_style = Style::default().fg(theme.claude);
        let success_style = Style::default().fg(theme.success);

        let mut lines: Vec<Line> = Vec::with_capacity(24);

        let sel = |field: ConfigField| -> Span {
            if self.form.focus == field {
                Span::styled("▸ ", highlight)
            } else {
                Span::raw("  ")
            }
        };

        let source_span = |src: &ValueSource| Span::styled(format!("  ({})", src.label()), dim);

        let field_val = |val: &str, style_dim: Style, style_norm: Style| -> Span<'static> {
            let s = val.to_string();
            if s.is_empty() || s == "(not set)" {
                Span::styled(s, style_dim)
            } else {
                Span::styled(s, style_norm)
            }
        };

        // ── Provider ─────────────────────────────
        lines.push(section_line("Provider", 36, section_style, dim));
        lines.push(Line::from(vec![
            sel(ConfigField::Provider),
            Span::styled("Provider  ", dim),
            if self.form.focus == ConfigField::Provider {
                Span::styled(self.form.display_provider().to_string(), highlight)
            } else {
                field_val(self.form.display_provider(), dim, normal)
            },
            source_span(&self.form.provider_source),
        ]));

        let api_key_str = self.form.display_api_key();
        lines.push(Line::from(vec![
            sel(ConfigField::ApiKey),
            Span::styled("API Key   ", dim),
            field_val(&api_key_str, dim, normal),
            source_span(&self.form.api_key_source),
        ]));
        lines.push(Line::raw(""));

        // ── Endpoint ─────────────────────────────
        let tag = self
            .form
            .preset
            .map_or(String::new(), |p| format!(" {} ", p.name));
        lines.push(section_line_with_tag(
            "Endpoint",
            &tag,
            36,
            section_style,
            dim,
        ));

        lines.push(Line::from(vec![
            sel(ConfigField::BaseUrl),
            Span::styled("Base URL  ", dim),
            field_val(self.form.display_base_url(), dim, normal),
            source_span(&self.form.base_url_source),
        ]));

        lines.push(Line::from(vec![
            sel(ConfigField::Format),
            Span::styled("Format    ", dim),
            field_val(self.form.display_format(), dim, normal),
            source_span(&self.form.format_source),
        ]));
        lines.push(Line::raw(""));

        // ── Model ────────────────────────────────
        lines.push(section_line("Model", 36, section_style, dim));

        let mut model_line = vec![
            sel(ConfigField::Model),
            Span::styled("Active    ", dim),
            field_val(self.form.display_model(), dim, normal),
            source_span(&self.form.model_source),
        ];
        if !self.form.model.is_empty() {
            model_line.push(Span::styled(" ●", success_style));
        }
        lines.push(Line::from(model_line));
        lines.push(Line::raw(""));

        // ── Footer ───────────────────────────────
        lines.push(Line::from(vec![
            Span::styled("↑↓", normal),
            Span::styled(" navigate  ", dim),
            Span::styled("Esc", normal),
            Span::styled(" close  ", dim),
            Span::styled("nocode --login", normal),
            Span::styled(" to change", dim),
        ]));

        // ── Render ───────────────────────────────
        let w = (area.width * 4 / 5).max(40).min(area.width);
        let content_h = lines.len() as u16 + 2;
        let h = content_h.max(12).min(area.height * 4 / 5).min(area.height);
        let x = area.x + (area.width.saturating_sub(w)) / 2;
        let y = area.y + (area.height.saturating_sub(h)) / 2;
        let overlay_area = Rect::new(x, y, w, h);

        Clear.render(overlay_area, frame.buffer_mut());

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.claude))
            .title(" Configuration ")
            .title_style(
                Style::default()
                    .fg(theme.claude)
                    .add_modifier(Modifier::BOLD),
            );
        let inner = block.inner(overlay_area);
        block.render(overlay_area, frame.buffer_mut());

        Paragraph::new(lines).render(inner, frame.buffer_mut());
    }
}

fn section_line<'a>(label: &'a str, width: usize, section_style: Style, dim: Style) -> Line<'a> {
    let pad = width.saturating_sub(label.len() + 4);
    Line::from(vec![
        Span::styled("── ", dim),
        Span::styled(label, section_style),
        Span::styled(" ", dim),
        Span::styled("─".repeat(pad), dim),
    ])
}

fn section_line_with_tag<'a>(
    label: &'a str,
    tag: &'a str,
    width: usize,
    section_style: Style,
    dim: Style,
) -> Line<'a> {
    let used = label.len() + tag.len() + 6;
    let pad = width.saturating_sub(used);
    Line::from(vec![
        Span::styled("── ", dim),
        Span::styled(label, section_style),
        Span::styled(" ", dim),
        Span::styled("─".repeat(pad / 2), dim),
        Span::styled(tag, section_style),
        Span::styled("─".repeat(pad.saturating_sub(pad / 2)), dim),
    ])
}
