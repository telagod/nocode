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
                r#"No MCP servers connected.

Configure in .nocode/settings.json under "mcp_servers".

Commands:
  /mcp-add <name> <command> [args...]  — Connect a server
  /mcp-remove <name>                   — Disconnect
  /mcp-restart <name>                  — Reconnect"#
                    .to_string()
            } else {
                let mut lines = vec![format!(
                    "Connected MCP servers ({}):
",
                    servers.len()
                )];
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
                lines.join(
                    "
",
                )
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
                "No background agents running.

Use /agent-create <name> <prompt> to spawn one."
                    .to_string()
            } else {
                let mut lines = vec![format!(
                    "Background agents ({}):
",
                    workers.len()
                )];
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
                lines.join(
                    "
",
                )
            };
            let overlay_w = OverlayBlock::new("Agents", &text);
            frame.render_widget(overlay_w, area);
        }
        Overlay::Config {
            selected,
            tier,
            suggestion_index,
            suggestion_scroll,
            filtering_models,
            editing,
            input,
            status,
            provider,
            provider_source,
            api_key,
            api_key_source,
            model,
            model_source,
            custom_base_url,
            custom_base_url_source,
            custom_api_format,
            custom_api_format_source,
            model_filter,
            all_model_suggestions: _,
            model_suggestions,
        } => {
            let is_custom_provider = provider == "custom";
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
            let api_key_value = if *selected == 1 && *editing && !*filtering_models {
                format!("{input}_")
            } else if api_key.is_empty() {
                "(not set)".to_string()
            } else {
                let tail: String = api_key
                    .chars()
                    .rev()
                    .take(4)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                format!("configured (…{tail})")
            };
            let filter_value = if *selected == 2 && *editing && *filtering_models {
                format!("{input}_")
            } else if model_filter.is_empty() {
                "(none)".to_string()
            } else {
                model_filter.clone()
            };
            let mut lines = vec![
                "Editable configuration".to_string(),
                String::new(),
                "[Provider]".to_string(),
                format!(
                    "{} Provider:     {} ({})",
                    active(0, *selected),
                    provider,
                    provider_source
                ),
                String::new(),
                "[Auth]".to_string(),
                format!(
                    "{} API Key:      {} ({})",
                    active(1, *selected),
                    api_key_value,
                    api_key_source
                ),
                String::new(),
                "[Model]".to_string(),
                format!(
                    "{} Model:        {} ({})",
                    active(2, *selected),
                    field_value(2, *selected, *editing && !*filtering_models, input, model),
                    model_source
                ),
                format!("  Filter:       {filter_value}"),
                String::new(),
                "[Endpoint]".to_string(),
            ];
            if is_custom_provider {
                lines.push(format!(
                    "{} Custom URL:   {} ({})",
                    active(3, *selected),
                    field_value(3, *selected, *editing, input, custom_base_url),
                    custom_base_url_source
                ));
                lines.push(format!(
                    "{} Custom API:   {} ({})",
                    active(4, *selected),
                    field_value(4, *selected, *editing, input, custom_api_format),
                    custom_api_format_source
                ));
            } else {
                lines.push("  Custom endpoint fields hidden until Provider = custom".to_string());
            }
            lines.extend([
                String::new(),
                format!("Save tier: {tier_label}  [Tab to cycle]"),
                format!(
                    "Quick provider toggle: {}",
                    if *selected == 0 { "←/→ or Enter" } else { "select Provider field" }
                ),
                format!(
                    "Quick API toggle: {}",
                    if is_custom_provider && *selected == 4 {
                        "←/→"
                    } else {
                        "select Custom API field"
                    }
                ),
                "Controls: ↑/↓ select  Enter apply/edit  E manual edit  / filter models  X clear/reset  T test  S save  R refresh  Esc close/cancel".to_string(),
                "Paste works while editing a field.".to_string(),
                "Tip: leave a field empty to clear it.".to_string(),
                String::new(),
                format!(
                    "Provider preview: {}",
                    if provider == "custom" {
                        format!(
                            "custom ({})",
                            if custom_api_format.is_empty() {
                                "openai"
                            } else {
                                custom_api_format
                            }
                        )
                    } else if provider == "auto" {
                        "auto".to_string()
                    } else {
                        provider.clone()
                    }
                ),
                "Default launch mode: TUI".to_string(),
            ]);
            if !model_suggestions.is_empty() {
                lines.push(String::new());
                lines.push(
                    "Model suggestions (auto-loaded; ←/→ move, PgUp/PgDn scroll, Enter/1-8 apply):"
                        .to_string(),
                );
                let visible = model_suggestions
                    .iter()
                    .enumerate()
                    .skip(*suggestion_scroll)
                    .take(8);
                for (actual_idx, suggestion) in visible {
                    let marker = if actual_idx == *suggestion_index {
                        ">"
                    } else {
                        " "
                    };
                    lines.push(format!(
                        "  {marker} {}. {suggestion}",
                        actual_idx - *suggestion_scroll + 1
                    ));
                }
                let shown_end = (*suggestion_scroll + 8).min(model_suggestions.len());
                lines.push(format!(
                    "  showing {}-{} of {}",
                    *suggestion_scroll + 1,
                    shown_end,
                    model_suggestions.len()
                ));
                if shown_end < model_suggestions.len() {
                    lines.push(format!(
                        "  ... and {} more",
                        model_suggestions.len() - shown_end
                    ));
                }
            }
            if let Some(status) = status {
                lines.push(String::new());
                lines.push(format!("Status: {status}"));
            }
            let config_text = lines.join(
                "
",
            );
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
