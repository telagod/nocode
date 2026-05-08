use crate::config_flow::{
    ConfigField, ConfigFormState, EditMode, FlowAction, ValueSource, spawn_detect,
    spawn_fetch_models,
};
use crate::protocol_detect::DetectResult;
use crate::tui_theme::default_theme;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};
use std::sync::mpsc::Receiver;

pub(crate) struct TuiConfigOverlay {
    pub form: ConfigFormState,
    pub detect_rx: Option<Receiver<DetectResult>>,
    pub models_rx: Option<Receiver<Result<Vec<String>, String>>>,
    pub closed: bool,
    pub saved: bool,
}

impl TuiConfigOverlay {
    pub fn new() -> Self {
        Self {
            form: ConfigFormState::new(),
            detect_rx: None,
            models_rx: None,
            closed: false,
            saved: false,
        }
    }

    pub fn poll(&mut self) {
        if let Some(rx) = self.detect_rx.take() {
            match rx.try_recv() {
                Ok(result) => {
                    self.form.on_detect_complete(result);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    self.detect_rx = Some(rx);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.form.on_detect_complete(DetectResult {
                        api_format: None,
                        models: Vec::new(),
                        error: Some("Detection thread crashed".into()),
                    });
                }
            }
        }
        if let Some(rx) = self.models_rx.take() {
            match rx.try_recv() {
                Ok(result) => {
                    self.form.on_models_fetched(result);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    self.models_rx = Some(rx);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.form
                        .on_models_fetched(Err("Fetch thread crashed".into()));
                }
            }
        }
    }

