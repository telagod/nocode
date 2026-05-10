//! TUI overlay rendering — only interactive overlays that need user input.

use crate::tui_app::Overlay;
use crate::status_hud::StatusHud;
use crate::tui_theme::default_theme;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap};

pub(crate) fn draw_overlay(
    overlay: &Overlay,
    _hud: &StatusHud,
    _scroll: u16,
    frame: &mut Frame,
    area: Rect,
) {
    match overlay {
        Overlay::None => {}
        Overlay::Permission { tool_name, tool_id } => {
            let text = format!(
                "Tool: {tool_name}\n\
                 ID: {tool_id}\n\n\
                 Allow this tool call?\n\n\
                 [y] Yes  [n] No  [a] Always allow"
            );
            let overlay_w = crate::tui_widgets::OverlayBlock::new("\u{26A0} Permission Required", &text);
            frame.render_widget(overlay_w, area);
        }
        Overlay::Question {
            questions,
            selected,
        } => {
            let mut text = String::from("The assistant has a question:\n\n");
            if let Some(arr) = questions.as_array() {
                for (i, q) in arr.iter().enumerate() {
                    let header = q["header"].as_str().unwrap_or("Question");
                    let question_text = q["question"].as_str().unwrap_or("?");
                    let cur = selected.get(i).copied().unwrap_or(0);
                    text.push_str(&format!("[{header}] {question_text}\n"));
                    if let Some(opts) = q["options"].as_array() {
                        for (j, opt) in opts.iter().enumerate() {
                            let label = opt["label"].as_str().unwrap_or("?");
                            let desc = opt["description"].as_str().unwrap_or("");
                            let marker = if j == cur { ">" } else { " " };
                            text.push_str(&format!("  {marker} {}. {label} — {desc}\n", j + 1));
                        }
                    }
                    text.push('\n');
                }
            }
            text.push_str(
                "[Enter] Confirm  [Esc] Cancel  [\u{2190}\u{2192}] Change  [1-4] Quick select",
            );
            let overlay_w = crate::tui_widgets::OverlayBlock::new("Question", &text);
            frame.render_widget(overlay_w, area);
        }
        Overlay::Errors(errors) => {
            let text = if errors.is_empty() {
                "No errors recorded.".to_string()
            } else {
                let mut lines = Vec::new();
                for (i, err) in errors.iter().rev().enumerate().take(30) {
                    lines.push(format!("  {}: {err}", errors.len() - i));
                }
                format!(
                    "Recent errors ({} total):\n\n{}",
                    errors.len(),
                    lines.join("\n")
                )
            };
            let overlay_w = crate::tui_widgets::OverlayBlock::new("Error Log", &text);
            frame.render_widget(overlay_w, area);
        }
        Overlay::SessionPicker {
            sessions,
            selected,
            show_all,
        } => {
            draw_session_picker(frame, area, sessions, *selected, *show_all);
        }
    }
}

fn draw_session_picker(
    frame: &mut Frame,
    area: Rect,
    sessions: &[nocode_core::session::persistence::SessionInfo],
    selected: usize,
    show_all: bool,
) {
    let theme = default_theme();

    let w = (area.width * 4 / 5).max(30).min(area.width);
    let h = (area.height * 3 / 4).max(10).min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let overlay_area = Rect::new(x, y, w, h);

    Clear.render(overlay_area, frame.buffer_mut());

    // Scope tabs
    let project_style = if show_all {
        Style::default().fg(theme.text_dim)
    } else {
        Style::default().fg(theme.claude).add_modifier(Modifier::BOLD)
    };
    let all_style = if show_all {
        Style::default().fg(theme.claude).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.text_dim)
    };

    let title_line = Line::from(vec![
        Span::styled(" Resume Session  ", Style::default().fg(theme.claude).add_modifier(Modifier::BOLD)),
        Span::styled("│", Style::default().fg(theme.border)),
        Span::styled(" Project ", project_style),
        Span::styled("│", Style::default().fg(theme.border)),
        Span::styled(" All ", all_style),
        Span::styled(" (Tab to switch) ", Style::default().fg(theme.text_inactive)),
    ]);

    let block = Block::default()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(Style::default().fg(theme.claude))
        .title(title_line)
        .title_style(Style::default());
    let inner = block.inner(overlay_area);
    block.render(overlay_area, frame.buffer_mut());

    if sessions.is_empty() {
        let msg = if show_all {
            "No sessions found anywhere."
        } else {
            "No sessions in this project. Press Tab for all."
        };
        let p = Paragraph::new(msg)
            .style(Style::default().fg(theme.text_dim))
            .wrap(Wrap { trim: false });
        p.render(inner, frame.buffer_mut());
        return;
    }

    // Build session list lines
    let visible_rows = inner.height as usize;
    let scroll_offset = if selected >= visible_rows.saturating_sub(2) {
        selected.saturating_sub(visible_rows.saturating_sub(3))
    } else {
        0
    };

    let mut lines: Vec<Line<'_>> = Vec::new();
    let now = chrono::Utc::now();

    for (i, info) in sessions.iter().enumerate().skip(scroll_offset).take(visible_rows.saturating_sub(1)) {
        let is_sel = i == selected;
        let marker = if is_sel { "\u{25B8} " } else { "  " };
        let preview = info.first_user_message.as_deref().unwrap_or("(empty)");
        let age = info.modified_at.map(|t| format_age(now, t)).unwrap_or_default();
        let msgs = format!("{} msgs", info.message_count);

        // Truncate preview to fit
        let max_preview = (w as usize).saturating_sub(age.len() + msgs.len() + 12);
        let preview_trunc = if preview.len() > max_preview && max_preview > 3 {
            let mut idx = max_preview.saturating_sub(3);
            while idx > 0 && !preview.is_char_boundary(idx) {
                idx -= 1;
            }
            format!("{}\u{2026}", &preview[..idx])
        } else {
            preview.to_string()
        };

        let marker_style = if is_sel {
            Style::default().fg(theme.claude).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text_dim)
        };
        let text_style = if is_sel {
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text)
        };
        let dim = Style::default().fg(theme.text_dim);
        let age_color = if is_sel { Color::Cyan } else { theme.text_inactive };

        lines.push(Line::from(vec![
            Span::styled(marker, marker_style),
            Span::styled(preview_trunc, text_style),
            Span::styled("  ", dim),
            Span::styled(msgs, dim),
            Span::styled("  ", dim),
            Span::styled(age, Style::default().fg(age_color)),
        ]));
    }

    // Footer hint
    lines.push(Line::from(vec![
        Span::styled(
            " Enter:select  Tab:scope  Esc:close",
            Style::default().fg(theme.text_inactive),
        ),
    ]));

    let p = Paragraph::new(lines);
    p.render(inner, frame.buffer_mut());
}

fn format_age(now: chrono::DateTime<chrono::Utc>, t: chrono::DateTime<chrono::Utc>) -> String {
    let delta = now.signed_duration_since(t);
    if delta.num_minutes() < 1 {
        "just now".to_string()
    } else if delta.num_minutes() < 60 {
        format!("{}m ago", delta.num_minutes())
    } else if delta.num_hours() < 24 {
        format!("{}h ago", delta.num_hours())
    } else {
        format!("{}d ago", delta.num_days())
    }
}
