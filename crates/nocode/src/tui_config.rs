use crate::config_flow::{ConfigField, ConfigFormState, ValueSource};
use crate::tui_theme::default_theme;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

pub(crate) struct TuiConfigOverlay {
    pub form: ConfigFormState,
    pub closed: bool,
    editing: Option<EditState>,
    picking: Option<PickState>,
}

struct EditState {
    field: ConfigField,
    buffer: String,
    cursor: usize,
}

struct PickState {
    field: ConfigField,
    selected: usize,
}

impl TuiConfigOverlay {
    pub fn new() -> Self {
        Self {
            form: ConfigFormState::new(),
            closed: false,
            editing: None,
            picking: None,
        }
    }

    fn settings_key_for(field: ConfigField) -> Option<&'static str> {
        match field {
            ConfigField::Model => Some("model"),
            ConfigField::Provider => Some("model_provider"),
            ConfigField::BaseUrl => Some("custom_base_url"),
            ConfigField::Format => Some("custom_api_format"),
            ConfigField::ApiKey => Some("__api_key__"),
        }
    }

    fn current_value_for(&self, field: ConfigField) -> String {
        match field {
            ConfigField::Model => self.form.model.clone(),
            ConfigField::Provider => self.form.display_provider().to_string(),
            ConfigField::BaseUrl => self.form.base_url.clone(),
            ConfigField::Format => self.form.api_format.clone(),
            ConfigField::ApiKey => String::new(),
        }
    }

    fn start_editing(&mut self) {
        let field = self.form.focus;
        if Self::settings_key_for(field).is_none() {
            return;
        }
        // Provider uses picker, not free-text editing
        if field == ConfigField::Provider {
            use crate::provider_presets::ALL_PRESETS;
            let current_idx = self
                .form
                .preset
                .and_then(|cur| ALL_PRESETS.iter().position(|p| p.name == cur.name))
                .unwrap_or(0);
            self.picking = Some(PickState {
                field,
                selected: current_idx,
            });
            return;
        }
        let value = self.current_value_for(field);
        let cursor = value.len();
        self.editing = Some(EditState {
            field,
            buffer: value,
            cursor,
        });
    }

    fn confirm_edit(&mut self) {
        let Some(edit) = self.editing.take() else {
            return;
        };
        if Self::settings_key_for(edit.field).is_none() {
            return;
        }
        let value = edit.buffer.trim().to_string();

        // Update form state in-memory
        match edit.field {
            ConfigField::Model => {
                self.form.model = value.clone();
                self.form.model_source = ValueSource::User;
            }
            ConfigField::BaseUrl => {
                self.form.base_url = value.clone();
                self.form.base_url_source = ValueSource::User;
            }
            ConfigField::Format => {
                self.form.api_format = value.clone();
                self.form.format_source = ValueSource::User;
            }
            ConfigField::Provider | ConfigField::ApiKey => {}
        }

        // ApiKey → credential store
        if edit.field == ConfigField::ApiKey {
            if !value.is_empty() {
                let provider_label = self
                    .form
                    .preset
                    .map_or("custom", |p| p.credential_slot);
                let cred_path =
                    nocode_core::storage::credentials::CredentialStore::default_path();
                let mut store =
                    nocode_core::storage::credentials::CredentialStore::load(&cred_path)
                        .unwrap_or_default();
                store.set_key(provider_label, &value);
                let _ = store.save(&cred_path);
                self.form.api_key = value.clone();
            }
            return;
        }

        // Other fields → persist to user tier config.toml
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        use nocode_core::config::settings::{Settings, SettingsTier};
        let key = Self::settings_key_for(edit.field).unwrap();
        let persist_value = if value.is_empty() {
            None
        } else {
            Some(value.as_str())
        };
        let _ = Settings::persist_key_value(key, persist_value, SettingsTier::User, &cwd);
    }

    fn cancel_edit(&mut self) {
        self.editing = None;
    }

    fn confirm_pick(&mut self) {
        let Some(pick) = self.picking.take() else {
            return;
        };
        use crate::provider_presets::ALL_PRESETS;
        let preset = &ALL_PRESETS[pick.selected];
        self.form.preset = Some(preset);
        self.form.provider_source = ValueSource::User;

        // Persist model_provider to settings
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        use nocode_core::config::settings::{Settings, SettingsTier};
        let _ = Settings::persist_key_value(
            "model_provider",
            Some(preset.provider_type),
            SettingsTier::User,
            &cwd,
        );
    }

    fn cancel_pick(&mut self) {
        self.picking = None;
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        // Picker mode — up/down to select, Enter to confirm, Esc to cancel
        if let Some(ref mut pick) = self.picking {
            use crate::provider_presets::ALL_PRESETS;
            let count = ALL_PRESETS.len();
            match key.code {
                KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                    pick.selected = (pick.selected + 1) % count;
                }
                KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
                    pick.selected = (pick.selected + count - 1) % count;
                }
                KeyCode::Enter => self.confirm_pick(),
                KeyCode::Esc => self.cancel_pick(),
                _ => {}
            }
            return;
        }

        if let Some(ref mut edit) = self.editing {
            match key.code {
                KeyCode::Enter => self.confirm_edit(),
                KeyCode::Esc => self.cancel_edit(),
                KeyCode::Backspace if edit.cursor > 0 => {
                    let prev = prev_char_boundary(&edit.buffer, edit.cursor);
                    edit.buffer.drain(prev..edit.cursor);
                    edit.cursor = prev;
                }
                KeyCode::Left if edit.cursor > 0 => {
                    edit.cursor = prev_char_boundary(&edit.buffer, edit.cursor);
                }
                KeyCode::Right if edit.cursor < edit.buffer.len() => {
                    edit.cursor = next_char_boundary(&edit.buffer, edit.cursor);
                }
                KeyCode::Home | KeyCode::Char('a')
                    if key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    edit.cursor = 0;
                }
                KeyCode::End | KeyCode::Char('e')
                    if key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    edit.cursor = edit.buffer.len();
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    edit.buffer.drain(..edit.cursor);
                    edit.cursor = 0;
                }
                KeyCode::Char(c) => {
                    edit.buffer.insert(edit.cursor, c);
                    edit.cursor += c.len_utf8();
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                self.form.focus = self.form.focus.next();
            }
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
                self.form.focus = self.form.focus.prev();
            }
            KeyCode::Enter => {
                self.start_editing();
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
        let edit_style = Style::default()
            .fg(theme.text)
            .add_modifier(Modifier::UNDERLINED);

        let mut lines: Vec<Line> = Vec::with_capacity(24);

        let sel = |field: ConfigField| -> Span {
            if self.editing.as_ref().is_some_and(|e| e.field == field)
                || self.picking.as_ref().is_some_and(|p| p.field == field)
            {
                Span::styled("✎ ", highlight)
            } else if self.form.focus == field {
                Span::styled("▸ ", highlight)
            } else {
                Span::raw("  ")
            }
        };

        let source_span = |src: &ValueSource| Span::styled(format!("  ({})", src.label()), dim);

        let field_val =
            |field: ConfigField, val: &str, style_dim: Style, style_norm: Style| -> Span<'static> {
                if let Some(ref edit) = self.editing
                    && edit.field == field
                {
                    return Span::styled(
                        if edit.buffer.is_empty() {
                            "…".to_string()
                        } else {
                            edit.buffer.clone()
                        },
                        edit_style,
                    );
                }
                let s = val.to_string();
                if s.is_empty() || s == "(not set)" {
                    Span::styled(s, style_dim)
                } else {
                    Span::styled(s, style_norm)
                }
            };

        let editable_hint = |field: ConfigField| -> Span<'static> {
            if Self::settings_key_for(field).is_some()
                && self.form.focus == field
                && self.editing.is_none()
                && self.picking.is_none()
            {
                if field == ConfigField::Provider {
                    Span::styled("  [Enter to select]", dim)
                } else {
                    Span::styled("  [Enter to edit]", dim)
                }
            } else {
                Span::raw("")
            }
        };

        // ── Provider ─────────────────────────────
        lines.push(section_line("Provider", 36, section_style, dim));

        if let Some(ref pick) = self.picking {
            // Picker mode — show scrollable list of presets
            use crate::provider_presets::ALL_PRESETS;
            for (i, preset) in ALL_PRESETS.iter().enumerate() {
                let is_sel = i == pick.selected;
                let prefix = if is_sel { "▸ " } else { "  " };
                let style = if is_sel { highlight } else { normal };
                lines.push(Line::from(Span::styled(
                    format!("{prefix}{}", preset.name),
                    style,
                )));
            }
        } else {
            lines.push(Line::from(vec![
                sel(ConfigField::Provider),
                Span::styled("Provider  ", dim),
                field_val(
                    ConfigField::Provider,
                    self.form.display_provider(),
                    dim,
                    normal,
                ),
                source_span(&self.form.provider_source),
                editable_hint(ConfigField::Provider),
            ]));
        }

        let api_key_display = self.form.display_api_key();
        lines.push(Line::from(vec![
            sel(ConfigField::ApiKey),
            Span::styled("API Key   ", dim),
            field_val(ConfigField::ApiKey, &api_key_display, dim, normal),
            source_span(&self.form.api_key_source),
            editable_hint(ConfigField::ApiKey),
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
            field_val(
                ConfigField::BaseUrl,
                self.form.display_base_url(),
                dim,
                normal,
            ),
            source_span(&self.form.base_url_source),
            editable_hint(ConfigField::BaseUrl),
        ]));

        lines.push(Line::from(vec![
            sel(ConfigField::Format),
            Span::styled("Format    ", dim),
            field_val(ConfigField::Format, self.form.display_format(), dim, normal),
            source_span(&self.form.format_source),
            editable_hint(ConfigField::Format),
        ]));
        lines.push(Line::raw(""));

        // ── Model ────────────────────────────────
        lines.push(section_line("Model", 36, section_style, dim));

        let mut model_line = vec![
            sel(ConfigField::Model),
            Span::styled("Active    ", dim),
            field_val(ConfigField::Model, self.form.display_model(), dim, normal),
            source_span(&self.form.model_source),
            editable_hint(ConfigField::Model),
        ];
        if !self.form.model.is_empty()
            && !self
                .editing
                .as_ref()
                .is_some_and(|e| e.field == ConfigField::Model)
        {
            model_line.push(Span::styled(" ●", success_style));
        }
        lines.push(Line::from(model_line));
        lines.push(Line::raw(""));

        // ── Footer ───────────────────────────────
        if self.editing.is_some() {
            lines.push(Line::from(vec![
                Span::styled("Enter", normal),
                Span::styled(" save  ", dim),
                Span::styled("Esc", normal),
                Span::styled(" cancel  ", dim),
                Span::styled("Ctrl+U", normal),
                Span::styled(" clear", dim),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled("↑↓", normal),
                Span::styled(" navigate  ", dim),
                Span::styled("Enter", normal),
                Span::styled(" edit  ", dim),
                Span::styled("Esc", normal),
                Span::styled(" close", dim),
            ]));
        }

        // ── Render ───────────────────────────────
        let w = (area.width * 4 / 5).max(40).min(area.width);
        let content_h = lines.len() as u16 + 2;
        let h = content_h.max(12).min(area.height * 4 / 5).min(area.height);
        let x = area.x + (area.width.saturating_sub(w)) / 2;
        let y = area.y + (area.height.saturating_sub(h)) / 2;
        let overlay_area = Rect::new(x, y, w, h);

        Clear.render(overlay_area, frame.buffer_mut());

        let title = if self.editing.is_some() {
            " Configuration (editing) "
        } else {
            " Configuration "
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.claude))
            .title(title)
            .title_style(
                Style::default()
                    .fg(theme.claude)
                    .add_modifier(Modifier::BOLD),
            );
        let inner = block.inner(overlay_area);
        block.render(overlay_area, frame.buffer_mut());

        Paragraph::new(lines).render(inner, frame.buffer_mut());

        // Show edit cursor
        if let Some(ref edit) = self.editing {
            let edit_line_y = match edit.field {
                ConfigField::BaseUrl => 5,
                ConfigField::Format => 6,
                ConfigField::Model => 9,
                _ => 0,
            };
            if edit_line_y > 0 {
                let label_width = 12; // "  Active    " etc
                let cursor_x = inner.x + label_width + edit.cursor as u16;
                let cursor_y = inner.y + edit_line_y;
                if cursor_x < inner.x + inner.width && cursor_y < inner.y + inner.height {
                    frame.set_cursor_position((cursor_x, cursor_y));
                }
            }
        }
    }
}

fn prev_char_boundary(s: &str, pos: usize) -> usize {
    let mut i = pos.saturating_sub(1);
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn next_char_boundary(s: &str, pos: usize) -> usize {
    let mut i = pos + 1;
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i.min(s.len())
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
