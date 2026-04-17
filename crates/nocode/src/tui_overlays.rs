//! TUI overlay rendering — extracted from tui_app.rs.

use crate::command_registry::CommandRegistry;
use crate::tui_widgets::OverlayBlock;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

use crate::status_hud::StatusHud;
use crate::tui_app::{Overlay, preset_label};

/// Draw the active overlay on top of the main UI.
pub(crate) fn draw_overlay(
    overlay: &Overlay,
    hud: &StatusHud,
    scroll: u16,
    frame: &mut Frame,
    area: Rect,
) {
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
                 Ctrl-A/E     — jump to start/end of line\n\
                 Ctrl-W       — delete word backward\n\
                 Ctrl-K       — delete to end of line\n\
                 Ctrl-U       — clear input\n\
                 Ctrl-P/N     — input history\n\
                 Ctrl-V       — paste image from clipboard\n\
                 Ctrl-Y       — copy last response to clipboard\n\
                 Ctrl-F       — search chat (Ctrl-N/P: next/prev match)\n\
                 Ctrl-O       — toggle thinking blocks\n\
                 Ctrl-T       — toggle theme\n\
                 Ctrl-L       — clear chat\n\
                 Tab          — toggle tool output / thinking\n\
                 /            — command autocomplete\n\
                 F4           — error log\n\
                 \n",
            );
            help.push_str(&cmd_reg.help_text());
            let overlay_w = OverlayBlock::new("Help", &help).with_scroll(scroll);
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
            let overlay_w = OverlayBlock::new("Status", &status).with_scroll(scroll);
            frame.render_widget(overlay_w, area);
        }
        Overlay::Sessions => {
            use nocode_core::session::persistence::SessionPersistence;
            let cwd = std::env::current_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| ".".to_string());
            let sessions = SessionPersistence::list_sessions_with_info(&cwd);
            let body = if sessions.is_empty() {
                "No saved sessions.\n\nSessions are saved automatically during conversations.\nUse /resume <id> to restore a session.".to_string()
            } else {
                let mut lines = Vec::new();
                lines.push(format!(
                    "{} saved session{}:\n",
                    sessions.len(),
                    if sessions.len() > 1 { "s" } else { "" }
                ));
                for (i, s) in sessions.iter().take(20).enumerate() {
                    let preview = s.first_user_message.as_deref().unwrap_or("(empty)");
                    let age = s
                        .modified_at
                        .map(|t| {
                            let now = chrono::Utc::now();
                            let dur = now.signed_duration_since(t);
                            if dur.num_days() > 0 {
                                format!("{}d ago", dur.num_days())
                            } else if dur.num_hours() > 0 {
                                format!("{}h ago", dur.num_hours())
                            } else {
                                format!("{}m ago", dur.num_minutes().max(1))
                            }
                        })
                        .unwrap_or_default();
                    lines.push(format!(
                        "  {}. {} ({} msgs, {}) {}",
                        i + 1,
                        &s.id[..s.id.len().min(12)],
                        s.message_count,
                        age,
                        preview,
                    ));
                }
                if sessions.len() > 20 {
                    lines.push(format!("\n  ... and {} more", sessions.len() - 20));
                }
                lines.push(String::from("\nUse /resume <id> to restore a session."));
                lines.join("\n")
            };
            let overlay_w = OverlayBlock::new("Sessions", &body).with_scroll(scroll);
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
                    for (server, tool) in tools.iter().take(100) {
                        lines.push(format!("  {server}:{}", tool.name));
                    }
                    if tools.len() > 100 {
                        lines.push(format!("  ... and {} more", tools.len() - 100));
                    }
                }
                lines.join(
                    "
",
                )
            };
            let overlay_w = OverlayBlock::new("MCP Servers", &text).with_scroll(scroll);
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
                    let elapsed = w.started_at.map_or(String::new(), |t| {
                        let secs = t.elapsed().as_secs();
                        if secs < 60 {
                            format!(" ({secs}s)")
                        } else {
                            format!(" ({}m{}s)", secs / 60, secs % 60)
                        }
                    });
                    let detail = match w.state {
                        nocode_core::agent::worker::WorkerState::Finished => {
                            w.result.as_deref().map_or(String::new(), |r| {
                                if r.chars().count() > 80 {
                                    let preview: String = r.chars().take(77).collect();
                                    format!("\n    \u{2192} {preview}...")
                                } else {
                                    format!("\n    \u{2192} {r}")
                                }
                            })
                        }
                        nocode_core::agent::worker::WorkerState::Failed => w
                            .error
                            .as_deref()
                            .map_or(String::new(), |e| format!("\n    ✖ {e}")),
                        _ => String::new(),
                    };
                    lines.push(format!(
                        "  {state_icon} {} ({}): {:?}{elapsed}{detail}",
                        w.name, w.id, w.state
                    ));
                }
                lines.push(String::new());
                lines.push(
                    "Commands: /agent-create <name> <prompt> | /agent-stop <name>".to_string(),
                );
                lines.join("\n")
            };
            let overlay_w = OverlayBlock::new("Agents", &text).with_scroll(scroll);
            frame.render_widget(overlay_w, area);
        }
        Overlay::Config(cs) => {
            use ratatui::style::{Modifier, Style};
            use ratatui::text::{Line, Span};
            use ratatui::widgets::{Block, Borders, Clear, Paragraph};

            let theme = crate::tui_theme::default_theme();
            let crate::tui_app::ConfigState {
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
                preset_index,
            } = cs.as_ref();

            let is_custom = provider == "custom";
            let tier_label = match tier {
                0 => "user",
                1 => "project",
                _ => "local",
            };
            let dim = Style::default().fg(theme.text_dim);
            let normal = Style::default().fg(theme.text);
            let highlight = Style::default()
                .fg(theme.claude)
                .add_modifier(Modifier::BOLD);
            let section_style = Style::default().fg(theme.claude);
            let sel_marker = |idx: usize| {
                if idx == *selected {
                    Span::styled("▸ ", highlight)
                } else {
                    Span::raw("  ")
                }
            };
            let source_span = |src: &str| Span::styled(format!("  ({src})"), dim);
            let field_val = |idx: usize, val: &str| -> Span {
                if idx == *selected && *editing && !*filtering_models {
                    Span::styled(format!("{input}█"), highlight)
                } else if val.is_empty() {
                    Span::styled("(not set)", dim)
                } else {
                    Span::styled(val.to_string(), normal)
                }
            };

            // API key display
            let key_display = if *selected == 1 && *editing && !*filtering_models {
                Span::styled(format!("{input}█"), highlight)
            } else if api_key.is_empty() {
                Span::styled("(not set)", dim)
            } else {
                let tail: String = api_key
                    .chars()
                    .rev()
                    .take(4)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                Span::styled(format!("···{tail}"), normal)
            };

            let mut lines: Vec<Line> = Vec::with_capacity(32);

            // === Section: Provider + Auth ===
            lines.push(Line::from(vec![
                Span::styled("── Provider ", section_style),
                Span::styled("─".repeat(20), dim),
            ]));
            lines.push(Line::from(vec![
                sel_marker(0),
                Span::styled("Provider  ", dim),
                if 0 == *selected {
                    Span::styled(provider.as_str(), highlight)
                } else {
                    Span::styled(provider.as_str(), normal)
                },
                source_span(provider_source),
                if 0 == *selected {
                    Span::styled("  ◀ ▶", dim)
                } else {
                    Span::raw("")
                },
            ]));
            lines.push(Line::from(vec![
                sel_marker(1),
                Span::styled("API Key   ", dim),
                key_display,
                source_span(api_key_source),
            ]));
            lines.push(Line::raw(""));

            // === Section: Endpoint (custom only) ===
            if is_custom {
                let preset_name = preset_label(*preset_index);
                lines.push(Line::from(vec![
                    Span::styled("── Endpoint ", section_style),
                    Span::styled("─".repeat(10), dim),
                    Span::styled(format!(" {preset_name} "), section_style),
                    Span::styled("─".repeat(6), dim),
                ]));
                lines.push(Line::from(vec![
                    sel_marker(3),
                    Span::styled("Base URL  ", dim),
                    field_val(3, custom_base_url),
                    source_span(custom_base_url_source),
                ]));
                lines.push(Line::from(vec![
                    sel_marker(4),
                    Span::styled("Format    ", dim),
                    field_val(4, custom_api_format),
                    source_span(custom_api_format_source),
                    if 4 == *selected {
                        Span::styled("  ◀ ▶", dim)
                    } else {
                        Span::raw("")
                    },
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::styled("── Endpoint ", section_style),
                    Span::styled("─".repeat(20), dim),
                ]));
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled("Set Provider to custom to configure", dim),
                ]));
            }
            lines.push(Line::raw(""));

            // === Section: Model list ===
            let filter_info = if model_filter.is_empty() {
                String::new()
            } else {
                format!(" /{model_filter}")
            };
            let count_info = if model_suggestions.is_empty() {
                String::new()
            } else {
                format!(" {}/{}", suggestion_index + 1, model_suggestions.len())
            };
            lines.push(Line::from(vec![
                Span::styled("── Model ", section_style),
                Span::styled("─".repeat(8), dim),
                Span::styled(filter_info, section_style),
                Span::styled(" ─".to_string(), dim),
                Span::styled(count_info, dim),
                Span::styled(" ─", dim),
            ]));

            // Current model
            lines.push(Line::from(vec![
                sel_marker(2),
                Span::styled("Current   ", dim),
                if 2 == *selected && *editing && !*filtering_models {
                    Span::styled(format!("{input}█"), highlight)
                } else if model.is_empty() {
                    Span::styled("(default)", dim)
                } else {
                    Span::styled(model.as_str(), normal)
                },
                source_span(model_source),
            ]));

            // Model suggestions list
            if !model_suggestions.is_empty() {
                let max_visible = 5;
                let visible = model_suggestions
                    .iter()
                    .enumerate()
                    .skip(*suggestion_scroll)
                    .take(max_visible);
                for (idx, suggestion) in visible {
                    let is_current = suggestion == model;
                    let is_selected = idx == *suggestion_index;
                    let marker = if is_selected { "▸ " } else { "  " };
                    let style = if is_selected {
                        highlight
                    } else if is_current {
                        Style::default().fg(theme.success)
                    } else {
                        normal
                    };
                    let mut spans = vec![
                        Span::raw("  "),
                        Span::styled(marker, if is_selected { highlight } else { dim }),
                        Span::styled(suggestion.as_str(), style),
                    ];
                    if is_current {
                        spans.push(Span::styled(" ●", Style::default().fg(theme.success)));
                    }
                    lines.push(Line::from(spans));
                }
            } else if status.as_deref() == Some("Loading models...") {
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled("Loading...", dim),
                ]));
            } else if !model_filter.is_empty() {
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled("No matches", dim),
                ]));
            }
            lines.push(Line::raw(""));

            // === Footer: controls + status ===
            let mut ctrl_spans = vec![
                Span::styled(format!("Save: {tier_label}"), normal),
                Span::styled(" [Tab]  ", dim),
                Span::styled("S", normal),
                Span::styled(" save  ", dim),
                Span::styled("T", normal),
                Span::styled(" test  ", dim),
                Span::styled("R", normal),
                Span::styled(" refresh  ", dim),
            ];
            if is_custom {
                ctrl_spans.push(Span::styled("P", normal));
                ctrl_spans.push(Span::styled(" preset  ", dim));
            }
            ctrl_spans.push(Span::styled("/", normal));
            ctrl_spans.push(Span::styled(" filter  ", dim));
            ctrl_spans.push(Span::styled("Esc", normal));
            ctrl_spans.push(Span::styled(" ×", dim));
            lines.push(Line::from(ctrl_spans));

            if let Some(status_msg) = status {
                lines.push(Line::from(vec![
                    Span::styled("Status: ", dim),
                    Span::styled(status_msg.as_str(), normal),
                ]));
            }

            // === Render ===
            let w = (area.width * 4 / 5).max(20).min(area.width);
            let content_h = lines.len() as u16 + 2; // +2 for border
            let h = content_h.max(12).min(area.height * 3 / 5).min(area.height);
            let x = area.x + (area.width.saturating_sub(w)) / 2;
            let y = area.y + (area.height.saturating_sub(h)) / 2;
            let overlay_area = ratatui::layout::Rect::new(x, y, w, h);

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

            let paragraph = Paragraph::new(lines);
            paragraph.render(inner, frame.buffer_mut());
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
            let overlay_w = OverlayBlock::new("Memory", &text).with_scroll(scroll);
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
            let overlay_w = OverlayBlock::new("Cost", &text).with_scroll(scroll);
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
            let overlay_w = OverlayBlock::new("Error Log", &text).with_scroll(scroll);
            frame.render_widget(overlay_w, area);
        }
    }
}
