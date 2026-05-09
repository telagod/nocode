//! TUI overlay rendering — extracted from tui_app.rs.

use crate::command_registry::CommandRegistry;
use crate::tui_widgets::OverlayBlock;
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::status_hud::StatusHud;
use crate::tui_app::Overlay;

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

Configure in .nocode/config.toml under "mcp_servers".

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
