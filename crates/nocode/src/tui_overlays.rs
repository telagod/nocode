//! TUI overlay rendering — only interactive overlays that need user input.

use crate::tui_widgets::OverlayBlock;
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::status_hud::StatusHud;
use crate::tui_app::Overlay;

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
            let overlay_w = OverlayBlock::new("\u{26A0} Permission Required", &text);
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
            text.push_str("[Enter] Confirm  [Esc] Cancel  [\u{2190}\u{2192}] Change  [1-4] Quick select");
            let overlay_w = OverlayBlock::new("Question", &text);
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
            let overlay_w = OverlayBlock::new("Error Log", &text);
            frame.render_widget(overlay_w, area);
        }
    }
}