    fn execute_action(&mut self, action: FlowAction) {
        match action {
            FlowAction::None => {}
            FlowAction::StartDetection => {
                self.form.status = Some("Detecting protocol...".into());
                let rx = spawn_detect(self.form.base_url.clone(), self.form.api_key.clone());
                self.detect_rx = Some(rx);
            }
            FlowAction::FetchModels => {
                self.form.status = Some("Fetching models...".into());
                let rx = spawn_fetch_models(
                    self.form.base_url.clone(),
                    self.form.api_key.clone(),
                    self.form.api_format.clone(),
                );
                self.models_rx = Some(rx);
            }
            FlowAction::Save => {
                self.saved = true;
                self.closed = true;
            }
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        match &self.form.mode {
            EditMode::Normal => self.handle_normal(key),
            EditMode::EditingText(_) => self.handle_editing(key),
            EditMode::BrowsingModels { .. } => self.handle_browsing_models(key),
            EditMode::FilteringModels { .. } => self.handle_filtering_models(key),
        }
    }

    fn handle_normal(&mut self, key: KeyEvent) {
        match key.code {
            // Navigation
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                self.form.focus = self.form.focus.next();
            }
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
                self.form.focus = self.form.focus.prev();
            }

            // Provider: ←→ cycle
            KeyCode::Left | KeyCode::Char('h') if self.form.focus == ConfigField::Provider => {
                self.form.cycle_provider_backward();
            }
            KeyCode::Right | KeyCode::Char('l') if self.form.focus == ConfigField::Provider => {
                self.form.cycle_provider_forward();
            }

            // Format: ←→ cycle
            KeyCode::Left | KeyCode::Char('h') if self.form.focus == ConfigField::Format => {
                self.form.cycle_format_backward();
            }
            KeyCode::Right | KeyCode::Char('l') if self.form.focus == ConfigField::Format => {
                self.form.cycle_format_forward();
            }

            // Enter: edit text fields, or browse models
            KeyCode::Enter => match self.form.focus {
                ConfigField::ApiKey => {
                    self.form.edit_buffer.clear();
                    self.form.mode = EditMode::EditingText(ConfigField::ApiKey);
                }
                ConfigField::BaseUrl => {
                    self.form.edit_buffer = self.form.base_url.clone();
                    self.form.mode = EditMode::EditingText(ConfigField::BaseUrl);
                }
                ConfigField::Model => {
                    if self.form.filtered_models.is_empty() {
                        self.form.edit_buffer = self.form.model.clone();
                        self.form.mode = EditMode::EditingText(ConfigField::Model);
                    } else {
                        let sel = self
                            .form
                            .filtered_models
                            .iter()
                            .position(|m| m == &self.form.model)
                            .unwrap_or(0);
                        self.form.mode = EditMode::BrowsingModels {
                            selected: sel,
                            scroll: sel.saturating_sub(4),
                        };
                    }
                }
                // Provider and Format only cycle via ←→
                ConfigField::Provider | ConfigField::Format => {}
            },

            // Shortcuts
            KeyCode::Char('s') | KeyCode::Char('S') => {
                self.save();
            }
            KeyCode::Char('t') | KeyCode::Char('T') => {
                let action = self.form.start_detection();
                self.execute_action(action);
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                let action = self.form.start_fetch_models();
                self.execute_action(action);
            }
            KeyCode::Char('/') if self.form.focus == ConfigField::Model => {
                self.form.mode = EditMode::FilteringModels {
                    selected: 0,
                    scroll: 0,
                    filter: String::new(),
                };
                self.form.apply_filter("");
            }
            KeyCode::Char('m') | KeyCode::Char('M') if self.form.focus == ConfigField::Model => {
                self.form.edit_buffer = self.form.model.clone();
                self.form.mode = EditMode::EditingText(ConfigField::Model);
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                self.closed = true;
            }
            _ => {}
        }
    }

    fn handle_editing(&mut self, key: KeyEvent) {
        let field = match &self.form.mode {
            EditMode::EditingText(f) => *f,
            _ => return,
        };
        match key.code {
            KeyCode::Enter => match field {
                ConfigField::ApiKey => self.form.submit_api_key(),
                ConfigField::BaseUrl => self.form.submit_base_url(),
                ConfigField::Model => self.form.submit_model_manual(),
                _ => self.form.mode = EditMode::Normal,
            },
            KeyCode::Esc => {
                self.form.edit_buffer.clear();
                self.form.mode = EditMode::Normal;
            }
            KeyCode::Backspace => {
                self.form.edit_buffer.pop();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.form.edit_buffer.clear();
            }
            KeyCode::Char(c) => {
                self.form.edit_buffer.push(c);
            }
            _ => {}
        }
    }

    fn handle_browsing_models(&mut self, key: KeyEvent) {
        let (selected, scroll) = match &mut self.form.mode {
            EditMode::BrowsingModels { selected, scroll } => (selected, scroll),
            _ => return,
        };
        let count = self.form.filtered_models.len();
        let max_visible: usize = 8;
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if *selected > 0 => {
                *selected -= 1;
                if *selected < *scroll {
                    *scroll = *selected;
                }
            }
            KeyCode::Down | KeyCode::Char('j') if *selected + 1 < count => {
                *selected += 1;
                if *selected >= *scroll + max_visible {
                    *scroll = *selected - max_visible + 1;
                }
            }
            KeyCode::Enter if count > 0 => {
                let model = self.form.filtered_models[*selected].clone();
                self.form.select_model_from_list(model);
            }
            KeyCode::Char('/') => {
                let s = *selected;
                let sc = *scroll;
                self.form.mode = EditMode::FilteringModels {
                    selected: s,
                    scroll: sc,
                    filter: String::new(),
                };
            }
            KeyCode::Esc => {
                self.form.mode = EditMode::Normal;
            }
            _ => {}
        }
    }

    fn handle_filtering_models(&mut self, key: KeyEvent) {
        let (selected, scroll, filter) = match &mut self.form.mode {
            EditMode::FilteringModels {
                selected,
                scroll,
                filter,
            } => (selected, scroll, filter),
            _ => return,
        };
        let count = self.form.filtered_models.len();
        let max_visible: usize = 8;
        match key.code {
            KeyCode::Esc => {
                self.form.apply_filter("");
                self.form.mode = EditMode::Normal;
            }
            KeyCode::Backspace => {
                filter.pop();
                let is_empty = filter.is_empty();
                *selected = 0;
                *scroll = 0;
                let f = filter.clone();
                self.form.apply_filter(&f);
                if is_empty {
                    self.form.mode = EditMode::BrowsingModels {
                        selected: 0,
                        scroll: 0,
                    };
                }
            }
            KeyCode::Enter if count > 0 => {
                let model = self.form.filtered_models[*selected].clone();
                self.form.apply_filter("");
                self.form.select_model_from_list(model);
            }
            KeyCode::Up if *selected > 0 => {
                *selected -= 1;
                if *selected < *scroll {
                    *scroll = *selected;
                }
            }
            KeyCode::Down if *selected + 1 < count => {
                *selected += 1;
                if *selected >= *scroll + max_visible {
                    *scroll = *selected - max_visible + 1;
                }
            }
            KeyCode::Char(c) => {
                filter.push(c);
                *selected = 0;
                *scroll = 0;
                let f = filter.clone();
                self.form.apply_filter(&f);
            }
            _ => {}
        }
    }

    fn save(&mut self) {
        self.saved = true;
        self.closed = true;
    }

    // ─── Rendering ──────────────────────────────────────────────────────

    pub fn draw(&self, frame: &mut Frame, area: Rect) {
        let theme = default_theme();
        let dim = Style::default().fg(theme.text_dim);
        let normal = Style::default().fg(theme.text);
        let highlight = Style::default()
            .fg(theme.claude)
            .add_modifier(Modifier::BOLD);
        let section_style = Style::default().fg(theme.claude);
        let success_style = Style::default().fg(theme.success);
        let error_style = Style::default().fg(theme.error);

        let mut lines: Vec<Line> = Vec::with_capacity(32);

        let sel = |field: ConfigField| -> Span {
            let focused = self.form.focus == field;
            let editing = matches!(&self.form.mode, EditMode::EditingText(f) if *f == field);
            if editing {
                Span::styled("● ", highlight)
            } else if focused && matches!(self.form.mode, EditMode::Normal) {
                Span::styled("▸ ", highlight)
            } else {
                Span::raw("  ")
            }
        };

        let source_span =
            |src: &ValueSource| -> Span { Span::styled(format!("  ({})", src.label()), dim) };

        let arrows = |field: ConfigField| -> Span {
            if self.form.focus == field && matches!(self.form.mode, EditMode::Normal) {
                Span::styled("  ◀ ▶", dim)
            } else {
                Span::raw("")
            }
        };

        // ── Provider ─────────────────────────────
        lines.push(section_line("Provider", 36, section_style, dim));

        lines.push(Line::from(vec![
            sel(ConfigField::Provider),
            Span::styled("Provider  ", dim),
            if self.form.focus == ConfigField::Provider {
                Span::styled(self.form.display_provider(), highlight)
            } else {
                field_display(self.form.display_provider(), normal, dim)
            },
            source_span(&self.form.provider_source),
            arrows(ConfigField::Provider),
        ]));

        // API Key
        let api_key_display_str = self.form.display_api_key();
        let editing_key = matches!(self.form.mode, EditMode::EditingText(ConfigField::ApiKey));
        let key_display = if editing_key {
            if self.form.edit_buffer.is_empty() {
                Span::styled("█ paste or type new key", highlight)
            } else {
                Span::styled(
                    format!("{}█", mask_input(&self.form.edit_buffer)),
                    highlight,
                )
            }
        } else {
            field_display(&api_key_display_str, normal, dim)
        };
        let key_source = if editing_key {
            Span::raw("")
        } else {
            source_span(&self.form.api_key_source)
        };
        lines.push(Line::from(vec![
            sel(ConfigField::ApiKey),
            Span::styled("API Key   ", dim),
            key_display,
            key_source,
        ]));
        lines.push(Line::raw(""));

        // ── Endpoint ─────────────────────────────
        let endpoint_extra = self
            .form
            .preset
            .map_or(String::new(), |p| format!(" {} ", p.name));
        lines.push(section_line_with_tag(
            "Endpoint",
            &endpoint_extra,
            36,
            section_style,
            dim,
        ));

        // Base URL
        let url_display = if matches!(self.form.mode, EditMode::EditingText(ConfigField::BaseUrl)) {
            Span::styled(format!("{}█", self.form.edit_buffer), highlight)
        } else {
            field_display(self.form.display_base_url(), normal, dim)
        };
        lines.push(Line::from(vec![
            sel(ConfigField::BaseUrl),
            Span::styled("Base URL  ", dim),
            url_display,
            source_span(&self.form.base_url_source),
        ]));

        // Format
        lines.push(Line::from(vec![
            sel(ConfigField::Format),
            Span::styled("Format    ", dim),
            if self.form.focus == ConfigField::Format {
                Span::styled(self.form.display_format(), highlight)
            } else {
                field_display(self.form.display_format(), normal, dim)
            },
            source_span(&self.form.format_source),
            arrows(ConfigField::Format),
        ]));
        lines.push(Line::raw(""));

        // ── Model ────────────────────────────────
        let filter_tag = match &self.form.mode {
            EditMode::FilteringModels { filter, .. } => format!(" /{filter} "),
            _ => String::new(),
        };
        let model_count_tag = if self.form.models.is_empty() {
            String::new()
        } else {
            format!(
                " {}/{} ",
                self.form.filtered_models.len(),
                self.form.models.len()
            )
        };
        lines.push(section_line_with_tags(
            "Model",
            &filter_tag,
            &model_count_tag,
            36,
            section_style,
            dim,
        ));

        // Active model
        let model_display = if matches!(self.form.mode, EditMode::EditingText(ConfigField::Model)) {
            Span::styled(format!("{}█", self.form.edit_buffer), highlight)
        } else {
            field_display(self.form.display_model(), normal, dim)
        };
        let mut active_line = vec![
            sel(ConfigField::Model),
            Span::styled("Active    ", dim),
            model_display,
            source_span(&self.form.model_source),
        ];
        if !self.form.model.is_empty() {
            active_line.push(Span::styled(" ●", success_style));
        }
        lines.push(Line::from(active_line));

        // Model list (browsing or filtering)
        let (browse_sel, browse_scroll) = match &self.form.mode {
            EditMode::BrowsingModels { selected, scroll } => (Some(*selected), *scroll),
            EditMode::FilteringModels {
                selected, scroll, ..
            } => (Some(*selected), *scroll),
            _ => (None, 0),
        };

        if browse_sel.is_some() || !self.form.filtered_models.is_empty() {
            let list = &self.form.filtered_models;
            let max_visible = 8;
            let end = (browse_scroll + max_visible).min(list.len());
            let show_list = browse_sel.is_some();

            if show_list {
                for (i, model) in list
                    .iter()
                    .enumerate()
                    .skip(browse_scroll)
                    .take(end.saturating_sub(browse_scroll))
                {
                    let is_active = model == &self.form.model;
                    let is_selected = browse_sel == Some(i);
                    let marker = if is_selected { "  ▸ " } else { "    " };
                    let style = if is_selected {
                        highlight
                    } else if is_active {
                        success_style
                    } else {
                        normal
                    };
                    let mut spans = vec![
                        Span::styled(marker, if is_selected { highlight } else { dim }),
                        Span::styled(model.as_str(), style),
                    ];
                    if is_active {
                        spans.push(Span::styled(" ●", success_style));
                    }
                    lines.push(Line::from(spans));
                }
                if list.is_empty() {
                    lines.push(Line::from(vec![
                        Span::raw("    "),
                        Span::styled("(no models — press R to fetch)", dim),
                    ]));
                }
                // Scroll indicators
                if list.len() > max_visible {
                    let mut ind = String::from("    ");
                    if browse_scroll > 0 {
                        ind.push_str("↑ ");
                    }
                    if end < list.len() {
                        ind.push('↓');
                    }
                    lines.push(Line::from(Span::styled(ind, dim)));
                }
            }
        } else if self.form.models.is_empty()
            && self.form.status.as_deref() != Some("Fetching models...")
            && self.form.status.as_deref() != Some("Detecting protocol...")
        {
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled("(no models — press R to fetch)", dim),
            ]));
        }
        lines.push(Line::raw(""));

        // ── Footer ───────────────────────────────
        let ctrl_spans = vec![
            Span::styled("↑↓", normal),
            Span::styled(" nav  ", dim),
            Span::styled("◀▶", normal),
            Span::styled(" cycle  ", dim),
            Span::styled("Enter", normal),
            Span::styled(" edit  ", dim),
            Span::styled("S", normal),
            Span::styled(" save  ", dim),
            Span::styled("T", normal),
            Span::styled(" test  ", dim),
            Span::styled("R", normal),
            Span::styled(" fetch  ", dim),
            Span::styled("Esc", normal),
            Span::styled(" ×", dim),
        ];
        lines.push(Line::from(ctrl_spans));

        // Status / error line
        if let Some(err) = &self.form.error {
            lines.push(Line::from(Span::styled(err.as_str(), error_style)));
        } else if let Some(status) = &self.form.status {
            lines.push(Line::from(Span::styled(status.as_str(), dim)));
        }

        // ── Render overlay ───────────────────────
        let w = (area.width * 4 / 5).max(40).min(area.width);
        let content_h = lines.len() as u16 + 2;
        let h = content_h.max(14).min(area.height * 4 / 5).min(area.height);
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

