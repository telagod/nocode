//! TUI overlay rendering — extracted from tui_app.rs.

use crate::command_registry::CommandRegistry;
use crate::tui_widgets::OverlayBlock;
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::status_hud::StatusHud;
use crate::tui_app::Overlay;

/// Draw the active overlay on top of the main UI.
pub(crate) fn draw_overlay(overlay: &Overlay, hud: &StatusHud, frame: &mut Frame, area: Rect) {
    match overlay {
        Overlay::None => {}
        Overlay::Help => {
            let cmd_reg = CommandRegistry::with_defaults();
            let mut help = String::from(
                "Keyboard shortcuts:\n\
                 \n\
                 Enter        — send message\n\
                 Shift-Enter  — newline\n\
                 Ctrl-C       — quit\n\
                 Esc          — vim normal / clear input\n\
                 Up/Down      — scroll chat\n\
                 Ctrl-T       — toggle theme\n\
                 Ctrl-L       — clear chat\n\
                 Ctrl-U       — clear input\n\
                 Ctrl-P/N     — input history\n\
                 Tab          — toggle tool output\n\
                 F4           — error log\n\
                 \n",
            );
            help.push_str(&cmd_reg.help_text());
            let overlay_w = OverlayBlock::new("Help", &help);
            frame.render_widget(overlay_w, area);
        }
        Overlay::Status => {
            let status = format!(
                "Session: {}\n\
                 Model: {}\n\
                 Input tokens: {}\n\
                 Output tokens: {}\n\
                 Context: {:.1}%",
                hud.session_name().unwrap_or("(unnamed)"),
                hud.model_name(),
                hud.cumulative_input_tokens(),
                hud.cumulative_output_tokens(),
                hud.context_pct(),
            );
            let overlay_w = OverlayBlock::new("Status", &status);
            frame.render_widget(overlay_w, area);
        }
        Overlay::Sessions => {
            let overlay_w = OverlayBlock::new(
                "Sessions",
                "Use /sessions in non-busy mode to list saved sessions.\n\
                 Use /resume <id> to restore a session.",
            );
            frame.render_widget(overlay_w, area);
        }
        Overlay::Mcp => {
            use nocode_core::mcp::manager::global_mcp_manager;
            let mgr = global_mcp_manager();
            let mgr = mgr.lock().unwrap_or_else(|e| e.into_inner());
            let servers = mgr.list_servers();
            let text = if servers.is_empty() {
                "No MCP servers connected.\n\nConfigure in .nocode/settings.json under \"mcp_servers\".".to_string()
            } else {
                let mut lines = Vec::new();
                for (name, phase, tool_count) in &servers {
                    lines.push(format!("  {name}: {phase:?} ({tool_count} tools)"));
                }
                format!("Connected MCP servers:\n\n{}", lines.join("\n"))
            };
            let overlay_w = OverlayBlock::new("MCP Servers", &text);
            frame.render_widget(overlay_w, area);
        }
        Overlay::Agents => {
            use nocode_core::agent::worker::global_worker_registry;
            let reg = global_worker_registry();
            let reg = reg.lock().unwrap_or_else(|e| e.into_inner());
            let workers = reg.list();
            let text = if workers.is_empty() {
                "No background agents running.".to_string()
            } else {
                let mut lines = Vec::new();
                for w in &workers {
                    lines.push(format!("  {} ({}): {:?}", w.name, w.id, w.state));
                }
                format!("Background agents:\n\n{}", lines.join("\n"))
            };
            let overlay_w = OverlayBlock::new("Agents", &text);
            frame.render_widget(overlay_w, area);
        }
        Overlay::Config => {
            let overlay_w = OverlayBlock::new(
                "Configuration",
                "Config loaded from:\n\
                 1. ~/.nocode/settings.json (user)\n\
                 2. .nocode/settings.json (project)\n\
                 3. .nocode/settings.local.json (local)\n\n\
                 Environment overrides: NOCODE_MODEL, NOCODE_SYSTEM_PROMPT, etc.",
            );
            frame.render_widget(overlay_w, area);
        }
        Overlay::Memory => {
            let overlay_w = OverlayBlock::new(
                "Memory",
                "Memory stored in ~/.nocode/memory/\n\
                 Use /memory <query> to search memories.",
            );
            frame.render_widget(overlay_w, area);
        }
        Overlay::Cost => {
            let cost = hud.estimated_cost();
            let text = format!(
                "Token usage:\n\n\
                 Input:  {}\n\
                 Output: {}\n\
                 Est. cost: ${cost:.4}",
                hud.cumulative_input_tokens(),
                hud.cumulative_output_tokens(),
            );
            let overlay_w = OverlayBlock::new("Cost", &text);
            frame.render_widget(overlay_w, area);
        }
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
