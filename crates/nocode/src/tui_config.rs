use crate::config_flow::{
    ConfigFlowState, ConfigStep, FlowAction, spawn_detect, spawn_fetch_models,
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
    pub flow: ConfigFlowState,
    pub input: String,
    pub detect_rx: Option<Receiver<DetectResult>>,
    pub models_rx: Option<Receiver<Result<Vec<String>, String>>>,
    pub closed: bool,
    pub saved: bool,
}

impl TuiConfigOverlay {
    pub fn new() -> Self {
        Self {
            flow: ConfigFlowState::new(),
            input: String::new(),
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
                    self.flow.on_detect_complete(result);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    self.detect_rx = Some(rx);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.flow.on_detect_complete(DetectResult {
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
                    self.flow.on_models_fetched(result);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    self.models_rx = Some(rx);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.flow
                        .on_models_fetched(Err("Fetch thread crashed".into()));
                }
            }
        }
    }

    fn execute_action(&mut self, action: FlowAction) {
        match action {
            FlowAction::None => {}
            FlowAction::StartDetection => {
                let rx = spawn_detect(self.flow.base_url.clone(), self.flow.api_key.clone());
                self.detect_rx = Some(rx);
            }
            FlowAction::FetchModels => {
                let rx = spawn_fetch_models(
                    self.flow.base_url.clone(),
                    self.flow.api_key.clone(),
                    self.flow.api_format.clone(),
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
        match &self.flow.step {
            ConfigStep::SelectProvider { .. } => self.handle_key_select_provider(key),
            ConfigStep::EnterUrl { .. } => self.handle_key_text_input(key, InputTarget::Url),
            ConfigStep::EnterKey { .. } => self.handle_key_text_input(key, InputTarget::Key),
            ConfigStep::Detecting => {
                if key.code == KeyCode::Esc {
                    let action = self.flow.go_back();
                    self.execute_action(action);
                }
            }
            ConfigStep::SelectFormat { .. } => self.handle_key_select_format(key),
            ConfigStep::SelectModel { .. } => self.handle_key_select_model(key),
            ConfigStep::ManualModel { .. } => self.handle_key_manual_model(key),
            ConfigStep::Confirm => self.handle_key_confirm(key),
            ConfigStep::Done => {}
        }
    }

    fn handle_key_select_provider(&mut self, key: KeyEvent) {
        let ConfigStep::SelectProvider {
            ref mut selected,
            ref mut scroll,
        } = self.flow.step
        else {
            return;
        };
        let list = ConfigFlowState::provider_list();
        let max_visible = 12;
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if *selected > 0 => {
                *selected -= 1;
                if *selected < *scroll {
                    *scroll = *selected;
                }
            }
            KeyCode::Down | KeyCode::Char('j') if *selected + 1 < list.len() + 1 => {
                *selected += 1;
                if *selected >= *scroll + max_visible {
                    *scroll = *selected - max_visible + 1;
                }
            }
            KeyCode::Enter => {
                let sel = *selected;
                if sel < list.len() {
                    let action = self.flow.select_provider(sel);
                    self.input.clear();
                    self.execute_action(action);
                } else {
                    let action = self.flow.select_custom_url();
                    self.input.clear();
                    self.execute_action(action);
                }
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                self.closed = true;
            }
            _ => {}
        }
    }

    fn handle_key_text_input(&mut self, key: KeyEvent, target: InputTarget) {
        match key.code {
            KeyCode::Enter => {
                let val = self.input.clone();
                if target == InputTarget::Url {
                    if val.trim().is_empty() {
                        return;
                    }
                    let action = self.flow.submit_url(val);
                    self.input.clear();
                    self.execute_action(action);
                } else {
                    let action = self.flow.submit_key(val);
                    self.input.clear();
                    self.execute_action(action);
                }
            }
            KeyCode::Esc => {
                self.input.clear();
                let action = self.flow.go_back();
                self.execute_action(action);
            }
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.clear();
            }
            KeyCode::Char(c) => {
                self.input.push(c);
            }
            _ => {}
        }
    }

    fn handle_key_select_format(&mut self, key: KeyEvent) {
        let ConfigStep::SelectFormat { ref mut selected } = self.flow.step else {
            return;
        };
        let formats = ["openai-chat", "openai-responses", "anthropic", "google"];
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if *selected > 0 => *selected -= 1,
            KeyCode::Down | KeyCode::Char('j') if *selected + 1 < formats.len() => *selected += 1,
            KeyCode::Enter => {
                let sel = *selected;
                let action = self.flow.select_format(sel);
                self.execute_action(action);
            }
            KeyCode::Esc => {
                let action = self.flow.go_back();
                self.execute_action(action);
            }
            _ => {}
        }
    }