// ─── Helpers ─────────────────────────────────────────────────────────────

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

fn section_line_with_tags<'a>(
    label: &'a str,
    tag1: &'a str,
    tag2: &'a str,
    width: usize,
    section_style: Style,
    dim: Style,
) -> Line<'a> {
    let used = label.len() + tag1.len() + tag2.len() + 8;
    let pad = width.saturating_sub(used);
    let half = pad / 2;
    Line::from(vec![
        Span::styled("── ", dim),
        Span::styled(label, section_style),
        Span::styled(" ", dim),
        Span::styled("─".repeat(half), dim),
        Span::styled(tag1, section_style),
        Span::styled("─".repeat(half), dim),
        Span::styled(tag2, dim),
        Span::styled("─", dim),
    ])
}

fn field_display<'a>(val: &'a str, normal: Style, dim: Style) -> Span<'a> {
    if val == "(not set)" || val.is_empty() {
        Span::styled(val, dim)
    } else {
        Span::styled(val, normal)
    }
}

fn mask_input(input: &str) -> String {
    if input.is_empty() {
        return String::new();
    }
    let len = input.chars().count();
    if len <= 1 {
        return input.to_string();
    }
    format!(
        "{}{}",
        "*".repeat(len - 1),
        input.chars().last().unwrap_or('*')
    )
}
