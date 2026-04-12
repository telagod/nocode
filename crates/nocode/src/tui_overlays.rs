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
            let tools = mgr.all_tools();
            let text = if servers.is_empty() {
                "No MCP servers connected.\n\nConfigure in .nocode/settings.json under \"mcp_servers\".\n\nCommands:\n  /mcp-add <name> <command> [args...]  — Connect a server\n  /mcp-remove <name>                   — Disconnect\n  /mcp-restart <name>                  — Reconnect".to_string()
            } else {
                let mut lines = vec![format!("Connected MCP servers ({}):\n", servers.len())];
                for (name, phase, tool_count) in &servers {
                    let status = match phase {
                        nocode_core::mcp::manager::McpPhase::Connected => "●",
                        nocode_core::mcp::manager::McpPhase::Handshake => "●",
                        nocode_core::mcp::manager::McpPhase::Spawning => "◐",
                        nocode_core::mcp::manager::McpPhase::Initializing => "◐",
                        nocode_core::mcp::manager::McpPhase::ToolDiscovery => "◐",
                        nocode_core::mcp::manager::McpPhase::Reconnecting => "◐",
                        nocode_core::mcp::manager::McpPhase::Degraded => "◐",
                        nocode_core::mcp::manager::McpPhase::HealthCheck => "◐",
                        _ => "○",
                    };
                    lines.push(format!("  {status} {name}: {phase:?} ({tool_count} tools)"));
                }
                lines.push(String::new());
                if !tools.is_empty() {
                    lines.push(format!("Discovered tools ({}):", tools.len()));
                    for (server, tool) in tools.iter().take(30) {
                        lines.push(format!("  {server}:{}", tool.name));
                    }
                    if tools.len() > 30 {
                        lines.push(format!("  ... and {} more", tools.len() - 30));
                    }
                }
                lines.join("\n")
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
                "No background agents running.\n\nUse /agent-create <name> <prompt> to spawn one."
                    .to_string()
            } else {
                let mut lines = vec![format!("Background agents ({}):\n", workers.len())];
                for w in &workers {
                    let state_icon = match w.state {
                        nocode_core::agent::worker::WorkerState::Running => "▶",
                        nocode_core::agent::worker::WorkerState::ReadyForPrompt => "●",
                        nocode_core::agent::worker::WorkerState::Finished => "✓",
                        nocode_core::agent::worker::WorkerState::Failed => "✗",
                        _ => "○",
                    };
                    lines.push(format!(
                        "  {state_icon} {} ({}): {:?}",
                        w.name, w.id, w.state
                    ));
                }
                lines.push(String::new());
                lines.push("Commands: /agent-create <name> <prompt>".to_string());
                lines.join("\n")
            };
            let overlay_w = OverlayBlock::new("Agents", &text);
            frame.render_widget(overlay_w, area);
        }
        Overlay::Config {
            selected,
            tier,
            suggestion_index,
            editing,
            input,
            status,
            model,
            custom_base_url,
            custom_api_format,
            model_suggestions,
        } => {
            let tier_label = match tier {
                0 => "user",
                1 => "project",
                _ => "local",
            };
            let active = |idx: usize, current: usize| if idx == current { ">" } else { " " };
            let field_value =
                |idx: usize, current: usize, editing: bool, input: &str, value: &str| {
                    if idx == current && editing {
                        format!("{input}_")
                    } else if value.is_empty() {
                        "(not set)".to_string()
                    } else {
                        value.to_string()
                    }
                };
            let mut lines = vec![
                "Editable configuration".to_string(),
                String::new(),
                format!(
                    "{} Model:        {}",
                    active(0, *selected),
                    field_value(0, *selected, *editing, input, model)
                ),
                format!(
                    "{} Custom URL:   {}",
                    active(1, *selected),
                    field_value(1, *selected, *editing, input, custom_base_url)
                ),
                format!(
                    "{} Custom API:   {}",
                    active(2, *selected),
                    field_value(2, *selected, *editing, input, custom_api_format)
                ),
                String::new(),
                format!("Save tier: {tier_label}  [Tab to cycle]"),
                format!("Quick API toggle: {}", if *selected == 2 { "←/→" } else { "select Custom API field" }),
                "Controls: ↑/↓ select  Enter apply/edit  E manual edit  S save  R refresh  Esc close/cancel".to_string(),
                "Paste works while editing a field.".to_string(),
                "Tip: leave a field empty to clear it.".to_string(),
                String::new(),
                format!(
                    "Provider preview: {}",
                    if custom_base_url.is_empty() && custom_api_format.is_empty() {
                        "default".to_string()
                    } else {
                        format!(
                            "custom ({})",
                            if custom_api_format.is_empty() {
                                "openai"
                            } else {
                                custom_api_format
                            }
                        )
                    }
                ),
                "Default launch mode: TUI".to_string(),
            ];
            if !model_suggestions.is_empty() {
                lines.push(String::new());
                lines.push(
                    "Model suggestions (auto-loaded; ←/→ move, Enter/1-8 apply):".to_string(),
                );
                for (idx, suggestion) in model_suggestions.iter().take(8).enumerate() {
                    let marker = if idx == *suggestion_index { ">" } else { " " };
                    lines.push(format!("  {marker} {}. {suggestion}", idx + 1));
                }
                if model_suggestions.len() > 8 {
                    lines.push(format!("  ... and {} more", model_suggestions.len() - 8));
                }
            }
            if let Some(status) = status {
                lines.push(String::new());
                lines.push(format!("Status: {status}"));
            }
            let config_text = lines.join("\n");
            let overlay_w = OverlayBlock::new("Configuration", &config_text);
            frame.render_widget(overlay_w, area);
        }
        Overlay::Memory => {
            use nocode_core::storage::memory::MemoryStore;
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
            let mem_dir = format!("{home}/.nocode/memory");
            let store = MemoryStore::new(&mem_dir);
            let text = match store.list() {
                Ok(entries) if entries.is_empty() => {
                    format!(
                        "Memory directory: {mem_dir}\n\nNo memories stored yet.\n\nUse the MemorySave tool or /memory <query> to search."
                    )
                }
                Ok(entries) => {
                    let mut lines = vec![format!("Memory entries ({}):\n", entries.len())];
                    for entry in entries.iter().take(20) {
                        let ty = entry.memory_type.as_str();
                        lines.push(format!("  [{ty}] {} — {}", entry.name, entry.description));
                    }
                    if entries.len() > 20 {
                        lines.push(format!("  ... and {} more", entries.len() - 20));
                    }
                    lines.push(String::new());
                    lines.push(format!("Directory: {mem_dir}"));
                    lines.join("\n")
                }
                Err(e) => format!("Memory directory: {mem_dir}\n\nError: {e}"),
            };
            let overlay_w = OverlayBlock::new("Memory", &text);
            frame.render_widget(overlay_w, area);
        }
        Overlay::Cost => {
            let cost = hud.estimated_cost();
            let inp = hud.cumulative_input_tokens();
            let out = hud.cumulative_output_tokens();
            let total = inp + out;
            let text = format!(
                "Token usage:\n\n\
                 \x20 Input:         {inp}\n\
                 \x20 Output:        {out}\n\
                 \x20 Total:         {total}\n\
                 \x20 Est. cost:     ${cost:.4}\n\
                 \x20 Context used:  {:.1}%\n\n\
                 Cost is estimated based on model pricing.\n\
                 Use /insights for detailed session statistics.",
                hud.context_pct(),
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
            text.push_str("[Enter] Confirm  [Esc] Cancel  [\u{2190}\u{2192}] Change option  [1-4] Quick select");
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