    fn handle_key_select_model(&mut self, key: KeyEvent) {
        let ConfigStep::SelectModel {
            ref mut selected,
            ref mut scroll,
            ref mut filter,
            ref mut filtering,
        } = self.flow.step
        else {
            return;
        };
        let max_visible = 10;
        let count = self.flow.filtered_models.len();

        if *filtering {
            match key.code {
                KeyCode::Esc => {
                    *filter = String::new();
                    *filtering = false;
                    *selected = 0;
                    *scroll = 0;
                    self.flow.apply_filter("");
                }
                KeyCode::Backspace => {
                    filter.pop();
                    if filter.is_empty() {
                        *filtering = false;
                    }
                    *selected = 0;
                    *scroll = 0;
                    let f = filter.clone();
                    self.flow.apply_filter(&f);
                }
                KeyCode::Enter if count > 0 => {
                    let model = self.flow.filtered_models[*selected].clone();
                    let action = self.flow.select_model(model);
                    self.execute_action(action);
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
                    self.flow.apply_filter(&f);
                }
                _ => {}
            }
            return;
        }

        // Normal (non-filter) mode
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
            KeyCode::Enter => {
                if count > 0 {
                    let model = self.flow.filtered_models[*selected].clone();
                    let action = self.flow.select_model(model);
                    self.execute_action(action);
                } else {
                    self.input.clear();
                    self.flow.step = ConfigStep::ManualModel {
                        input: String::new(),
                    };
                }
            }
            KeyCode::Char('/') if count > 0 => {
                *filtering = true;
                *filter = String::new();
                *selected = 0;
                *scroll = 0;
                self.flow.apply_filter("");
            }
            KeyCode::Char('m') | KeyCode::Char('i') => {
                self.input.clear();
                self.flow.step = ConfigStep::ManualModel {
                    input: String::new(),
                };
            }
            KeyCode::Esc => {
                let action = self.flow.go_back();
                self.execute_action(action);
            }
            _ => {}
        }
    }

    fn handle_key_manual_model(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => {
                let model = self.input.clone();
                let action = self.flow.submit_manual_model(model);
                if action != FlowAction::None {
                    self.input.clear();
                    self.execute_action(action);
                }
            }
            KeyCode::Esc => {
                self.input.clear();
                let action = self.flow.go_back();
                self.execute_action(action);
            }
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.clear();
            }
            KeyCode::Char(c) => {
                self.input.push(c);
            }
            _ => {}
        }
    }

    fn handle_key_confirm(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter | KeyCode::Char('y') => {
                let action = self.flow.confirm();
                self.execute_action(action);
            }
            KeyCode::Esc | KeyCode::Char('n') => {
                let action = self.flow.go_back();
                self.execute_action(action);
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
        let success_style = Style::default().fg(theme.success);

        let mut lines: Vec<Line> = Vec::with_capacity(24);

        match &self.flow.step {
            ConfigStep::SelectProvider { selected, scroll } => {
                lines.push(Line::from(Span::styled("Select a provider", highlight)));
                lines.push(Line::raw(""));

                let list = ConfigFlowState::provider_list();
                let max_visible = 12;
                let end = (*scroll + max_visible).min(list.len() + 1);
                for (i, preset) in list
                    .iter()
                    .enumerate()
                    .skip(*scroll)
                    .take(end.saturating_sub(*scroll))
                {
                    let marker = if i == *selected { "▸ " } else { "  " };
                    let style = if i == *selected { highlight } else { normal };
                    lines.push(Line::from(vec![
                        Span::styled(marker, if i == *selected { highlight } else { dim }),
                        Span::styled(preset.name, style),
                        Span::styled(format!("  {}", preset.auth_hint), dim),
                    ]));
                }
                // Custom URL entry option
                let custom_idx = list.len();
                if custom_idx < end {
                    let marker = if custom_idx == *selected {
                        "▸ "
                    } else {
                        "  "
                    };
                    let style = if custom_idx == *selected {
                        highlight
                    } else {
                        normal
                    };
                    lines.push(Line::from(vec![
                        Span::styled(
                            marker,
                            if custom_idx == *selected {
                                highlight
                            } else {
                                dim
                            },
                        ),
                        Span::styled("Custom URL...", style),
                        Span::styled("  Enter any base URL", dim),
                    ]));
                }
                lines.push(Line::raw(""));
                lines.push(Line::from(vec![
                    Span::styled("↑↓", normal),
                    Span::styled(" navigate  ", dim),
                    Span::styled("Enter", normal),
                    Span::styled(" select  ", dim),
                    Span::styled("Esc", normal),
                    Span::styled(" cancel", dim),
                ]));
            }

            ConfigStep::EnterUrl { .. } => {
                lines.push(Line::from(Span::styled("Enter base URL", highlight)));
                lines.push(Line::raw(""));
                lines.push(Line::from(vec![
                    Span::styled("URL: ", dim),
                    Span::styled(format!("{}█", self.input), normal),
                ]));
                lines.push(Line::raw(""));
                lines.push(Line::from(vec![
                    Span::styled("Example: ", dim),
                    Span::styled("http://localhost:11434/v1", dim),
                ]));
                lines.push(Line::raw(""));
                lines.push(Line::from(vec![
                    Span::styled("Enter", normal),
                    Span::styled(" confirm  ", dim),
                    Span::styled("Esc", normal),
                    Span::styled(" back", dim),
                ]));
            }

            ConfigStep::EnterKey { .. } => {
                let name = self.flow.preset.map_or("Custom", |p| p.name);
                let hint = self.flow.preset.map_or("", |p| p.auth_hint);
                lines.push(Line::from(Span::styled(
                    format!("API Key — {name}"),
                    highlight,
                )));
                lines.push(Line::raw(""));
                if !hint.is_empty() {
                    lines.push(Line::from(Span::styled(hint, dim)));
                    lines.push(Line::raw(""));
                }
                let masked = mask_input(&self.input);
                lines.push(Line::from(vec![
                    Span::styled("Key: ", dim),
                    Span::styled(format!("{masked}█"), normal),
                ]));
                lines.push(Line::raw(""));
                lines.push(Line::from(vec![
                    Span::styled("Enter", normal),
                    Span::styled(" confirm  ", dim),
                    Span::styled("Esc", normal),
                    Span::styled(" back", dim),
                ]));
            }

            ConfigStep::Detecting => {
                lines.push(Line::from(Span::styled("Detecting...", highlight)));
                lines.push(Line::raw(""));
                lines.push(Line::from(Span::styled(
                    self.flow.status.as_deref().unwrap_or("Probing endpoint..."),
                    dim,
                )));
                lines.push(Line::raw(""));
                lines.push(Line::from(vec![
                    Span::styled("Esc", normal),
                    Span::styled(" cancel", dim),
                ]));
            }

            ConfigStep::SelectFormat { selected } => {
                lines.push(Line::from(Span::styled(
                    "Auto-detect failed — select API format",
                    highlight,
                )));
                if let Some(err) = &self.flow.error {
                    lines.push(Line::from(Span::styled(err.as_str(), dim)));
                }
                lines.push(Line::raw(""));
                let formats = [
                    (
                        "openai-chat",
                        "OpenAI Chat Completions (/v1/chat/completions)",
                    ),
                    ("openai-responses", "OpenAI Responses (/v1/responses)"),
                    ("anthropic", "Anthropic Messages (/v1/messages)"),
                    ("google", "Google Gemini (generateContent)"),
                ];
                for (i, (name, desc)) in formats.iter().enumerate() {
                    let marker = if i == *selected { "▸ " } else { "  " };
                    let style = if i == *selected { highlight } else { normal };
                    lines.push(Line::from(vec![
                        Span::styled(marker, if i == *selected { highlight } else { dim }),
                        Span::styled(*name, style),
                        Span::styled(format!("  {desc}"), dim),
                    ]));
                }
                lines.push(Line::raw(""));
                lines.push(Line::from(vec![
                    Span::styled("↑↓", normal),
                    Span::styled(" navigate  ", dim),
                    Span::styled("Enter", normal),
                    Span::styled(" select  ", dim),
                    Span::styled("Esc", normal),
                    Span::styled(" back", dim),
                ]));
            }

            ConfigStep::SelectModel {
                selected,
                scroll,
                filter,
                filtering,
            } => {
                let count = self.flow.filtered_models.len();
                let total = self.flow.models.len();
                let filter_info = if *filtering || !filter.is_empty() {
                    format!(" /{filter}")
                } else {
                    String::new()
                };
                lines.push(Line::from(vec![
                    Span::styled("Select model", highlight),
                    Span::styled(format!("  ({count}/{total}){filter_info}"), dim),
                ]));
                if let Some(status) = &self.flow.status {
                    lines.push(Line::from(Span::styled(status.as_str(), success_style)));
                }
                lines.push(Line::raw(""));

                let max_visible = 10;
                let end = (*scroll + max_visible).min(count);
                for (i, model) in self
                    .flow
                    .filtered_models
                    .iter()
                    .enumerate()
                    .skip(*scroll)
                    .take(end.saturating_sub(*scroll))
                {
                    let marker = if i == *selected { "▸ " } else { "  " };
                    let style = if i == *selected { highlight } else { normal };
                    lines.push(Line::from(vec![
                        Span::styled(marker, if i == *selected { highlight } else { dim }),
                        Span::styled(model.as_str(), style),
                    ]));
                }
                if count == 0 {
                    lines.push(Line::from(Span::styled(
                        "  No models found — type model name manually",
                        dim,
                    )));
                }
                lines.push(Line::raw(""));
                lines.push(Line::from(vec![
                    Span::styled("↑↓", normal),
                    Span::styled(" navigate  ", dim),
                    Span::styled("/", normal),
                    Span::styled(" filter  ", dim),
                    Span::styled("m", normal),
                    Span::styled(" manual  ", dim),
                    Span::styled("Enter", normal),
                    Span::styled(" select  ", dim),
                    Span::styled("Esc", normal),
                    Span::styled(" back", dim),
                ]));
            }

            ConfigStep::ManualModel { .. } => {
                lines.push(Line::from(Span::styled("Enter model name", highlight)));
                lines.push(Line::raw(""));
                lines.push(Line::from(vec![
                    Span::styled("Model: ", dim),
                    Span::styled(format!("{}█", self.input), normal),
                ]));
                if let Some(preset) = self.flow.preset
                    && !preset.default_model.is_empty()
                {
                    lines.push(Line::raw(""));
                    lines.push(Line::from(vec![
                        Span::styled("Default: ", dim),
                        Span::styled(preset.default_model, dim),
                    ]));
                }
                lines.push(Line::raw(""));
                lines.push(Line::from(vec![
                    Span::styled("Enter", normal),
                    Span::styled(" confirm  ", dim),
                    Span::styled("Esc", normal),
                    Span::styled(" back", dim),
                ]));
            }

            ConfigStep::Confirm => {
                lines.push(Line::from(Span::styled("Confirm configuration", highlight)));
                lines.push(Line::raw(""));
                let name = self.flow.preset.map_or("Custom", |p| p.name);
                lines.push(Line::from(vec![
                    Span::styled("  Provider: ", dim),
                    Span::styled(name, normal),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("  Base URL: ", dim),
                    Span::styled(self.flow.base_url.as_str(), normal),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("  Format:   ", dim),
                    Span::styled(self.flow.api_format.as_str(), normal),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("  Model:    ", dim),
                    Span::styled(self.flow.model.as_str(), normal),
                ]));
                if !self.flow.api_key.is_empty() {
                    lines.push(Line::from(vec![
                        Span::styled("  API Key:  ", dim),
                        Span::styled(mask_key(&self.flow.api_key), normal),
                    ]));
                }
                lines.push(Line::raw(""));
                lines.push(Line::from(vec![
                    Span::styled("Enter", normal),
                    Span::styled(" save  ", dim),
                    Span::styled("Esc", normal),
                    Span::styled(" back", dim),
                ]));
            }

            ConfigStep::Done => {
                lines.push(Line::from(Span::styled("✓ Saved!", success_style)));
            }
        }

        // Render centered overlay
        let w = (area.width * 4 / 5).max(40).min(area.width);
        let content_h = lines.len() as u16 + 2;
        let h = content_h.max(8).min(area.height * 4 / 5).min(area.height);
        let x = area.x + (area.width.saturating_sub(w)) / 2;
        let y = area.y + (area.height.saturating_sub(h)) / 2;
        let overlay_area = Rect::new(x, y, w, h);

        Clear.render(overlay_area, frame.buffer_mut());

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.claude))
            .title(" Setup ")
            .title_style(
                Style::default()
                    .fg(theme.claude)
                    .add_modifier(Modifier::BOLD),
            );
        let inner = block.inner(overlay_area);
        block.render(overlay_area, frame.buffer_mut());

        let paragraph = Paragraph::new(lines);
        paragraph.render(inner, frame.buffer_mut());
    }
}

fn mask_input(input: &str) -> String {
    if input.is_empty() {
        return String::new();
    }
    "*".repeat(input.len().saturating_sub(1))
        + &input
            .chars()
            .last()
            .map_or(String::new(), |c| c.to_string())
}

fn mask_key(key: &str) -> String {
    if key.len() <= 8 {
        return "*".repeat(key.len());
    }
    format!("{}...{}", &key[..4], &key[key.len() - 4..])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputTarget {
    Url,
    Key,
}
